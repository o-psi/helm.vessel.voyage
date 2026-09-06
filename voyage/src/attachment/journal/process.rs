//! Atomic process-command receipts and metadata updates under the session fence.
use super::*;
use anyhow::bail;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;

impl Journal {
    pub(crate) fn process_latest_run(&self, session_id: Uuid) -> Result<Option<RunRecord>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM runs WHERE session_id=?1 ORDER BY rowid DESC LIMIT 1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.run(Uuid::parse_str(&id)?)).transpose()
    }
    pub(crate) fn initialize_process_commands(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_commands(id TEXT PRIMARY KEY, request TEXT NOT NULL, receipt TEXT NOT NULL)")?;
        Ok(())
    }
    pub(crate) fn process_receipt(&self, id: Uuid) -> Result<Option<Value>> {
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT receipt FROM process_commands WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(saved) = saved {
            return Ok(Some(serde_json::from_str(&saved)?));
        }
        let run: Option<String> = self
            .connection
            .query_row(
                "SELECT run_id FROM commands WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(run) = run {
            let run = self.run(Uuid::parse_str(&run)?)?;
            return Ok(Some(
                json!({"run_id":run.id,"command_id":id,"status":"accepted","state":run.state}),
            ));
        }
        let steering: Option<String> = self
            .connection
            .query_row(
                "SELECT record FROM steering WHERE id=?1",
                [id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        steering
            .map(|record| serde_json::from_str(&record).map_err(Into::into))
            .transpose()
    }
    pub(crate) fn process_metadata(
        &mut self,
        guard: &ExecutionGuard,
        actor: super::super::local_actor::LocalActor,
        command: &RuntimeCommand,
        now: i64,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        let (id, expected, expiry) = match command {
            RuntimeCommand::Rename {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::SetModel {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Cancel {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            } => (*command_id, *expected_revision, *expires_at_ms),
            _ => bail!("unsupported metadata operation"),
        };
        ensure!(!id.is_nil(), "nil command ID");
        let request = serde_json::to_string(command)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved: Option<(String, String)> = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((original, receipt)) = saved {
            ensure!(original == request, "command ID payload conflict");
            return Ok(serde_json::from_str(&receipt)?);
        }
        let collisions: i64 = tx.query_row("SELECT (SELECT count(*) FROM commands WHERE id=?1)+(SELECT count(*) FROM steering WHERE id=?1)",[id.to_string()],|row|row.get(0))?;
        ensure!(collisions == 0, "command ID already used");
        let expiry = i64::try_from(expiry)?;
        ensure!(
            expiry > now && expiry - now <= 300_000,
            "invalid command deadline"
        );
        let mut saved = read_session(&tx, guard.session_id)?;
        ensure!(saved.revision == expected, "session revision conflict");
        let mut next = saved.revision;
        let receipt = match command {
            RuntimeCommand::Cancel { run_id, .. } => {
                let run = read_run(&tx, *run_id)?;
                ensure!(
                    run.session_id == guard.session_id
                        && run.machine_id == actor.installation_id
                        && run.principal_id == actor.principal_id,
                    "run authority mismatch"
                );
                let active = matches!(run.state, RunState::Accepted | RunState::Running);
                if active {
                    tx.execute(
                        "INSERT OR IGNORE INTO local_cancel_intents VALUES(?1,?2,?3,?4,?5,?6)",
                        params![
                            run_id.to_string(),
                            guard.session_id.to_string(),
                            actor.installation_id.to_string(),
                            actor.principal_id.to_string(),
                            now,
                            expiry
                        ],
                    )?;
                }
                json!({"command_id":id,"run_id":run_id,"status":if active {"requested"} else {"already_terminal"},"state":run.state,"revision":next})
            }
            RuntimeCommand::Rename { name, .. } | RuntimeCommand::SetModel { model: name, .. } => {
                let active: i64 = tx.query_row(
                    "SELECT count(*) FROM runs WHERE session_id=?1 AND active=1",
                    [guard.session_id.to_string()],
                    |row| row.get(0),
                )?;
                ensure!(active == 0, "metadata change requires idle voyage");
                ensure!(
                    !name.trim().is_empty()
                        && name.len() <= 512
                        && !name.chars().any(char::is_control),
                    "invalid name or model"
                );
                match command {
                    RuntimeCommand::Rename { name, .. } => saved.session.set_name(name.clone()),
                    RuntimeCommand::SetModel { model, .. } => {
                        saved.session.switch_model(model.clone())?;
                    }
                    _ => unreachable!(),
                }
                next = next.checked_add(1).context("revision overflow")?;
                saved.session.revision = next;
                tx.execute(
                    "UPDATE sessions SET revision=?1,state=?2 WHERE id=?3",
                    params![
                        i64::try_from(next)?,
                        snapshot(&saved.session)?,
                        guard.session_id.to_string()
                    ],
                )?;
                json!({"command_id":id,"status":"applied","revision":next})
            }
            _ => unreachable!(),
        };
        let count: i64 = tx.query_row("SELECT count(*) FROM process_commands", [], |row| {
            row.get(0)
        })?;
        ensure!(count < 100_000, "process command receipt capacity reached");
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![id.to_string(), request, serde_json::to_string(&receipt)?],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(receipt)
    }
}

pub(super) fn receipt_exists(db: &Connection, id: Uuid) -> Result<bool> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='process_commands')",
        [],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM process_commands WHERE id=?1)",
        [id.to_string()],
        |row| row.get(0),
    )?)
}
