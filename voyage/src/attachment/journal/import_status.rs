use super::*;
impl Journal {
    pub(crate) fn json_import_finalized(&self, session: Uuid, transfer: Uuid) -> Result<bool> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_import_finalized')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(false);
        }
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM process_import_finalized WHERE session_id=?1 AND transfer_id=?2)",params![session.to_string(),transfer.to_string()],|r|r.get(0))?)
    }
    pub(crate) fn finalize_json_import(&self, session: Uuid, transfer: Uuid) -> Result<()> {
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_import_finalized(session_id TEXT PRIMARY KEY,transfer_id TEXT NOT NULL)")?;
        self.connection.execute(
            "INSERT OR IGNORE INTO process_import_finalized VALUES(?1,?2)",
            params![session.to_string(), transfer.to_string()],
        )?;
        Ok(())
    }
    pub(crate) fn json_import_provenance(
        &self,
        session: Uuid,
        transfer: Uuid,
    ) -> Result<Option<serde_json::Value>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT provenance FROM imports WHERE session_id=?1 AND transfer_id=?2",
                params![session.to_string(), transfer.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }
}
