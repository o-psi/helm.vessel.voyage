//! Immutable caller/payload reservation precedes every process mutation.
use super::*;
use voyage_protocol::process::RuntimeCommand;
impl Journal {
    pub(crate) fn initialize_command_bindings(
        &mut self,
        guard: &ExecutionGuard,
        principal: Uuid,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS process_command_bindings(id TEXT PRIMARY KEY,principal TEXT NOT NULL,request TEXT)")?;
        tx.execute("INSERT OR IGNORE INTO process_command_bindings SELECT c.id,json_extract(r.record,'$.principal_id'),NULL FROM commands c JOIN runs r ON r.id=c.run_id",[])?;
        tx.execute("INSERT OR IGNORE INTO process_command_bindings SELECT id,?1,request FROM process_commands",[principal.to_string()])?;
        tx.execute("INSERT OR IGNORE INTO process_command_bindings SELECT id,json_extract(record,'$.request.actor.principal_id'),NULL FROM steering",[])?;
        commit(tx, &self.commit_fence)
    }
    pub(crate) fn bind_process_command(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
        principal: Uuid,
        command: &RuntimeCommand,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(!id.is_nil() && !principal.is_nil(), "nil command actor");
        let request = serde_json::to_string(command)?;
        if let Some(record) = self.transferred_command(id)? {
            ensure!(
                record.principal_id == principal,
                "historical command ID principal conflict"
            );
            let hash = record
                .payload_sha256
                .context("historical command payload unavailable; inspect Receipt")?;
            ensure!(
                hash == hex::encode(Sha256::digest(request.as_bytes())),
                "historical command ID payload conflict"
            );
            return Ok(());
        }
        ensure!(
            request.len() <= 128 * 1024,
            "command payload exceeds reservation limit"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT principal,request FROM process_command_bindings WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((actor, payload)) = old {
            ensure!(
                actor == principal.to_string()
                    && payload
                        .as_ref()
                        .is_none_or(|payload| super::deletion::request_matches(payload, &request)),
                "command ID payload or principal conflict"
            );
            return Ok(());
        }
        let count: i64 =
            tx.query_row("SELECT count(*) FROM process_command_bindings", [], |r| {
                r.get(0)
            })?;
        ensure!(count < MAX_COMMANDS, "command reservation capacity reached");
        tx.execute(
            "INSERT INTO process_command_bindings VALUES(?1,?2,?3)",
            params![id.to_string(), principal.to_string(), request],
        )?;
        commit(tx, &self.commit_fence)
    }
}
