//! Prepared, positively fenced ownership transfer. No timeout can authorize takeover.
mod destination;
mod prepare;
mod source;
mod status;
use super::{access::store, registry, service::Supervisor};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use voyage_protocol::process::*;

#[derive(Clone, Serialize, Deserialize)]
struct Prepared {
    preparation: SignedArtifact<TransferPreparation>,
    config_path: Option<PathBuf>,
    manifest: Option<SignedArtifact<TransferManifest>>,
    activated: bool,
    /// Persisted before entering startup. Presence is pending intent, never
    /// completion evidence or permission to repeat a startup effect.
    #[serde(default)]
    activation_command: Option<Uuid>,
    /// Captured before preparation effects, never reconstructed for old state.
    #[serde(default)]
    catalogue_namespace: Option<(u64, u64)>,
}
fn directory(root: &Path, id: Uuid) -> PathBuf {
    root.join("transfers").join(id.to_string())
}
fn load(root: &Path, id: Uuid) -> Result<Prepared> {
    store::load(&directory(root, id).join("prepared.json"))
}
fn save(root: &Path, id: Uuid, prepared: &Prepared) -> Result<()> {
    store::save(&directory(root, id).join("prepared.json"), prepared)
}
impl Supervisor {
    pub(super) async fn transfer(&self, command: VesselCommand) -> Result<serde_json::Value> {
        let destination_id = match &command {
            VesselCommand::PrepareTransfer { transfer_id, .. }
            | VesselCommand::UploadTransferChunk { transfer_id, .. }
            | VesselCommand::ActivateTransfer { transfer_id, .. }
            | VesselCommand::TransferStatus { transfer_id, .. } => Some(*transfer_id),
            VesselCommand::AcceptTransfer { manifest } => Some(manifest.payload.transfer_id),
            _ => None,
        };
        // Export already owns its source-side per-transfer lock. Destination
        // operations serialize intent/status/uploads before registrations.
        let lock = if let Some(id) = destination_id {
            Some(self.assignment_lock(id).await?)
        } else {
            None
        };
        let _transfer = if let Some(lock) = &lock {
            Some(lock.lock().await)
        } else {
            None
        };
        match command {
            command @ VesselCommand::PrepareTransfer { .. } => self.prepare_transfer(command).await,
            command @ VesselCommand::ExportTransfer { .. } => self
                .export_transfer(command)
                .await
                .map_err(|error| error.context(super::routing::OutcomeUnknown)),
            VesselCommand::AcceptTransfer { manifest } => self.accept_transfer(manifest).await,
            VesselCommand::TransferChunk {
                transfer_id,
                offset,
                limit,
            } => self.transfer_chunk(transfer_id, offset, limit).await,
            VesselCommand::UploadTransferChunk {
                transfer_id,
                offset,
                data,
            } => self.upload_transfer(transfer_id, offset, data).await,
            command @ VesselCommand::ActivateTransfer { .. } => self
                .activate_transfer(command)
                .await
                .map_err(|error| error.context(super::routing::OutcomeUnknown)),
            VesselCommand::TransferStatus {
                transfer_id,
                activate_command_id,
                expected_manifest_digest,
                manifest,
            } => self
                .transfer_status(
                    transfer_id,
                    activate_command_id,
                    expected_manifest_digest,
                    manifest,
                )
                .await
                .and_then(|status| serde_json::to_value(status).map_err(Into::into)),
            _ => anyhow::bail!("unsupported transfer operation"),
        }
    }
    pub(super) fn check_transfer_retention(&self) -> Result<()> {
        let root = self.directory.join("transfers");
        if !root.exists() {
            return Ok(());
        }
        let mut retained = 0;
        for entry in std::fs::read_dir(root)?.take(4097) {
            retained += 1;
            let path = entry?.path().join("prepared.json");
            if !path.exists() {
                continue;
            }
            let _: Prepared = store::load(&path)?;
        }
        ensure!(
            retained <= 4096,
            "transfer receipt retention capacity exceeded"
        );
        Ok(())
    }
}

#[cfg(test)]
mod transfer_final_tests;

#[cfg(test)]
mod transfer_status_tests;
