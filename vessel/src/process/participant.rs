//! Durable locally accepted subordinate work, never a second owner of the parent session.
mod admission;
mod fence;
mod observation;
use super::{access::store, identity, registry, service::Supervisor};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use voyage_protocol::process::*;

#[derive(Clone, Serialize, Deserialize)]
struct Assignment {
    request: AssignmentRequest,
    principal_id: Uuid,
    source_grant: GrantBinding,
    child_grant_id: Uuid,
    start_command_id: Uuid,
    observation: AssignmentObservation,
    #[serde(default)]
    cancel: Option<RuntimeCommand>,
}
fn root(directory: &Path) -> PathBuf {
    directory.join("participants")
}
fn assignment_path(directory: &Path, id: Uuid) -> PathBuf {
    root(directory)
        .join("assignments")
        .join(format!("{id}.json"))
}
fn binding_path(directory: &Path, id: Uuid) -> PathBuf {
    root(directory).join("bindings").join(format!("{id}.json"))
}
fn initialize(directory: &Path) -> Result<()> {
    registry::private_directory(&root(directory))?;
    registry::private_directory(&root(directory).join("assignments"))?;
    registry::private_directory(&root(directory).join("bindings"))
}
impl Supervisor {
    pub(super) async fn assignment_lock(
        &self,
        id: Uuid,
    ) -> Result<std::sync::Arc<tokio::sync::Mutex<()>>> {
        let mut locks = self.assignment_locks.lock().await;
        ensure!(
            locks.contains_key(&id) || locks.len() < 4096,
            "assignment lock retention capacity exceeded"
        );
        Ok(locks
            .entry(id)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone())
    }

    pub(super) async fn participant_admin(
        &self,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        let _serial = self.registrations.lock().await;
        initialize(&self.directory)?;
        match &command {
            VesselCommand::AcceptParticipant {
                command_id,
                binding,
            } => {
                ensure!(
                    !binding.binding_id.is_nil()
                        && !binding.parent_session_id.is_nil()
                        && !binding.parent_vessel_id.is_nil()
                        && !binding.principal_id.is_nil()
                        && binding.revision == 1
                        && !binding.revoked
                        && !binding.cancel_existing,
                    "invalid participant binding"
                );
                ensure!(
                    (1..=32768).contains(&binding.max_context_bytes)
                        && (1..=256).contains(&binding.max_assignments),
                    "invalid participant bounds"
                );
                ensure!(
                    binding.expires_at_ms > store::now()?
                        && binding.expires_at_ms - store::now()? <= 30 * 86400000,
                    "participant binding expiry exceeds thirty days"
                );
                ensure!(
                    binding.workspace.is_absolute()
                        && binding.workspace.is_dir()
                        && std::fs::canonicalize(&binding.workspace)? == binding.workspace,
                    "participant workspace must be canonical"
                );
                if let Some(config) = &binding.config_path {
                    ensure!(
                        config.is_absolute() && config.is_file(),
                        "participant config must be local"
                    );
                }
                let path = binding_path(&self.directory, binding.binding_id);
                if registry::command_record(&self.directory, *command_id, &command, false)?
                    && path.exists()
                {
                    let previous: ParticipantBinding = store::load(&path)?;
                    return Ok(serde_json::to_value(previous)?);
                }
                ensure!(!path.exists(), "participant binding already exists");
                ensure!(
                    std::fs::read_dir(root(&self.directory).join("bindings"))?
                        .take(4096)
                        .count()
                        < 4096,
                    "participant binding capacity exceeded"
                );
                registry::command_record(&self.directory, *command_id, &command, true)?;
                store::save(&path, binding)?;
                Ok(serde_json::to_value(binding)?)
            }
            VesselCommand::RemoveParticipant {
                command_id,
                binding_id,
                expected_revision,
                cancel,
            } => {
                let path = binding_path(&self.directory, *binding_id);
                let mut binding: ParticipantBinding = store::load(&path)?;
                if registry::command_record(&self.directory, *command_id, &command, false)?
                    && binding.revision > *expected_revision
                {
                    return Ok(
                        serde_json::json!({"binding_id":binding_id,"revision":binding.revision,"revoked":binding.revoked,"cancel_existing":binding.cancel_existing}),
                    );
                }
                ensure!(
                    binding.revision == *expected_revision,
                    "participant binding revision conflict"
                );
                registry::command_record(&self.directory, *command_id, &command, true)?;
                binding.revoked = true;
                binding.cancel_existing = *cancel;
                binding.revision = binding
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("binding revision overflow"))?;
                store::save(&path, &binding)?;
                // Active child dispatch observes cancellation through its private binding authority.
                Ok(
                    serde_json::json!({"binding_id":binding_id,"revision":binding.revision,"disposition":if *cancel{"cancel"}else{"drain"},"cleanup":"pending"}),
                )
            }
            _ => anyhow::bail!("participant administration requires local owner"),
        }
    }
}
