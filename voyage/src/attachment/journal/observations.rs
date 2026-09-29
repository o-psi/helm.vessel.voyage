//! Bounded durable public observations. Content events are committed with canonical state.
use super::*;
use serde_json::{Value, json};

const TEXT_CHUNK_BYTES: usize = 4096;
impl Journal {
    pub(crate) fn initialize_observations(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_observations(cursor INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,kind TEXT NOT NULL,revision INTEGER NOT NULL,run_id TEXT,entity_id TEXT,payload TEXT)")?;
        let has_payload: bool = self
            .connection
            .prepare("PRAGMA table_info(process_observations)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|name| name == "payload");
        if !has_payload {
            self.connection
                .execute_batch("ALTER TABLE process_observations ADD COLUMN payload TEXT")?;
        }
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_observation_pruned(session_id TEXT PRIMARY KEY, cursor INTEGER NOT NULL);
        CREATE TRIGGER IF NOT EXISTS process_observation_prune_marker BEFORE DELETE ON process_observations BEGIN INSERT INTO process_observation_pruned(session_id,cursor) VALUES(OLD.session_id,OLD.cursor) ON CONFLICT(session_id) DO UPDATE SET cursor=max(cursor,OLD.cursor); END;
        DROP TRIGGER IF EXISTS process_observation_retention;
        CREATE TRIGGER process_observation_retention AFTER INSERT ON process_observations BEGIN DELETE FROM process_observations WHERE cursor<=NEW.cursor-2048; END;
        DROP TRIGGER IF EXISTS process_observe_session;
        CREATE TRIGGER process_observe_session AFTER UPDATE ON sessions BEGIN INSERT INTO process_observations(session_id,kind,revision,payload) VALUES(NEW.id,'session',NEW.revision,json_object('name',json_extract(NEW.state,'$.name'),'revision',NEW.revision,'total_messages',json_array_length(json_extract(NEW.state,'$.messages')))); END;
        DROP TRIGGER IF EXISTS process_observe_run_create;
        CREATE TRIGGER process_observe_run_create AFTER INSERT ON runs BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,payload) VALUES(NEW.session_id,'run',(SELECT revision FROM sessions WHERE id=NEW.session_id),NEW.id,json_object('state',json_extract(NEW.record,'$.state'),'run_id',NEW.id,'partial_text_bytes',length(CAST(json_extract(NEW.record,'$.partial_text') AS BLOB)))); END;
        DROP TRIGGER IF EXISTS process_observe_run_update;
        CREATE TRIGGER process_observe_run_update AFTER UPDATE ON runs BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,payload) VALUES(NEW.session_id,'run',(SELECT revision FROM sessions WHERE id=NEW.session_id),NEW.id,json_object('state',json_extract(NEW.record,'$.state'),'run_id',NEW.id,'partial_text_bytes',length(CAST(json_extract(NEW.record,'$.partial_text') AS BLOB)))); END;
        DROP TRIGGER IF EXISTS process_observe_decision_create;
        CREATE TRIGGER process_observe_decision_create AFTER INSERT ON process_decisions BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id,payload) SELECT r.session_id,'decision',s.revision,r.id,NEW.id,json_object('decision_id',NEW.id,'status','pending') FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        DROP TRIGGER IF EXISTS process_observe_decision_update;
        CREATE TRIGGER process_observe_decision_update AFTER UPDATE ON process_decisions BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id,payload) SELECT r.session_id,'decision',s.revision,r.id,NEW.id,json_object('decision_id',NEW.id,'status',CASE WHEN NEW.response IS NULL THEN 'pending' ELSE 'resolved' END) FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        -- process_commands has no session key. A command may never fan out to other voyages.
        DROP TRIGGER IF EXISTS process_observe_command;
        DROP TRIGGER IF EXISTS process_observe_lifecycle;
        CREATE TRIGGER process_observe_lifecycle AFTER UPDATE ON process_lifecycle BEGIN INSERT INTO process_observations(session_id,kind,revision,payload) SELECT id,'lifecycle',revision,json_object('archived',NEW.archived,'deleted',NEW.deleted,'generation',NEW.generation,'transfer_id',NEW.transfer_id) FROM sessions WHERE id=NEW.session_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_assignment_create AFTER INSERT ON process_assignments BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT r.session_id,'assignment',s.revision,r.id,NEW.id FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_assignment_update AFTER UPDATE ON process_assignments BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT r.session_id,'assignment',s.revision,r.id,NEW.id FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_cleanup AFTER UPDATE ON local_cleanup_obligations BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id) SELECT id,'cleanup',revision,NEW.run_id FROM sessions WHERE id=NEW.session_id; END;")?;
        Ok(())
    }

    fn observations_initialized(tx: &Transaction<'_>) -> Result<bool> {
        Ok(tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='process_observations')",[],|r|r.get(0))?)
    }

    pub(super) fn append_public_message_created(
        tx: &Transaction<'_>,
        run: &RunRecord,
        index: usize,
        message: &crate::model::Message,
    ) -> Result<()> {
        if !Self::observations_initialized(tx)? {
            return Ok(());
        }
        let (content, truncated) = super::text_prefix(&message.content, 4096);
        let projection = json!({"role":message.role,"content":content,"content_truncated":truncated,"content_bytes":message.content.len(),"message_index":index,"projection_truncated":truncated || !message.parts.is_empty(),"complete_message":"message_chunk"});
        tx.execute("INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id,payload) VALUES(?1,'message_created',(SELECT revision FROM sessions WHERE id=?1),?2,?3,?4)",
            params![run.session_id.to_string(),run.id.to_string(),index.to_string(),serde_json::to_string(&json!({"message_index":index,"message":projection}))?])?;
        Ok(())
    }

    /// Append a sanitized, immutable public projection inside the canonical transaction.
    pub(super) fn append_public_message(
        tx: &Transaction<'_>,
        run: &RunRecord,
        index: usize,
        message: &crate::model::Message,
    ) -> Result<()> {
        // The public history projection can disclose tool arguments and tool
        // output to authorized history readers. Live events are less specific:
        // expose only bounded text and identifiers, never raw tool contents.
        let (content, truncated) = super::text_prefix(&message.content, 4096);
        let message = json!({"role":message.role,"content":content,"content_truncated":truncated,
            "content_bytes":message.content.len(),"tool_call_id":message.tool_call_id,
            "tool_calls_omitted":!message.tool_calls.is_empty(),"message_index":index,
            "projection_truncated":truncated || !message.tool_calls.is_empty() || message.tool_output.is_some(),
            "complete_message":"message_chunk"});
        if !Self::observations_initialized(tx)? {
            return Ok(());
        }
        tx.execute("INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id,payload) VALUES(?1,'message_finalized',(SELECT revision FROM sessions WHERE id=?1),?2,?3,?4)",
            params![run.session_id.to_string(),run.id.to_string(),index.to_string(),serde_json::to_string(&json!({"message_index":index,"message":message}))?])?;
        Ok(())
    }

    /// A preview may include provider-authored arguments; expose only operation
    /// identity and name. Canonical tool-result messages are emitted separately.
    pub(super) fn append_public_tool_activity(tx: &Transaction<'_>, run: &RunRecord) -> Result<()> {
        if !Self::observations_initialized(tx)? {
            return Ok(());
        }
        let calls: Vec<Value> = run
            .tool_previews
            .iter()
            .take(32)
            .map(|p| {
                json!({
                    "call_id":p.call_id,"name":p.name
                })
            })
            .collect();
        tx.execute("INSERT INTO process_observations(session_id,kind,revision,run_id,payload) VALUES(?1,'tool_activity',(SELECT revision FROM sessions WHERE id=?1),?2,?3)",
            params![run.session_id.to_string(),run.id.to_string(),serde_json::to_string(&json!({"calls":calls,"count":run.tool_previews.len()}))?])?;
        Ok(())
    }

    /// Command requests and receipts can contain private input. Only emit the
    /// scoped identity and a small, fixed-vocabulary status; never copy either JSON.
    pub(super) fn append_public_command(
        tx: &Transaction<'_>,
        session: Uuid,
        id: Uuid,
    ) -> Result<()> {
        if !Self::observations_initialized(tx)? {
            return Ok(());
        }
        let status: String = tx.query_row("SELECT coalesce(json_extract(receipt,'$.status'),'unknown') FROM process_commands WHERE id=?1",[id.to_string()],|row|row.get(0))?;
        let status = match status.as_str() {
            "applied" | "accepted" | "rejected" | "not_admitted" | "relinquished"
            | "snapshot_committed" => status.as_str(),
            _ => "unknown",
        };
        tx.execute("INSERT INTO process_observations(session_id,kind,revision,entity_id,payload) VALUES(?1,'command_receipt',(SELECT revision FROM sessions WHERE id=?1),?2,?3)",params![session.to_string(),id.to_string(),serde_json::to_string(&json!({"command_id":id,"status":status}))?])?;
        Ok(())
    }

    /// UTF-8 byte offsets refer to the run's provisional text, not session history.
    pub(super) fn append_public_text(
        tx: &Transaction<'_>,
        run: &RunRecord,
        offset: usize,
        delta: &str,
    ) -> Result<()> {
        if !Self::observations_initialized(tx)? {
            return Ok(());
        }
        let mut start = 0;
        while start < delta.len() {
            let mut end = (start + TEXT_CHUNK_BYTES).min(delta.len());
            while !delta.is_char_boundary(end) {
                end -= 1;
            }
            ensure!(end > start, "invalid UTF-8 chunk boundary");
            tx.execute("INSERT INTO process_observations(session_id,kind,revision,run_id,payload) VALUES(?1,'text_delta',(SELECT revision FROM sessions WHERE id=?1),?2,?3)",
                params![run.session_id.to_string(),run.id.to_string(),serde_json::to_string(&json!({"offset":offset + start,"text":&delta[start..end]}))?])?;
            start = end;
        }
        Ok(())
    }

    pub(crate) fn observation_cursor(&self, session: Uuid) -> Result<u64> {
        Ok(self.connection.query_row(
            "SELECT coalesce(max(cursor),0) FROM process_observations WHERE session_id=?1",
            [session.to_string()],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }
    pub(crate) fn observations(&self, session: Uuid, after: u64, limit: u32) -> Result<Value> {
        ensure!(
            (1..=128).contains(&limit),
            "event page limit must be 1..128"
        );
        let after = i64::try_from(after)?;
        let latest: i64 = self.connection.query_row(
            "SELECT coalesce(max(cursor),0) FROM process_observations WHERE session_id=?1",
            [session.to_string()],
            |r| r.get(0),
        )?;
        // Retention is global, while session cursors are sparse. A session's first
        // cursor may be 50 simply because another voyage emitted 49 events.
        let global_latest: i64 = self
            .connection
            .query_row(
                "SELECT coalesce(seq,0) FROM sqlite_sequence WHERE name='process_observations'",
                [],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        let pruned_for_session: i64 = self
            .connection
            .query_row(
                "SELECT cursor FROM process_observation_pruned WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0);
        // For legacy journals upgraded after retention already occurred, there
        // is no prior prune marker. Refuse an ambiguous replay until snapshot.
        let legacy_floor = if pruned_for_session == 0 && global_latest > 2048 {
            global_latest - 2048
        } else {
            0
        };
        let pruned_through = pruned_for_session.max(legacy_floor);
        ensure!(after <= global_latest, "event cursor is ahead of owner");
        if after < pruned_through {
            return Ok(
                json!({"projection":"public-v1","replay_gap":true,"cursor":latest,"latest_cursor":latest,"has_more":false,"events":[],"recovery":"snapshot"}),
            );
        }
        let mut query=self.connection.prepare("SELECT cursor,kind,revision,run_id,entity_id,payload FROM process_observations WHERE session_id=?1 AND cursor>?2 ORDER BY cursor LIMIT ?3")?;
        let rows = query.query_map(params![session.to_string(), after, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut events = Vec::new();
        let mut cursor = after;
        for row in rows {
            let (seq, kind, revision, run, entity, _payload) = row?;
            cursor = seq;
            events.push(json!({"cursor":seq,"session_id":session,"kind":kind,"revision":revision,"run_id":run,"entity_id":entity}));
        }
        Ok(
            json!({"projection":"public-v1","replay_gap":false,"cursor":cursor,"latest_cursor":latest,"has_more":cursor<latest,"events":events}),
        )
    }

    pub(crate) fn live_observations(&self, session: Uuid, after: u64, limit: u32) -> Result<Value> {
        // Share v1's bounded paging and conservative replay-gap calculation, but
        // never project raw run or session records from a later database state.
        let page = self.observations(session, after, limit)?;
        if page["replay_gap"] == true {
            return Ok(
                json!({"projection":"public-v2","replay_gap":true,"cursor":page["cursor"],"latest_cursor":page["latest_cursor"],"has_more":false,"events":[],"recovery":"snapshot"}),
            );
        }
        let mut query = self.connection.prepare("SELECT cursor,kind,revision,run_id,entity_id,payload FROM process_observations WHERE session_id=?1 AND cursor>?2 ORDER BY cursor LIMIT ?3")?;
        let rows = query.query_map(
            params![session.to_string(), i64::try_from(after)?, limit],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (cursor, kind, revision, run_id, entity_id, payload) = row?;
            let kind = match kind.as_str() {
                "message_finalized" => "message_finalized",
                "message_created" => "message_created",
                "text_delta" => "text_delta",
                "tool_activity" => "tool_activity",
                "run" => "run_state",
                "decision" => "decision",
                "command_receipt" => "command_outcome",
                "lifecycle" => "lifecycle",
                "session" => "session",
                "assignment" | "cleanup" => "run_state",
                _ => "catalogue",
            };
            let payload: Value = payload
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_else(|| json!({}));
            events.push(json!({"cursor":cursor,"session_id":session,"kind":kind,"revision":revision,"run_id":run_id,"entity_id":entity_id,"payload":payload}));
        }
        Ok(
            json!({"projection":"public-v2","replay_gap":false,"cursor":page["cursor"],"latest_cursor":page["latest_cursor"],"has_more":page["has_more"],"events":events}),
        )
    }
}
