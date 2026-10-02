use super::*;
use crate::process::identity;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    io::{Read, Seek, Write},
    os::unix::fs::OpenOptionsExt,
};
impl Supervisor {
    pub(super) async fn accept_transfer(
        &self,
        manifest: SignedArtifact<TransferManifest>,
    ) -> Result<serde_json::Value> {
        let _serial = self.registrations.observe_existing().await?;
        let id = manifest.payload.transfer_id;
        let mut prepared = load(&self.directory, id)?;
        ensure!(
            prepared.catalogue_namespace == Some(_serial.namespace),
            "transfer catalogue witness unavailable or changed; no acceptance permitted"
        );
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
        let _serial = self.registrations.observe_existing().await?;
        let prepared = load(&self.directory, id)?;
        ensure!(
            prepared.catalogue_namespace == Some(_serial.namespace),
            "transfer catalogue witness unavailable or changed; no upload permitted"
        );
        ensure!(!prepared.activated, "transfer already activated");
        let manifest = prepared
            .manifest
            .ok_or_else(|| anyhow::anyhow!("transfer manifest not accepted"))?;
        ensure!(
            prepared.activation_command.is_none(),
            "transfer activation is pending; uploads refused"
        );
        ensure!(
            !_serial.contains_key(&manifest.payload.session_id),
            "transfer target already admitted; uploads refused"
        );
        let admission = super::super::database::transfer_admission_observation(
            &self.directory,
            id,
            manifest.payload.session_id,
            None,
            Some(_serial.namespace),
        )
        .await?;
        ensure!(
            !admission.any_activation && !admission.any_registration,
            "transfer activation admission prevents further uploads"
        );
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
        let registrations = self.registrations.observe_existing().await?;
        let mut prepared = load(&self.directory, *transfer_id)?;
        ensure!(
            prepared.catalogue_namespace == Some(registrations.namespace),
            "transfer catalogue witness unavailable or changed; no activation permitted"
        );
        let manifest = prepared
            .manifest
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("transfer manifest not accepted"))?;
        identity::verify(&self.directory, manifest.payload.source_vessel_id, manifest)?;
        let artifact_path = directory(&self.directory, *transfer_id).join("artifact.json");
        ensure!(
            super::status::artifact_bytes(&self.directory, *transfer_id, &manifest.payload)?
                == manifest.payload.artifact_bytes,
            "transfer artifact incomplete"
        );
        ensure!(
            prepared
                .activation_command
                .is_none_or(|id| id == *command_id),
            "transfer activation command changed"
        );
        let initialization = RuntimeInitialization::Transfer {
            transfer_id: *transfer_id,
            artifact_path,
            sha256: manifest.payload.artifact_sha256.clone(),
            prepare_digest: manifest.payload.prepare_digest.clone(),
            generation: manifest.payload.generation,
        };
        let transfer_id = *transfer_id;
        // Pin intent after all complete-artifact checks and before startup.
        // The per-transfer dispatcher lock prevents uploads/status racing this
        // intent; do not hold registrations across start_initialized.
        prepared.activation_command = Some(*command_id);
        save(&self.directory, transfer_id, &prepared)?;
        drop(registrations);
        let namespace = prepared
            .catalogue_namespace
            .ok_or_else(|| anyhow::anyhow!("transfer catalogue witness missing"))?;
        let result = super::super::database::in_transfer_namespace(
            &self.directory,
            namespace,
            self.start_initialized(
                *command_id,
                manifest.payload.session_id,
                prepared.preparation.payload.workspace.clone(),
                prepared.config_path.clone(),
                Some(initialization),
                command,
            ),
        )
        .await?;
        prepared.activated = true;
        save(&self.directory, transfer_id, &prepared)?;
        Ok(result)
    }
}
