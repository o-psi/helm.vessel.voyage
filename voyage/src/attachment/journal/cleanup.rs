//! Bounded, authored cleanup progress. It is diagnostic evidence, never cleanup authority.
use super::*;
use serde_json::Value;

impl Journal {
    pub(crate) fn initialize_cleanup_progress(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_cleanup_progress(session_id TEXT PRIMARY KEY,run_id TEXT NOT NULL,record TEXT NOT NULL);
            CREATE TRIGGER IF NOT EXISTS process_observe_cleanup_insert AFTER INSERT ON process_cleanup_progress BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id) SELECT id,'cleanup',revision,NEW.run_id FROM sessions WHERE id=NEW.session_id; END;
            CREATE TRIGGER IF NOT EXISTS process_observe_cleanup_update AFTER UPDATE ON process_cleanup_progress BEGIN INSERT INTO process_observations(session_id,kind,revision,run_id) SELECT id,'cleanup',revision,NEW.run_id FROM sessions WHERE id=NEW.session_id; END;")?;
        self.connection.execute("UPDATE process_cleanup_progress SET record=json_set(record,'$.phase','blocked','$.retryable',json('false'),'$.pending',json('[\"Previous runtime cleanup evidence\"]')) WHERE session_id=?1 AND (json_extract(record,'$.phase')='running' OR json_extract(record,'$.retryable')=1)", [guard.session_id.to_string()])?;
        Ok(())
    }
    pub(crate) fn record_cleanup_progress(
        &mut self,
        guard: &ExecutionGuard,
        run: Uuid,
        mut value: Value,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            self.run(run)?.session_id == guard.session_id,
            "cleanup run mismatch"
        );
        value["run_id"] = serde_json::json!(run);
        let encoded = serde_json::to_string(&value)?;
        ensure!(encoded.len() <= 4096, "cleanup progress exceeds bound");
        self.connection.execute("INSERT INTO process_cleanup_progress VALUES(?1,?2,?3) ON CONFLICT(session_id) DO UPDATE SET run_id=excluded.run_id,record=excluded.record", params![guard.session_id.to_string(),run.to_string(),encoded])?;
        Ok(())
    }
    pub(crate) fn cleanup_progress(&self, session: Uuid) -> Result<Value> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_cleanup_progress')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(Value::Null);
        }
        let value: Option<String> = self.connection.query_row("SELECT record FROM process_cleanup_progress WHERE session_id=?1 AND run_id=(SELECT id FROM runs WHERE session_id=?1 ORDER BY rowid DESC LIMIT 1) AND length(CAST(record AS BLOB))<=4096", [session.to_string()], |r| r.get(0)).optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .unwrap_or(Ok(Value::Null))
    }
}
