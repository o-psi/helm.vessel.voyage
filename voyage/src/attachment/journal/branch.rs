//! Immutable owner-created branch snapshots are imported only by a destination runtime.
use super::*;
use voyage_protocol::process::RuntimeInitialization;
impl Journal {
    pub(crate) fn import_process_branch(
        &mut self,
        initialization: &RuntimeInitialization,
        workspace: &Path,
    ) -> Result<()> {
        let RuntimeInitialization::Branch {
            source_directory,
            source_session_id,
            source_command_id,
            branch_id,
        } = initialization
        else {
            anyhow::bail!("not branch initialization")
        };
        let provenance = serde_json::to_string(initialization)?;
        let imported: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_branch_import')",
            [],
            |r| r.get(0),
        )?;
        if imported {
            let prior: Option<String> = self
                .connection
                .query_row(
                    "SELECT provenance FROM process_branch_import WHERE session_id=?1",
                    [branch_id.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                ensure!(prior == provenance, "branch import provenance conflict");
                self.load_session(*branch_id)?;
                return Ok(());
            }
        }
        let source = Journal::open(source_directory.join("journal"))?;
        let (actual_source,actual_branch,payload,digest,configuration):(String,String,String,String,Option<String>)=source.connection.query_row("SELECT source_session_id,branch_id,snapshot,digest,configuration FROM process_branches WHERE command_id=?1",[source_command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(
            actual_source == source_session_id.to_string()
                && actual_branch == branch_id.to_string(),
            "branch provenance mismatch"
        );
        let mut hash = Sha256::new();
        hash.update(payload.as_bytes());
        if let Some(configuration) = &configuration {
            ensure!(
                configuration.len() <= 1024 * 1024,
                "branch configuration exceeds limit"
            );
            hash.update(configuration.as_bytes());
        }
        ensure!(
            payload.len() <= MAX_SNAPSHOT && hex::encode(hash.finalize()) == digest,
            "branch snapshot integrity mismatch"
        );
        let branch: Session = serde_json::from_str(&payload)?;
        ensure!(
            branch.id == *branch_id
                && branch.parent_id == Some(*source_session_id)
                && branch.workspace.canonicalize()? == workspace,
            "branch destination mismatch"
        );
        ensure!(
            branch.messages.iter().all(|m| m.provider_state.is_none())
                && branch.terminals.is_empty()
                && branch.completion_runs.is_empty()
                && branch.workflow_runs.is_empty(),
            "branch retains live runtime state"
        );
        // Persist destination-owned blobs before admitting the branch snapshot.
        // A crash leaves only bounded unreferenced blobs; the idempotent import
        // verifies/copies them again without rewriting canonical image identities.
        if branch.messages.iter().any(|m| !m.parts.is_empty()) {
            let images = crate::images::Store::open(&source.directory, *source_session_id)?;
            let mut destination = crate::images::Store::open(&self.directory, *branch_id)?;
            for part in branch.messages.iter().flat_map(|m| &m.parts) {
                if let voyage_protocol::content::ContentPart::Image { attachment } = part {
                    ensure!(
                        images.copy_to(&mut destination, attachment)? == *attachment,
                        "branch image identity changed"
                    );
                }
            }
        }
        let provenance = serde_json::to_string(initialization)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS process_branch_import(session_id TEXT PRIMARY KEY,provenance TEXT NOT NULL,digest TEXT NOT NULL)")?;
        let prior: Option<(String, String)> = tx
            .query_row(
                "SELECT provenance,digest FROM process_branch_import WHERE session_id=?1",
                [branch_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((saved, hash)) = prior {
            ensure!(
                saved == provenance && hash == digest,
                "branch import provenance conflict"
            );
            return Ok(());
        }
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1)",
            [branch_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!exists, "branch destination identity already exists");
        steering::validate_snapshot_ids(&tx, &branch)?;
        tx.execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,0,?2)",
            params![branch_id.to_string(), payload],
        )?;
        if let Some(configuration) = configuration {
            tx.execute_batch("CREATE TABLE IF NOT EXISTS process_configuration(session_id TEXT PRIMARY KEY,settings TEXT NOT NULL)")?;
            tx.execute(
                "INSERT INTO process_configuration VALUES(?1,?2)",
                params![branch_id.to_string(), configuration],
            )?;
        }
        tx.execute(
            "INSERT INTO process_branch_import VALUES(?1,?2,?3)",
            params![branch_id.to_string(), provenance, digest],
        )?;
        commit(tx, &self.commit_fence)
    }
    pub(crate) fn frozen_branch_configuration(
        initialization: Option<&RuntimeInitialization>,
    ) -> Result<Option<String>> {
        let Some(RuntimeInitialization::Branch {
            source_directory,
            source_session_id,
            source_command_id,
            branch_id,
        }) = initialization
        else {
            return Ok(None);
        };
        let source = Journal::open(source_directory.join("journal"))?;
        let (session,branch,payload,digest,configuration):(String,String,String,String,Option<String>)=source.connection.query_row("SELECT source_session_id,branch_id,snapshot,digest,configuration FROM process_branches WHERE command_id=?1",[source_command_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        ensure!(
            session == source_session_id.to_string() && branch == branch_id.to_string(),
            "frozen branch configuration identity mismatch"
        );
        let mut hash = Sha256::new();
        hash.update(payload.as_bytes());
        if let Some(settings) = &configuration {
            ensure!(
                settings.len() <= 1024 * 1024,
                "frozen branch configuration exceeds limit"
            );
            hash.update(settings.as_bytes());
        }
        ensure!(
            hex::encode(hash.finalize()) == digest,
            "frozen branch configuration integrity mismatch"
        );
        Ok(configuration)
    }
}
