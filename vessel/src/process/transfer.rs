//! Prepared, positively fenced ownership transfer. No timeout can authorize takeover.
mod destination;
mod prepare;
mod source;
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
            _ => anyhow::bail!("unsupported transfer operation"),
        }
    }
    pub(super) fn reserved_transfers(&self, except: Option<Uuid>) -> Result<usize> {
        let root = self.directory.join("transfers");
        if !root.exists() {
            return Ok(0);
        }
        let mut reserved = 0;
        let mut retained = 0;
        for entry in std::fs::read_dir(root)?.take(4097) {
            retained += 1;
            let path = entry?.path().join("prepared.json");
            if !path.exists() {
                continue;
            }
            let prepared: Prepared = store::load(&path)?;
            if !prepared.activated && Some(prepared.preparation.payload.transfer_id) != except {
                reserved += 1;
            }
        }
        ensure!(
            retained <= 4096,
            "transfer receipt retention capacity exceeded"
        );
        Ok(reserved)
    }
}
