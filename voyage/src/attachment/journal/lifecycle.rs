//! Session disposition and immutable branch publication share the owner's transaction.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;
impl Journal {
    pub(crate) fn initialize_lifecycle(&mut self, guard: &ExecutionGuard) -> Result<()> {
        self.check_guard(guard, guard.session_id)?;
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_lifecycle(session_id TEXT PRIMARY KEY REFERENCES sessions(id),archived INTEGER NOT NULL DEFAULT 0,deleted INTEGER NOT NULL DEFAULT 0); CREATE TABLE IF NOT EXISTS process_branches(command_id TEXT PRIMARY KEY,source_session_id TEXT NOT NULL,branch_id TEXT NOT NULL UNIQUE,snapshot TEXT NOT NULL,digest TEXT NOT NULL)")?;
        let branch_columns = {
            let mut query = self
                .connection
                .prepare("PRAGMA table_info(process_branches)")?;
            query
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !branch_columns.iter().any(|name| name == "configuration") {
            self.connection
                .execute_batch("ALTER TABLE process_branches ADD COLUMN configuration TEXT")?;
        }
        let columns = {
            let mut query = self
                .connection
                .prepare("PRAGMA table_info(process_lifecycle)")?;
            query
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !columns.iter().any(|name| name == "transfer_id") {
            self.connection.execute_batch("ALTER TABLE process_lifecycle ADD COLUMN transfer_id TEXT; ALTER TABLE process_lifecycle ADD COLUMN generation INTEGER NOT NULL DEFAULT 0")?;
        }
        self.connection.execute(
            "INSERT OR IGNORE INTO process_lifecycle(session_id) VALUES(?1)",
            [guard.session_id.to_string()],
        )?;
        let imported: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_transfer_import')",
            [],
            |r| r.get(0),
        )?;
        if imported {
            self.connection.execute("UPDATE process_lifecycle SET generation=max(generation,coalesce((SELECT generation FROM process_transfer_import WHERE session_id=?1),0)) WHERE session_id=?1",[guard.session_id.to_string()])?;
        }
        Ok(())
    }
    pub(crate) fn lifecycle_status(&self, session: Uuid) -> Result<Value> {
        let (archived,deleted,transfer,generation):(bool,bool,Option<String>,i64)=self.connection.query_row("SELECT archived,deleted,transfer_id,generation FROM process_lifecycle WHERE session_id=?1",[session.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
        Ok(
            json!({"archived":archived,"deleted":deleted,"transfer_id":transfer,"generation":generation}),
        )
    }
    pub(crate) fn apply_lifecycle(
        &mut self,
        guard: &ExecutionGuard,
        command: &RuntimeCommand,
        now: i64,
        configuration: Option<&str>,
    ) -> Result<Value> {
        self.check_guard(guard, guard.session_id)?;
        let (id, revision, expiry) = match command {
            RuntimeCommand::Clear {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Compact {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Archive {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Delete {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            }
            | RuntimeCommand::Branch {
                command_id,
                expected_revision,
                expires_at_ms,
                ..
            } => (*command_id, *expected_revision, *expires_at_ms),
            _ => anyhow::bail!("unsupported lifecycle command"),
        };
        ensure!(!id.is_nil(), "nil command ID");
        let encoded = serde_json::to_string(command)?;
        if matches!(command, RuntimeCommand::Delete { .. }) {
            self.connection.execute_batch("PRAGMA secure_delete=ON")?;
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((request, receipt)) = old {
            ensure!(request == encoded, "command ID payload conflict");
            return Ok(serde_json::from_str(&receipt)?);
        }
        let collisions:i64=tx.query_row("SELECT (SELECT count(*) FROM commands WHERE id=?1)+(SELECT count(*) FROM steering WHERE id=?1)",[id.to_string()],|r|r.get(0))?;
        ensure!(collisions == 0, "command ID already used");
        let expiry = i64::try_from(expiry)?;
        ensure!(
            expiry > now && expiry - now <= 300_000,
            "invalid lifecycle deadline"
        );
        let mut saved = read_session(&tx, guard.session_id)?;
        ensure!(saved.revision == revision, "session revision conflict");
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND active=1)",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        if !matches!(command, RuntimeCommand::Branch { .. }) {
            ensure!(
                session_resources::pending(&tx, guard.session_id)? == 0,
                "session resources still owned or cleanup unknown"
            );
        }
        ensure!(
            !active && catalogue::pending_cleanup(&tx, guard.session_id)?.is_none(),
            "lifecycle change requires idle voyage and observed cleanup"
        );
        if matches!(
            command,
            RuntimeCommand::Clear { .. }
                | RuntimeCommand::Compact { .. }
                | RuntimeCommand::Delete { .. }
        ) {
            let retained: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_retained_cleanup')",
                [],
                |r| r.get(0),
            )?;
            if retained {
                let unknown: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM process_retained_cleanup WHERE session_id=?1 AND confirmation IS NULL) OR EXISTS(SELECT 1 FROM process_session_resources WHERE session_id=?1 AND state='retained_unknown')", [guard.session_id.to_string()], |r| r.get(0))?;
                ensure!(
                    !unknown,
                    "retained unknown effects require preserving conversation history"
                );
            }
        }
        let transferred: bool = tx.query_row(
            "SELECT transfer_id IS NOT NULL FROM process_lifecycle WHERE session_id=?1",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!transferred, "session ownership permanently relinquished");
        let deleted: bool = tx.query_row(
            "SELECT deleted FROM process_lifecycle WHERE session_id=?1",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            !deleted,
            "session was deleted; retained tombstone prevents resurrection"
        );
        let next = revision.checked_add(1).context("revision overflow")?;
        let mut receipt = json!({"command_id":id,"session_id":guard.session_id,"status":"applied","revision":next});
        match command {
            RuntimeCommand::Clear {
                confirm_session_id, ..
            } => {
                ensure!(
                    *confirm_session_id == guard.session_id,
                    "clear requires exact session confirmation"
                );
                saved.session.clear_conversation();
                receipt["cleared"] = json!(true);
            }
            RuntimeCommand::Compact { retain, .. } => {
                ensure!(
                    (1..=100_000).contains(retain),
                    "retain must be 1..100000 messages"
                );
                receipt["compacted_messages"] = json!(saved.session.compact(*retain as usize)?);
                receipt["canonical_preserved"] = json!(true);
                receipt["removed_messages"] = json!(0);
            }
            RuntimeCommand::Archive { archived, .. } => {
                tx.execute(
                    "UPDATE process_lifecycle SET archived=?1 WHERE session_id=?2",
                    params![archived, guard.session_id.to_string()],
                )?;
                receipt["archived"] = json!(archived);
            }
            RuntimeCommand::Delete {
                confirm_session_id, ..
            } => {
                ensure!(
                    *confirm_session_id == guard.session_id,
                    "deletion requires exact session confirmation"
                );
                tx.execute(
                    "UPDATE process_lifecycle SET archived=1,deleted=1 WHERE session_id=?1",
                    [guard.session_id.to_string()],
                )?;
                saved.session.messages.clear();
                saved.session.terminals.clear();
                saved.session.completion_runs.clear();
                saved.session.run_summaries.clear();
                saved.session.workflow_runs.clear();
                saved.session.github_references.clear();
                saved.session.draft.clear();
                saved.session.name = None;
                saved.session.title_state = None;
                tx.execute("UPDATE runs SET record=json_set(record,'$.partial_text','') WHERE session_id=?1",[guard.session_id.to_string()])?;
                tx.execute(
                    "DELETE FROM events WHERE session_id=?1",
                    [guard.session_id.to_string()],
                )?;
                super::deletion::scrub(&tx, guard.session_id)?;
                receipt["deleted"] = json!(true);
                receipt["status"] = json!("cleanup_pending");
            }
            RuntimeCommand::Branch {
                branch_id, name, ..
            } => {
                ensure!(
                    !branch_id.is_nil() && *branch_id != guard.session_id,
                    "branch requires a different nonnil identity"
                );
                if let Some(name) = name {
                    ensure!(
                        !name.trim().is_empty()
                            && name.len() <= 512
                            && !name.chars().any(char::is_control),
                        "invalid branch name"
                    );
                }
                let mut branch = Session::new(
                    saved.session.workspace.clone(),
                    saved
                        .session
                        .pending_model
                        .clone()
                        .unwrap_or_else(|| saved.session.model.clone()),
                );
                branch.id = *branch_id;
                branch.parent_id = Some(guard.session_id);
                // Session::new used a different UUID; rebuild title provenance
                // for the final branch identity, preserving explicit names.
                branch.name = None;
                branch.title_state = None;
                branch.clear_conversation();
                if let Some(name) = name {
                    branch.set_name(name.clone());
                }
                branch.messages = saved.session.messages.clone();
                for message in &mut branch.messages {
                    message.provider_state = None;
                }
                let payload = snapshot(&branch)?;
                let configuration = configuration.context("branch launch configuration missing")?;
                ensure!(
                    configuration.len() <= 1024 * 1024,
                    "branch launch configuration exceeds limit"
                );
                let mut hash = Sha256::new();
                hash.update(payload.as_bytes());
                hash.update(configuration.as_bytes());
                let digest = hex::encode(hash.finalize());
                tx.execute(
                    "INSERT INTO process_branches(command_id,source_session_id,branch_id,snapshot,digest,configuration) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        id.to_string(),
                        guard.session_id.to_string(),
                        branch_id.to_string(),
                        payload,
                        digest,configuration
                    ],
                )?;
                receipt["branch_id"] = json!(branch_id);
                receipt["source_revision"] = json!(revision);
                receipt["status"] = json!("snapshot_committed");
            }
            _ => unreachable!(),
        }
        saved.session.revision = next;
        tx.execute(
            "UPDATE sessions SET revision=?1,state=?2 WHERE id=?3",
            params![
                i64::try_from(next)?,
                snapshot(&saved.session)?,
                guard.session_id.to_string()
            ],
        )?;
        let count: i64 = tx.query_row("SELECT count(*) FROM process_commands", [], |r| r.get(0))?;
        ensure!(count < MAX_COMMANDS, "command receipt capacity reached");
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![id.to_string(), encoded, serde_json::to_string(&receipt)?],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(receipt)
    }
}
pub(super) fn ensure_admissible(db: &Connection, session: Uuid) -> Result<()> {
    let resources: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_session_resources')",
        [],
        |r| r.get(0),
    )?;
    if resources {
        let unknown:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM process_session_resources WHERE session_id=?1 AND state='cleanup_unknown')",[session.to_string()],|r|r.get(0))?;
        ensure!(!unknown, "session resource cleanup unknown");
    }
    let exists:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='process_lifecycle')",[],|r|r.get(0))?;
    if exists {
        let blocked:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM process_lifecycle WHERE session_id=?1 AND (archived=1 OR deleted=1))",[session.to_string()],|r|r.get(0))?;
        ensure!(!blocked, "session is archived or deleted");
    }
    Ok(())
}
