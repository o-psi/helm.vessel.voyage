//! Bounded durable public invalidations. Text is recovered through canonical projections.
use super::*;
use serde_json::{Value, json};
impl Journal {
    pub(crate) fn initialize_observations(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_observations(cursor INTEGER PRIMARY KEY AUTOINCREMENT,session_id TEXT NOT NULL,kind TEXT NOT NULL,revision INTEGER NOT NULL,run_id TEXT,entity_id TEXT);
        CREATE TRIGGER IF NOT EXISTS process_observation_retention AFTER INSERT ON process_observations BEGIN DELETE FROM process_observations WHERE cursor<=NEW.cursor-2048; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_session AFTER UPDATE ON sessions BEGIN INSERT INTO process_observations(session_id,kind,revision) VALUES(NEW.id,'session',NEW.revision); END;
        CREATE TRIGGER IF NOT EXISTS process_observe_run_create AFTER INSERT ON runs BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id) VALUES(NEW.session_id,'run',(SELECT revision FROM sessions WHERE id=NEW.session_id),NEW.id); END;
        CREATE TRIGGER IF NOT EXISTS process_observe_run_update AFTER UPDATE ON runs BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id) VALUES(NEW.session_id,'run',(SELECT revision FROM sessions WHERE id=NEW.session_id),NEW.id); END;
        CREATE TRIGGER IF NOT EXISTS process_observe_decision_create AFTER INSERT ON process_decisions BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT r.session_id,'decision',s.revision,r.id,NEW.id FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_decision_update AFTER UPDATE ON process_decisions BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT r.session_id,'decision',s.revision,r.id,NEW.id FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_command AFTER INSERT ON process_commands BEGIN INSERT INTO process_observations(session_id,kind,revision,entity_id) SELECT id,'command_receipt',revision,NEW.id FROM sessions; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_lifecycle AFTER UPDATE ON process_lifecycle BEGIN INSERT INTO process_observations(session_id,kind,revision) SELECT id,'lifecycle',revision FROM sessions WHERE id=NEW.session_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_assignment_create AFTER INSERT ON process_assignments BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT r.session_id,'assignment',s.revision,r.id,NEW.id FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_assignment_update AFTER UPDATE ON process_assignments BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT r.session_id,'assignment',s.revision,r.id,NEW.id FROM runs r JOIN sessions s ON s.id=r.session_id WHERE r.id=NEW.run_id; END;
        CREATE TRIGGER IF NOT EXISTS process_observe_cleanup AFTER UPDATE ON local_cleanup_obligations BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id) SELECT id,'cleanup',revision,NEW.run_id FROM sessions WHERE id=NEW.session_id; END;")?;
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
        let (first,latest):(Option<i64>,i64)=self.connection.query_row("SELECT min(cursor),coalesce(max(cursor),0) FROM process_observations WHERE session_id=?1",[session.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
        ensure!(after <= latest, "event cursor is ahead of owner");
        if first.is_some_and(|first| after < first - 1) {
            return Ok(
                json!({"projection":"public-v1","replay_gap":true,"cursor":latest,"latest_cursor":latest,"has_more":false,"events":[],"recovery":"snapshot"}),
            );
        }
        let mut query=self.connection.prepare("SELECT cursor,kind,revision,run_id,entity_id FROM process_observations WHERE session_id=?1 AND cursor>?2 ORDER BY cursor LIMIT ?3")?;
        let rows = query.query_map(params![session.to_string(), after, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        let mut cursor = after;
        for row in rows {
            let (seq, kind, revision, run, entity) = row?;
            cursor = seq;
            events.push(json!({"cursor":seq,"session_id":session,"kind":kind,"revision":revision,"run_id":run,"entity_id":entity}));
        }
        Ok(
            json!({"projection":"public-v1","replay_gap":false,"cursor":cursor,"latest_cursor":latest,"has_more":cursor<latest,"events":events}),
        )
    }
}
