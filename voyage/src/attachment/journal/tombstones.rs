//! Historical command identity survives a positive owner move without replay.
use super::*;
use voyage_protocol::process::CommandTombstone;
pub(super) fn collect(db: &Connection) -> Result<Vec<CommandTombstone>> {
    let mut result = std::collections::BTreeMap::new();
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_command_tombstones')",
        [],
        |r| r.get(0),
    )?;
    if exists {
        let mut query = db.prepare("SELECT record FROM process_command_tombstones")?;
        for row in query.query_map([], |r| r.get::<_, String>(0))? {
            let record: CommandTombstone = serde_json::from_str(&row?)?;
            result.insert(record.command_id, record);
        }
    }
    let mut query=db.prepare("SELECT b.id,b.request,c.run_id,coalesce(json_extract(p.receipt,'$.status'),CASE WHEN c.id IS NOT NULL THEN 'accepted' WHEN st.id IS NOT NULL THEN 'steering' ELSE 'not_admitted' END),json_extract(r.record,'$.state'),b.principal FROM process_command_bindings b LEFT JOIN commands c ON c.id=b.id LEFT JOIN process_commands p ON p.id=b.id LEFT JOIN runs r ON r.id=c.run_id LEFT JOIN steering st ON st.id=b.id")?;
    for row in query.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
        ))
    })? {
        let (id, request, run, status, run_state, principal) = row?;
        let id = Uuid::parse_str(&id)?;
        let record = CommandTombstone {
            command_id: id,
            principal_id: Uuid::parse_str(&principal)?,
            payload_sha256: request.map(|request| {
                request
                    .strip_prefix("sha256:")
                    .map(str::to_owned)
                    .unwrap_or_else(|| hex::encode(Sha256::digest(request.as_bytes())))
            }),
            run_id: run.map(|run| Uuid::parse_str(&run)).transpose()?,
            original_status: status,
            run_state,
        };
        result.entry(id).or_insert(record);
    }
    ensure!(
        result.len() <= MAX_COMMANDS as usize,
        "transferred command ledger exceeds capacity"
    );
    Ok(result.into_values().collect())
}
pub(super) fn import(tx: &Transaction<'_>, records: &[CommandTombstone]) -> Result<()> {
    ensure!(
        records.len() <= MAX_COMMANDS as usize,
        "transferred command ledger exceeds capacity"
    );
    tx.execute_batch("CREATE TABLE IF NOT EXISTS process_command_tombstones(id TEXT PRIMARY KEY,record TEXT NOT NULL)")?;
    for record in records {
        ensure!(
            !record.command_id.is_nil()
                && !record.principal_id.is_nil()
                && record
                    .payload_sha256
                    .as_ref()
                    .is_none_or(|hash| hash.len() == 64 && hex::decode(hash).is_ok())
                && record.run_state.as_ref().is_none_or(|state| matches!(
                    state.as_str(),
                    "accepted"
                        | "running"
                        | "completed"
                        | "incomplete"
                        | "cancelled"
                        | "failed"
                        | "interrupted"
                ))
                && record.original_status.len() <= 64
                && !record.original_status.chars().any(char::is_control),
            "invalid historical command evidence"
        );
        tx.execute(
            "INSERT INTO process_command_tombstones VALUES(?1,?2)",
            params![
                record.command_id.to_string(),
                serde_json::to_string(record)?
            ],
        )?;
    }
    Ok(())
}
impl Journal {
    pub(crate) fn transferred_command(&self, id: Uuid) -> Result<Option<CommandTombstone>> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_command_tombstones')",
            [],
            |r| r.get(0),
        )?;
        if !exists {
            return Ok(None);
        }
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT record FROM process_command_tombstones WHERE id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Into::into))
            .transpose()
    }
}
