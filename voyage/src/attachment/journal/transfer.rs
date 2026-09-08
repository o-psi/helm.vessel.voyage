//! Positive source relinquishment is irreversible; portable text never carries credentials.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::{PortableCheckpoint, RuntimeCommand, RuntimeInitialization};
impl Journal {
    pub(crate) fn relinquish(
        &mut self,
        guard: &ExecutionGuard,
        command: &RuntimeCommand,
        now: i64,
    ) -> Result<(Value, Vec<u8>)> {
        self.check_guard(guard, guard.session_id)?;
        let RuntimeCommand::Relinquish {
            command_id,
            expected_revision,
            expires_at_ms,
            transfer_id,
            destination_vessel_id,
            prepare_digest,
        } = command
        else {
            anyhow::bail!("not relinquishment")
        };
        ensure!(
            !transfer_id.is_nil() && !destination_vessel_id.is_nil() && prepare_digest.len() == 64,
            "invalid transfer identity"
        );
        self.connection.execute_batch("CREATE TABLE IF NOT EXISTS process_transfers(id TEXT PRIMARY KEY,command_id TEXT NOT NULL,artifact TEXT NOT NULL)")?;
        let request = serde_json::to_string(command)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let old: Option<(String, String)> = tx
            .query_row(
                "SELECT request,receipt FROM process_commands WHERE id=?1",
                [command_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((saved, receipt)) = old {
            ensure!(saved == request, "transfer command payload conflict");
            let artifact: String = tx.query_row(
                "SELECT artifact FROM process_transfers WHERE id=?1 AND command_id=?2",
                params![transfer_id.to_string(), command_id.to_string()],
                |r| r.get(0),
            )?;
            return Ok((serde_json::from_str(&receipt)?, artifact.into_bytes()));
        }
        lifecycle::ensure_admissible(&tx, guard.session_id)?;
        let mut saved = read_session(&tx, guard.session_id)?;
        ensure!(
            saved.revision == *expected_revision,
            "session revision conflict"
        );
        let expiry = i64::try_from(*expires_at_ms)?;
        ensure!(
            expiry > now && expiry - now <= 300_000,
            "transfer command deadline invalid"
        );
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id=?1 AND active=1)",
            [guard.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            session_resources::pending(&tx, guard.session_id)? == 0,
            "session resources still owned or cleanup unknown"
        );
        ensure!(
            !active && catalogue::pending_cleanup(&tx, guard.session_id)?.is_none(),
            "owner move requires idle observed cleanup"
        );
        let generation: u64 = tx
            .query_row(
                "SELECT generation FROM process_lifecycle WHERE session_id=?1",
                [guard.session_id.to_string()],
                |r| r.get::<_, i64>(0),
            )?
            .try_into()?;
        let generation = generation
            .checked_add(1)
            .context("ownership generation overflow")?;
        let mut portable =
            Session::new(saved.session.workspace.clone(), saved.session.model.clone());
        portable.id = guard.session_id;
        portable.parent_id = saved.session.parent_id;
        portable.revision = saved.revision.checked_add(1).context("revision overflow")?;
        portable.created_at = saved.session.created_at;
        portable.model_history = saved.session.model_history.clone();
        portable.name = saved.session.name.clone();
        portable.title_state = saved.session.title_state.clone();
        portable.messages = saved.session.messages.clone();
        portable.usage = saved.session.usage.clone();
        for message in &mut portable.messages {
            message.provider_state = None;
        }
        let artifact = serde_json::to_vec(&PortableCheckpoint {
            transfer_id: *transfer_id,
            session_id: guard.session_id,
            destination_vessel_id: *destination_vessel_id,
            prepare_digest: prepare_digest.clone(),
            generation,
            session: serde_json::to_value(portable)?,
            commands: tombstones::collect(&tx)?,
        })?;
        ensure!(
            artifact.len() <= MAX_SNAPSHOT,
            "portable snapshot exceeds bound"
        );
        tx.execute(
            "INSERT INTO process_transfers VALUES(?1,?2,?3)",
            params![
                transfer_id.to_string(),
                command_id.to_string(),
                std::str::from_utf8(&artifact)?
            ],
        )?;
        tx.execute("UPDATE process_lifecycle SET archived=1,transfer_id=?1,generation=?2 WHERE session_id=?3",params![transfer_id.to_string(),i64::try_from(generation)?,guard.session_id.to_string()])?;
        let revision = saved.revision.checked_add(1).context("revision overflow")?;
        saved.session.revision = revision;
        tx.execute(
            "UPDATE sessions SET revision=?1,state=?2 WHERE id=?3",
            params![
                i64::try_from(revision)?,
                snapshot(&saved.session)?,
                guard.session_id.to_string()
            ],
        )?;
        let receipt = json!({"command_id":command_id,"transfer_id":transfer_id,"session_id":guard.session_id,"status":"relinquished","revision":revision,"artifact_sha256":hex::encode(Sha256::digest(&artifact)),"artifact_bytes":artifact.len(),"generation":generation});
        tx.execute(
            "INSERT INTO process_commands VALUES(?1,?2,?3)",
            params![
                command_id.to_string(),
                request,
                serde_json::to_string(&receipt)?
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok((receipt, artifact))
    }
    pub(crate) fn import_transfer(
        &mut self,
        initialization: &RuntimeInitialization,
        bytes: &[u8],
        session: Uuid,
        workspace: &Path,
    ) -> Result<()> {
        let RuntimeInitialization::Transfer {
            transfer_id,
            sha256,
            prepare_digest,
            generation,
            ..
        } = initialization
        else {
            anyhow::bail!("not transfer initialization")
        };
        ensure!(
            bytes.len() <= MAX_SNAPSHOT && hex::encode(Sha256::digest(bytes)) == *sha256,
            "transfer artifact digest mismatch"
        );
        let portable: PortableCheckpoint = serde_json::from_slice(bytes)?;
        ensure!(
            portable.transfer_id == *transfer_id
                && portable.session_id == session
                && portable.prepare_digest == *prepare_digest
                && portable.generation == *generation
                && *generation > 0,
            "transfer artifact binding mismatch"
        );
        let mut restored: Session = serde_json::from_value(portable.session)?;
        ensure!(
            restored.id == session
                && restored.messages.iter().all(|m| m.provider_state.is_none())
                && restored.terminals.is_empty()
                && restored.completion_runs.is_empty()
                && restored.workflow_runs.is_empty(),
            "portable checkpoint contains foreign/live state"
        );
        restored.workspace = workspace.to_path_buf();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS process_transfer_import(session_id TEXT PRIMARY KEY,transfer_id TEXT NOT NULL,sha256 TEXT NOT NULL,generation INTEGER NOT NULL)")?;
        let old:Option<(String,String,i64)>=tx.query_row("SELECT transfer_id,sha256,generation FROM process_transfer_import WHERE session_id=?1",[session.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((id, hash, prior)) = old {
            ensure!(
                id == transfer_id.to_string()
                    && hash == *sha256
                    && prior == i64::try_from(*generation)?,
                "destination transfer provenance conflict"
            );
            return Ok(());
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            [session.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!exists, "destination already owns session identity");
        steering::validate_snapshot_ids(&tx, &restored)?;
        tombstones::import(&tx, &portable.commands)?;
        tx.execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,?2,?3)",
            params![
                session.to_string(),
                i64::try_from(restored.revision)?,
                snapshot(&restored)?
            ],
        )?;
        tx.execute(
            "INSERT INTO process_transfer_import VALUES(?1,?2,?3,?4)",
            params![
                session.to_string(),
                transfer_id.to_string(),
                sha256,
                i64::try_from(*generation)?
            ],
        )?;
        commit(tx, &self.commit_fence)
    }
}
