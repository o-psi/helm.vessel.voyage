//! Browser intent and reply checkpoints share the canonical private remote store.
//! Dispatched effects also use existing session-resource obligations, so older readers
//! cannot silently treat an interrupted browser executor as observed cleanup.
use super::*;
impl Journal {
    pub(crate) fn browser_load(&mut self, session: Uuid) -> Result<Option<String>> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS browser_state(session_id TEXT PRIMARY KEY, state TEXT NOT NULL)")?;
        Ok(self
            .connection
            .query_row(
                "SELECT state FROM browser_state WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub(crate) fn browser_save(&mut self, session: Uuid, state: &str) -> Result<()> {
        self.check_schema()?;
        ensure!(
            state.len() <= 16 * 1024 * 1024,
            "browser journal bound exceeded"
        );
        let value: serde_json::Value = serde_json::from_str(state)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS browser_state(session_id TEXT PRIMARY KEY, state TEXT NOT NULL); CREATE TABLE IF NOT EXISTS process_session_resources(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,run_id TEXT NOT NULL,kind TEXT NOT NULL,state TEXT NOT NULL)")?;
        let has_observations:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='process_observations')",[],|r|r.get(0))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (id, entry) in value["entries"]
            .as_object()
            .context("browser entries missing")?
        {
            // Durable metadata-only reverse notification. Public observers see no action,
            // URL, DOM, image or file bytes; only the bound executor can fetch pending work.
            if has_observations && entry["receipt"]["state"] == "pending" {
                let json_path = format!("$.entries.\"{id}\".receipt.state");
                let prior: Option<Option<String>> = tx
                    .query_row(
                        "SELECT json_extract(state,?2) FROM browser_state WHERE session_id=?1",
                        params![session.to_string(), json_path],
                        |r| r.get(0),
                    )
                    .optional()?;
                if prior.flatten().as_deref() != Some("pending") {
                    tx.execute("INSERT INTO process_observations(session_id,kind,revision,run_id,entity_id) SELECT id,'browser',revision,?2,?3 FROM sessions WHERE id=?1",params![session.to_string(),entry["request"]["binding"]["run_id"].as_str(),id])?;
                }
            }
            let pending = entry["receipt"]["cleanup_pending"] == true;
            let run = entry["request"]["binding"]["run_id"]
                .as_str()
                .context("browser request run missing")?;
            if pending {
                let owned = read_run(&tx, Uuid::parse_str(run)?)?;
                ensure!(
                    owned.session_id == session,
                    "browser request run/session mismatch"
                );
                let status = if entry["receipt"]["state"] == "dispatched" {
                    "owned"
                } else {
                    "cleanup_unknown"
                };
                let prior: Option<(String, String, String)> = tx
                    .query_row(
                        "SELECT session_id,run_id,kind FROM process_session_resources WHERE id=?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )
                    .optional()?;
                if let Some((s, r, k)) = prior {
                    ensure!(
                        s == session.to_string() && r == run && k == "browser_effect",
                        "browser resource identity conflict"
                    );
                }
                tx.execute("INSERT INTO process_session_resources(id,session_id,run_id,kind,state) VALUES(?1,?2,?3,'browser_effect',?4) ON CONFLICT(id) DO UPDATE SET state=excluded.state WHERE state!=excluded.state", params![id,session.to_string(),run,status])?;
            } else {
                tx.execute("UPDATE process_session_resources SET state='observed' WHERE id=?1 AND session_id=?2 AND kind='browser_effect' AND state!='observed'",params![id,session.to_string()])?;
            }
        }
        // Compaction may retire an entry in the same checkpoint that observes
        // cleanup. Update its canonical resource obligation atomically as well.
        if let Some(retired) = value["retired_entries"].as_object() {
            for id in retired.keys() {
                tx.execute("UPDATE process_session_resources SET state='observed' WHERE id=?1 AND session_id=?2 AND kind='browser_effect' AND state!='observed'", params![id, session.to_string()])?;
            }
        }
        tx.execute("INSERT INTO browser_state(session_id,state) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET state=excluded.state", params![session.to_string(),state])?;
        commit(tx, &self.commit_fence)
    }
}
