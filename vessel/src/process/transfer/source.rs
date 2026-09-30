use super::*;
use crate::process::identity;
use base64::{Engine, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
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
        let source =
            super::super::runtime_storage::directory(&self.directory, &registration).await?;
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
                if self.inspect_registration(&registration).await.state == ProcessState::Stopped {
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
        let (artifact_sha256, artifact_bytes, prepare_digest, generation) = if registration
            .peer_uids
            .is_some()
        {
            #[cfg(target_os = "linux")]
            {
                let response=crate::process::identity_accounts::run_owned_helper(&self.directory,&self.binary,&crate::process::accounts::Scope::Owner,&registration.workspace,ProcessRight::History,voyage_protocol::identity_helper::IdentityHelperOperation::ObserveTransferArtifact{artifact_path:artifact.clone()},crate::process::identity_accounts::Selection::Bound(&registration)).await?;
                let voyage_protocol::identity_helper::IdentityHelperResponse::Value { value } =
                    response
                else {
                    anyhow::bail!("source transfer metadata unavailable");
                };
                ensure!(
                    value["session_id"] == serde_json::json!(session_id)
                        && value["transfer_id"] == serde_json::json!(prepared.transfer_id)
                        && value["destination_vessel_id"]
                            == serde_json::json!(prepared.destination_vessel_id)
                        && value["prepare_digest"] == identity::digest(preparation)?,
                    "portable checkpoint binding mismatch"
                );
                let digest = value["artifact_sha256"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("artifact digest missing"))?
                    .to_owned();
                let length = value["artifact_bytes"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("artifact length missing"))?;
                let generation = value["generation"]
                    .as_u64()
                    .ok_or_else(|| anyhow::anyhow!("artifact generation missing"))?;
                ensure!(
                    digest.len() == 64
                        && length > 0
                        && length <= 16 * 1024 * 1024
                        && generation > 0,
                    "invalid portable artifact metadata"
                );
                let facts_path = path.join("artifact-facts.json");
                if facts_path.exists() {
                    let prior: serde_json::Value = store::load(&facts_path)?;
                    ensure!(prior == value, "retained source artifact identity changed");
                } else {
                    store::save(&facts_path, &value)?;
                }
                (digest, length, identity::digest(preparation)?, generation)
            }
            #[cfg(not(target_os = "linux"))]
            anyhow::bail!("bound transfer unsupported");
        } else {
            let bytes = read_artifact(&artifact, unsafe { libc::geteuid() }, None)?;
            let checkpoint: PortableCheckpoint = serde_json::from_slice(&bytes)?;
            ensure!(
                checkpoint.session_id == *session_id
                    && checkpoint.transfer_id == prepared.transfer_id
                    && checkpoint.destination_vessel_id == prepared.destination_vessel_id
                    && checkpoint.prepare_digest == identity::digest(preparation)?
                    && checkpoint.generation > 0,
                "portable checkpoint binding mismatch"
            );
            (
                format!("{:x}", Sha256::digest(&bytes)),
                bytes.len() as u64,
                checkpoint.prepare_digest,
                checkpoint.generation,
            )
        };
        let manifest = identity::sign(
            &self.directory,
            TransferManifest {
                transfer_id: prepared.transfer_id,
                source_vessel_id: public.vessel_id,
                destination_vessel_id: prepared.destination_vessel_id,
                session_id: *session_id,
                source_incarnation: *incarnation,
                prepare_digest,
                artifact_sha256,
                artifact_bytes,
                generation,
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
        registry::save(&self.directory, &source, current).await?;
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
        let registration = self.registration(manifest.payload.session_id).await?;
        if registration.peer_uids.is_some() {
            super::super::database::bound_observer_identity(&self.directory, &registration).await?;
        }
        let uid = registration
            .peer_uids
            .as_ref()
            .map_or(unsafe { libc::geteuid() }, |peers| peers.runtime);
        // Forward bounded opaque signed bytes; never parse ordinary canonical records.
        let pin = if registration.peer_uids.is_some() {
            Some(store::load::<serde_json::Value>(
                &path.join("artifact-facts.json"),
            )?)
        } else {
            None
        };
        let bytes = read_artifact(&location, uid, pin.as_ref())?;
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
fn read_artifact(path: &Path, uid: u32, pin: Option<&serde_json::Value>) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut file = open_artifact(path, uid)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == uid
            && meta.nlink() == 1
            && meta.mode() & 0o077 == 0
            && meta.len() <= 16 * 1024 * 1024,
        "unsafe transfer artifact"
    );
    if let Some(pin) = pin {
        ensure!(
            pin["artifact_device"].as_u64() == Some(meta.dev())
                && pin["artifact_inode"].as_u64() == Some(meta.ino())
                && pin["artifact_uid"].as_u64() == Some(uid as u64)
                && pin["artifact_bytes"].as_u64() == Some(meta.len()),
            "retained transfer inode changed"
        );
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "transfer artifact grew beyond bound"
    );
    let after = file.metadata()?;
    ensure!(
        after.dev() == meta.dev()
            && after.ino() == meta.ino()
            && after.uid() == meta.uid()
            && after.mode() == meta.mode()
            && after.nlink() == 1
            && after.len() == meta.len()
            && bytes.len() as u64 == meta.len(),
        "transfer artifact metadata changed during read"
    );
    Ok(bytes)
}

fn open_artifact(path: &Path, uid: u32) -> Result<std::fs::File> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    ensure!(path.is_absolute(), "absolute transfer artifact required");
    let mut directory = std::fs::File::open("/")?;
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) => {
                let name = CString::new(value.as_bytes())?;
                let last = index + 1 == components.len();
                let flags = libc::O_RDONLY
                    | libc::O_NOFOLLOW
                    | libc::O_CLOEXEC
                    | libc::O_NONBLOCK
                    | if last { 0 } else { libc::O_DIRECTORY };
                let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
                ensure!(fd >= 0, "transfer artifact descriptor open refused");
                let file = unsafe { std::fs::File::from_raw_fd(fd) };
                if last {
                    return Ok(file);
                }
                let meta = file.metadata()?;
                ensure!(
                    meta.is_dir()
                        && (meta.uid() == 0 || meta.uid() == uid)
                        && ((meta.mode() & 0o022 == 0)
                            || (meta.uid() == 0 && meta.mode() & 0o1000 != 0)),
                    "foreign transfer artifact ancestor"
                );
                directory = file;
            }
            _ => anyhow::bail!("relative transfer artifact components refused"),
        }
    }
    anyhow::bail!("transfer artifact file missing")
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn opaque_signed_artifact_read_pins_inode_before_transport() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("artifact.json");
        std::fs::write(&path, b"opaque bytes").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let uid = unsafe { libc::geteuid() };
        let pin = serde_json::json!({"artifact_device":meta.dev(),"artifact_inode":meta.ino(),"artifact_uid":uid,"artifact_bytes":meta.len()});
        assert_eq!(
            read_artifact(&path, uid, Some(&pin)).unwrap(),
            b"opaque bytes"
        );
        let replacement = root.path().join("replacement");
        std::fs::write(&replacement, b"opaque bytes").unwrap();
        std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert!(
            read_artifact(&path, uid, Some(&pin)).is_err(),
            "same bytes cannot replace original owned inode"
        );
    }
    #[test]
    fn opaque_read_refuses_links_shared_modes_and_ancestor_redirection() {
        let root = tempfile::tempdir().unwrap();
        let original = root.path().join("source");
        std::fs::create_dir(&original).unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o700)).unwrap();
        let file = original.join("artifact.json");
        std::fs::write(&file, b"opaque").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        let uid = unsafe { libc::geteuid() };
        let redirected = root.path().join("alias");
        std::os::unix::fs::symlink(&original, &redirected).unwrap();
        assert!(read_artifact(&redirected.join("artifact.json"), uid, None).is_err());
        let link = original.join("linked");
        std::fs::hard_link(&file, &link).unwrap();
        assert!(read_artifact(&file, uid, None).is_err());
        std::fs::remove_file(link).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_artifact(&file, uid, None).is_err());
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(read_artifact(&file, uid.wrapping_add(1), None).is_err());
    }
}
