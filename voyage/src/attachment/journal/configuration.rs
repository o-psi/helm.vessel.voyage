//! Executing-host settings are private, fenced, and excluded from conversation export.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
impl Journal {
    /// Startup caller holds the process startup lock; reading settings has no effects.
    pub(crate) fn initial_configuration(&self, session: Uuid) -> Result<Option<String>> {
        let exists:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='process_configuration')",[],|row|row.get(0))?;
        if !exists {
            return Ok(None);
        }
        Ok(self
            .connection
            .query_row(
                "SELECT settings FROM process_configuration WHERE session_id=?1",
                [session.to_string()],
                |row| row.get(0),
            )
            .optional()?)
    }
    pub(crate) fn saved_configuration(&mut self, guard: &ExecutionGuard) -> Result<Option<String>> {
        self.check_guard(guard, guard.session_id)?;
        self.initial_configuration(guard.session_id)
    }
    /// Freeze the original executing-host configuration without changing history.
    pub(crate) fn retain_initial_configuration(
        &mut self,
        guard: &ExecutionGuard,
        settings: String,
    ) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        ensure!(
            settings.len() <= 1024 * 1024,
            "initial configuration exceeds limit"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS process_configuration(session_id TEXT PRIMARY KEY, settings TEXT NOT NULL)")?;
        tx.execute(
            "INSERT INTO process_configuration VALUES(?1,?2) ON CONFLICT(session_id) DO NOTHING",
            params![guard.session_id.to_string(), settings],
        )?;
        commit(tx, &self.commit_fence)
    }

    pub(crate) fn configure(
        &mut self,
        guard: &ExecutionGuard,
        command: RuntimeCommand,
        settings: String,
        model: String,
        now: i64,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_configuration(session_id TEXT PRIMARY KEY, settings TEXT NOT NULL)")?;
        let (RuntimeCommand::Configure {
            command_id,
            expected_revision,
            expires_at_ms,
            ..
        }
        | RuntimeCommand::SetAccess {
            command_id,
            expected_revision,
            expires_at_ms,
            ..
        }) = &command
        else {
            anyhow::bail!("not configure")
        };
        ensure!(
            !command_id.is_nil() && settings.len() <= 1024 * 1024,
            "invalid configuration command"
        );
        let request = serde_json::to_string(&command)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some((original, receipt)) = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [command_id.to_string()],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()?
        {
            ensure!(original == request, "command ID payload conflict");
            return Ok(serde_json::from_str(&receipt)?);
        }
        let expiry = i64::try_from(*expires_at_ms)?;
        ensure!(
            expiry > now && expiry - now <= 300000,
            "invalid command deadline"
        );
        super::lifecycle::ensure_admissible(&tx, guard.session_id)?;
        let active: i64 = tx.query_row(
            "SELECT count(*) FROM runs WHERE session_id=?1 AND active=1",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        let access_only = matches!(&command, RuntimeCommand::SetAccess { .. });
        ensure!(
            (access_only && active > 0)
                || (active == 0
                    && super::catalogue::pending_cleanup(&tx, guard.session_id)?.is_none()),
            "configuration requires idle voyage and observed cleanup"
        );
        let mut saved = read_session(&tx, guard.session_id)?;
        ensure!(
            saved.revision == *expected_revision,
            "session revision conflict"
        );
        if access_only {
            ensure!(
                saved.session.model == model,
                "access change cannot switch model"
            );
            // Changing mode is not approval for a previously displayed effect.
            // Questions are deliberately left pending.
            tx.execute(
                "UPDATE process_decisions SET response='\"invalidated\"' WHERE response IS NULL AND json_extract(request, '$.kind')='approval' AND run_id IN (SELECT id FROM runs WHERE session_id=?1 AND active=1)",
                [guard.session_id.to_string()],
            )?;
        } else {
            saved.session.switch_model(model)?;
        }
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
        tx.execute("INSERT INTO process_configuration VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET settings=excluded.settings",params![guard.session_id.to_string(),settings])?;
        let receipt = json!({"command_id":command_id,"status":"applied","revision":saved.revision});
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                request,
                serde_json::to_string(&receipt)?
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(receipt)
    }
}
