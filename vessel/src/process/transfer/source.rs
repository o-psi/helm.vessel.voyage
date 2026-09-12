use super::*;
use crate::process::{identity, routing};
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
impl Supervisor {
    pub(super) async fn export_transfer(
        &self,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        let VesselCommand::ExportTransfer {
            command_id,
            session_id,
            incarnation,
            expected_revision,
            expires_at_ms,
            preparation,
        } = &command
        else {
            anyhow::bail!("not transfer export")
        };
        let transfer_lock = self
            .assignment_lock(preparation.payload.transfer_id)
            .await?;
        let _transfer = transfer_lock.lock().await;
        let public = identity::public(&self.directory)?;
        let prepared = &preparation.payload;
        ensure!(
            prepared.source_vessel_id == public.vessel_id && prepared.session_id == *session_id,
            "source preparation binding mismatch"
        );
        identity::verify(&self.directory, prepared.destination_vessel_id, preparation)?;
        let path = directory(&self.directory, prepared.transfer_id);
        registry::private_directory(&self.directory.join("transfers"))?;
        registry::private_directory(&path)?;
        let recorded =
            registry::command_record(&self.directory, *command_id, &command, false).await?;
        if recorded && path.join("export.json").exists() {
            let manifest: SignedArtifact<TransferManifest> =
                store::load(&path.join("export.json"))?;
            return Ok(serde_json::to_value(manifest)?);
        }
        let mut registration = self.registration(*session_id).await?;
        ensure!(
            registration.incarnation == *incarnation,
            "stale source incarnation"
        );
        let source = registry::directory(&self.directory, *session_id);
        let artifact = source
            .join("transfers")
            .join(format!("{}.json", prepared.transfer_id));
        if !recorded {
            ensure!(
                prepared.expires_at_ms > store::now()?,
                "destination preparation expired before relinquishment"
            );
            registry::command_record(&self.directory, *command_id, &command, true).await?;
        }
        if !artifact.exists() {
            let response = self
                .forward_resuming(
                    *session_id,
                    registration.incarnation,
                    RuntimeCommand::Relinquish {
                        command_id: *command_id,
                        expected_revision: *expected_revision,
                        expires_at_ms: *expires_at_ms,
                        transfer_id: prepared.transfer_id,
                        destination_vessel_id: prepared.destination_vessel_id,
                        prepare_digest: identity::digest(preparation)?,
                    },
                    None,
                )
                .await?;
            ensure!(
                response.error.is_none(),
                "source relinquishment refused: {}",
                response.error.unwrap_or_default()
            );
        }
        registration = self.registration(*session_id).await?;
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if routing::inspect(&source, &registration).await.state == ProcessState::Stopped {
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap_or(false);
        ensure!(
            stopped,
            "source cleanup not positively observed; transfer remains pending"
        );
        let bytes = read_artifact(&artifact)?;
        let checkpoint: PortableCheckpoint = serde_json::from_slice(&bytes)?;
        ensure!(
            checkpoint.session_id == *session_id
                && checkpoint.transfer_id == prepared.transfer_id
                && checkpoint.destination_vessel_id == prepared.destination_vessel_id
                && checkpoint.prepare_digest == identity::digest(preparation)?
                && checkpoint.generation > 0,
            "portable checkpoint binding mismatch"
        );
        let manifest = identity::sign(
            &self.directory,
            TransferManifest {
                transfer_id: prepared.transfer_id,
                source_vessel_id: public.vessel_id,
                destination_vessel_id: prepared.destination_vessel_id,
                session_id: *session_id,
                source_incarnation: *incarnation,
                prepare_digest: checkpoint.prepare_digest,
                artifact_sha256: format!("{:x}", Sha256::digest(&bytes)),
                artifact_bytes: bytes.len() as u64,
                generation: checkpoint.generation,
            },
        )?;
        // Keep a source-local artifact location; no canonical content enters the supervision record.
        store::save(&path.join("artifact-location.json"), &artifact)?;
        store::save(&path.join("export.json"), &manifest)?;
        let mut registrations = self.registrations.lock().await?;
        let current = registrations
            .get_mut(session_id)
            .ok_or_else(|| anyhow::anyhow!("source registration lost"))?;
        ensure!(
            current.incarnation == *incarnation,
            "source incarnation changed during transfer"
        );
        current.state = ProcessState::Relinquished;
        registry::save(&source, current).await?;
        Ok(serde_json::to_value(manifest)?)
    }
    pub(super) async fn transfer_chunk(
        &self,
        id: Uuid,
        offset: u64,
        limit: u32,
    ) -> Result<serde_json::Value> {
        ensure!(
            (1..=65536).contains(&limit),
            "transfer chunk limit must be 1..65536"
        );
        let path = directory(&self.directory, id);
        let manifest: SignedArtifact<TransferManifest> = store::load(&path.join("export.json"))?;
        let location: PathBuf = store::load(&path.join("artifact-location.json"))?;
        let bytes = read_artifact(&location)?;
        ensure!(
            bytes.len() as u64 == manifest.payload.artifact_bytes
                && format!("{:x}", Sha256::digest(&bytes)) == manifest.payload.artifact_sha256,
            "export artifact changed"
        );
        let offset: usize = offset.try_into()?;
        ensure!(offset <= bytes.len(), "transfer offset beyond artifact");
        let end = offset.saturating_add(limit as usize).min(bytes.len());
        Ok(
            serde_json::json!({"transfer_id":id,"offset":offset,"next_offset":end,"total_bytes":bytes.len(),"data":STANDARD.encode(&bytes[offset..end]),"has_more":end<bytes.len()}),
        )
    }
}
fn read_artifact(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= 16 * 1024 * 1024,
        "unsafe transfer artifact"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
