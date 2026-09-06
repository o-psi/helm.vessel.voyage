//! Admit operator effects before dispatch; uncertain accepted work is never replayed.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
impl Journal {
    pub(crate) fn record_workflow(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        invocation: crate::workflow::Invocation,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id && run.state == RunState::Accepted,
            "workflow needs accepted run"
        );
        let mut saved = read_session(&tx, guard.session_id)?;
        ensure!(
            saved.session.workflow_runs.len() < 10000,
            "workflow history capacity reached"
        );
        saved.session.workflow_runs.push(invocation);
        saved.revision = saved.revision.checked_add(1).context("revision overflow")?;
        saved.session.revision = saved.revision;
        tx.execute(
            "UPDATE sessions SET revision=?1,state=?2 WHERE id=?3",
            params![
                i64::try_from(saved.revision)?,
                snapshot(&saved.session)?,
                guard.session_id.to_string()
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub(crate) fn control_receipt(&self, command: &RuntimeCommand) -> Result<Option<Value>> {
        let RuntimeCommand::ExecuteTool { command_id, .. } = command else {
            anyhow::bail!("not a control command")
        };
        let existing: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [command_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        existing
            .map(|(request, receipt)| {
                ensure!(
                    request == serde_json::to_string(command)?,
                    "command ID payload conflict"
                );
                Ok(serde_json::from_str(&receipt)?)
            })
            .transpose()
    }
    pub(crate) fn admit_control(
        &mut self,
        guard: &ExecutionGuard,
        command: RuntimeCommand,
        now: i64,
    ) -> Result<(Value, bool)> {
        self.check_guard(guard, guard.session_id)?;
        let RuntimeCommand::ExecuteTool {
            command_id,
            expected_revision,
            expires_at_ms,
            run_id,
            ..
        } = command
        else {
            anyhow::bail!("not a control command")
        };
        ensure!(!command_id.is_nil(), "nil command ID");
        let request = serde_json::to_string(&command)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [command_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((original, receipt)) = existing {
            ensure!(request == original, "command ID payload conflict");
            return Ok((serde_json::from_str(&receipt)?, false));
        }
        let collisions:i64=tx.query_row("SELECT (SELECT count(*) FROM commands WHERE id=?1)+(SELECT count(*) FROM steering WHERE id=?1)",[command_id.to_string()],|row|row.get(0))?;
        ensure!(collisions == 0, "command ID already used");
        let expiry = i64::try_from(expires_at_ms)?;
        ensure!(
            expiry > now && expiry - now <= 300000,
            "invalid command deadline"
        );
        let saved = read_session(&tx, guard.session_id)?;
        ensure!(
            saved.revision == expected_revision,
            "session revision conflict"
        );
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == guard.session_id
                && matches!(run.state, RunState::Accepted | RunState::Running),
            "control needs the exact active run"
        );
        let count: i64 = tx.query_row("SELECT count(*) FROM process_commands", [], |row| {
            row.get(0)
        })?;
        ensure!(count < 100000, "control receipt capacity reached");
        let receipt = json!({"command_id":command_id,"run_id":run_id,"status":"accepted","outcome":"pending_or_unknown"});
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                request,
                serde_json::to_string(&receipt)?
            ],
        )?;
        tx.commit()?;
        Ok((receipt, true))
    }
    pub(crate) fn complete_control(
        &mut self,
        guard: &ExecutionGuard,
        id: Uuid,
        outcome: Value,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (request, receipt): (String, String) = tx.query_row(
            "SELECT request,receipt FROM process_commands WHERE id=?1",
            [id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            matches!(
                serde_json::from_str::<RuntimeCommand>(&request)?,
                RuntimeCommand::ExecuteTool { .. }
            ),
            "not a tool receipt"
        );
        let mut receipt: Value = serde_json::from_str(&receipt)?;
        ensure!(
            receipt["outcome"] == "pending_or_unknown",
            "control already finalized"
        );
        receipt["outcome"] = outcome;
        ensure!(
            serde_json::to_vec(&receipt)?.len() <= 2 * 1024 * 1024,
            "control result exceeds durable bound"
        );
        tx.execute(
            "UPDATE process_commands SET receipt=?1 WHERE id=?2",
            params![serde_json::to_string(&receipt)?, id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }
}
