//! Lifecycle orchestration passes private initialization provenance, never transcripts.
use super::{registry, service::Supervisor};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use voyage_protocol::process::*;
impl Supervisor {
    pub(super) async fn initialize_managed(&self, command: VesselCommand) -> Result<Value> {
        let VesselCommand::ManagedImport {
            command_id,
            session_id,
            workspace,
            source_directory,
            expected_revision,
            config_path,
        } = &command
        else {
            anyhow::bail!("not managed import")
        };
        ensure!(
            source_directory.is_absolute() && source_directory.is_dir(),
            "managed source must be an absolute host installation directory"
        );
        let initialize = RuntimeInitialization::ManagedImport {
            transfer_id: *command_id,
            source_directory: source_directory.clone(),
            expected_revision: *expected_revision,
        };
        self.start_initialized(
            *command_id,
            *session_id,
            workspace.clone(),
            config_path.clone(),
            Some(initialize),
            command,
        )
        .await
    }

    pub(super) async fn initialize_import(&self, command: VesselCommand) -> Result<Value> {
        let VesselCommand::Import {
            command_id,
            session_id,
            workspace,
            source_directory,
            expected_revision,
            source_sha256,
            config_path,
        } = &command
        else {
            anyhow::bail!("not import")
        };
        ensure!(
            source_directory.is_absolute() && source_directory.is_dir(),
            "import source must be an absolute host directory"
        );
        ensure!(
            source_sha256.len() == 64
                && source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "invalid source SHA256"
        );
        let initialize = RuntimeInitialization::Import {
            transfer_id: *command_id,
            source_directory: source_directory.clone(),
            expected_revision: *expected_revision,
            source_sha256: source_sha256.clone(),
        };
        self.start_initialized(
            *command_id,
            *session_id,
            workspace.clone(),
            config_path.clone(),
            Some(initialize),
            command,
        )
        .await
    }
    pub(super) async fn branch(&self, command: VesselCommand) -> Result<Value> {
        let VesselCommand::Branch {
            command_id,
            session_id,
            incarnation,
            expected_revision,
            expires_at_ms,
            branch_id,
            name,
        } = &command
        else {
            anyhow::bail!("not branch")
        };
        ensure!(
            !branch_id.is_nil() && branch_id != session_id,
            "invalid branch identity"
        );
        let registrations = self.registrations.lock().await?;
        let source = registrations
            .get(session_id)
            .context("source session not registered")?
            .clone();
        ensure!(
            source.incarnation == *incarnation,
            "stale source incarnation"
        );
        drop(registrations);
        let source_directory = registry::directory(&self.directory, *session_id);
        let frozen = self
            .forward_resuming(
                *session_id,
                *incarnation,
                RuntimeCommand::Branch {
                    command_id: *command_id,
                    expected_revision: *expected_revision,
                    expires_at_ms: *expires_at_ms,
                    branch_id: *branch_id,
                    name: name.clone(),
                },
                None,
            )
            .await?;
        ensure!(
            frozen.error.is_none(),
            "branch snapshot refused: {}",
            frozen.error.unwrap_or_default()
        );
        ensure!(
            frozen.result["status"] == "snapshot_committed",
            "branch snapshot no longer available"
        );
        let initialization = RuntimeInitialization::Branch {
            source_directory,
            source_session_id: *session_id,
            source_command_id: *command_id,
            branch_id: *branch_id,
        };
        let process = self
            .start_initialized(
                *command_id,
                *branch_id,
                source.workspace,
                source.config_path,
                Some(initialization),
                command,
            )
            .await?;
        Ok(process)
    }
}
