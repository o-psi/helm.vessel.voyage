use super::*;
use crate::process::identity;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, Write},
    os::unix::fs::OpenOptionsExt,
};
impl Supervisor {
    pub(super) async fn accept_transfer(
        &self,
        manifest: SignedArtifact<TransferManifest>,
    ) -> Result<serde_json::Value> {
        let _serial = self.registrations.lock().await?;
        let id = manifest.payload.transfer_id;
        let mut prepared = load(&self.directory, id)?;
        identity::verify(
            &self.directory,
            manifest.payload.source_vessel_id,
            &manifest,
        )?;
        ensure!(
            manifest.payload.destination_vessel_id == identity::public(&self.directory)?.vessel_id
                && manifest.payload.source_vessel_id
                    == prepared.preparation.payload.source_vessel_id
                && manifest.payload.session_id == prepared.preparation.payload.session_id
                && manifest.payload.prepare_digest == identity::digest(&prepared.preparation)?
                && manifest.payload.generation > 0,
            "transfer manifest binding mismatch"
        );
        ensure!(
            manifest.payload.artifact_bytes > 0
                && manifest.payload.artifact_bytes <= 16 * 1024 * 1024
                && manifest.payload.artifact_sha256.len() == 64,
            "transfer artifact exceeds limit"
        );
        if let Some(previous) = &prepared.manifest {
            ensure!(
                identity::digest(previous)? == identity::digest(&manifest)?,
                "transfer manifest changed"
            );
        }
        prepared.manifest = Some(manifest);
        save(&self.directory, id, &prepared)?;
        Ok(serde_json::json!({"transfer_id":id,"state":"receiving"}))
    }
    pub(super) async fn upload_transfer(
        &self,
        id: Uuid,
        offset: u64,
        data: String,
    ) -> Result<serde_json::Value> {
        ensure!(data.len() <= 90000, "transfer chunk exceeds limit");
        let bytes = STANDARD.decode(data)?;
        ensure!(
            !bytes.is_empty() && bytes.len() <= 65536,
            "invalid transfer chunk"
        );
        let _serial = self.registrations.lock().await?;
        let prepared = load(&self.directory, id)?;
        ensure!(!prepared.activated, "transfer already activated");
        let manifest = prepared
            .manifest
            .ok_or_else(|| anyhow::anyhow!("transfer manifest not accepted"))?;
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("transfer offset overflow"))?;
        ensure!(
            end <= manifest.payload.artifact_bytes,
            "transfer bytes exceed signed artifact length"
        );
        let path = directory(&self.directory, id).join("artifact.json");
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let length = file.metadata()?.len();
        ensure!(offset <= length, "transfer chunk gap");
        if offset < length {
            ensure!(end <= length, "overlapping transfer chunk");
            file.seek(std::io::SeekFrom::Start(offset))?;
            let mut previous = vec![0; bytes.len()];
            file.read_exact(&mut previous)?;
            ensure!(previous == bytes, "transfer chunk payload conflict");
        } else {
            file.seek(std::io::SeekFrom::End(0))?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Ok(
            serde_json::json!({"transfer_id":id,"next_offset":end,"stored_bytes":file.metadata()?.len()}),
        )
    }
    pub(super) async fn activate_transfer(
        &self,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        let VesselCommand::ActivateTransfer {
            command_id,
            transfer_id,
        } = &command
        else {
            anyhow::bail!("not activation")
        };
        let mut prepared = load(&self.directory, *transfer_id)?;
        let manifest = prepared
            .manifest
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("transfer manifest not accepted"))?;
        identity::verify(&self.directory, manifest.payload.source_vessel_id, manifest)?;
        let artifact_path = directory(&self.directory, *transfer_id).join("artifact.json");
        let metadata = std::fs::symlink_metadata(&artifact_path)?;
        ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() == manifest.payload.artifact_bytes,
            "transfer artifact incomplete"
        );
        let bytes = std::fs::read(&artifact_path)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == manifest.payload.artifact_sha256,
            "transfer artifact digest mismatch"
        );
        let initialization = RuntimeInitialization::Transfer {
            transfer_id: *transfer_id,
            artifact_path,
            sha256: manifest.payload.artifact_sha256.clone(),
            prepare_digest: manifest.payload.prepare_digest.clone(),
            generation: manifest.payload.generation,
        };
        let transfer_id = *transfer_id;
        let result = self
            .start_initialized(
                *command_id,
                manifest.payload.session_id,
                prepared.preparation.payload.workspace.clone(),
                prepared.config_path.clone(),
                Some(initialization),
                command,
            )
            .await?;
        prepared.activated = true;
        save(&self.directory, transfer_id, &prepared)?;
        Ok(result)
    }
}
