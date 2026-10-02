//! Readonly proof for the original ordinary transfer activation. There is no
//! timeout takeover, resource cleanup claim, startup or receipt reservation.
use super::*;
use crate::process::{database, identity};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
};

pub(super) fn artifact_bytes(root: &Path, id: Uuid, manifest: &TransferManifest) -> Result<u64> {
    let path = directory(root, id).join("artifact.json");
    let leaf = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        leaf.is_file() && !leaf.file_type().is_symlink(),
        "unsafe transfer artifact leaf"
    );
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) => return Err(error.into()),
    };
    let before = file.metadata()?;
    ensure!(
        before.is_file()
            && before.nlink() == 1
            && before.uid() == unsafe { libc::geteuid() }
            && before.mode() & 0o077 == 0
            && before.len() <= manifest.artifact_bytes
            && manifest.artifact_bytes <= 16 * 1024 * 1024,
        "unsafe transfer artifact observation"
    );
    ensure!(
        (leaf.dev(), leaf.ino()) == (before.dev(), before.ino()),
        "transfer artifact leaf changed before open"
    );
    if before.len() == manifest.artifact_bytes {
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        let mut count = 0u64;
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            count = count
                .checked_add(read as u64)
                .ok_or_else(|| anyhow::anyhow!("transfer artifact length overflow"))?;
            ensure!(
                count <= manifest.artifact_bytes,
                "transfer artifact grew during observation"
            );
            hash.update(&buffer[..read]);
        }
        ensure!(
            count == manifest.artifact_bytes
                && format!("{:x}", hash.finalize()) == manifest.artifact_sha256,
            "transfer artifact digest mismatch"
        );
    }
    let after = file.metadata()?;
    let current = std::fs::symlink_metadata(&path)?;
    ensure!(
        !current.file_type().is_symlink()
            && (current.dev(), current.ino()) == (before.dev(), before.ino())
            && (
                before.dev(),
                before.ino(),
                before.len(),
                before.uid(),
                before.mode(),
                before.nlink(),
                before.mtime(),
                before.mtime_nsec(),
                before.ctime(),
                before.ctime_nsec()
            ) == (
                after.dev(),
                after.ino(),
                after.len(),
                after.uid(),
                after.mode(),
                after.nlink(),
                after.mtime(),
                after.mtime_nsec(),
                after.ctime(),
                after.ctime_nsec()
            ),
        "transfer artifact changed during observation"
    );
    Ok(before.len())
}

fn matches_registration(
    root: &Path,
    registration: &ProcessRegistration,
    prepared: &Prepared,
    manifest: &TransferManifest,
    command: Uuid,
) -> bool {
    registration.peer_uids.is_none()
        && registration.session_id == manifest.session_id
        && registration.command_id == command
        && registration.workspace == prepared.preparation.payload.workspace
        && registration.config_path == prepared.config_path
        && matches!(&registration.initialize,Some(RuntimeInitialization::Transfer{transfer_id,artifact_path,sha256,prepare_digest,generation})
            if *transfer_id==manifest.transfer_id && sha256==&manifest.artifact_sha256
                && artifact_path==&directory(root,manifest.transfer_id).join("artifact.json")
                && prepare_digest==&manifest.prepare_digest && *generation==manifest.generation)
}

impl Supervisor {
    /// Caller holds the same per-transfer mutex as accept/upload/activation.
    pub(super) async fn transfer_status(
        &self,
        id: Uuid,
        command: Uuid,
        digest: String,
        manifest: SignedArtifact<TransferManifest>,
    ) -> Result<TransferStatus> {
        ensure!(
            !super::super::runtime_storage::has_bound_layout(&self.directory),
            "ordinary transfer status unavailable for bound control layout"
        );
        ensure!(
            !id.is_nil()
                && !command.is_nil()
                && digest.len() == 64
                && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid transfer status identity"
        );
        let registrations = self.registrations.observe_existing().await?;
        let prepared = load(&self.directory, id)?;
        ensure!(
            prepared
                .catalogue_namespace
                .is_none_or(|namespace| namespace == registrations.namespace),
            "original transfer catalogue namespace changed; evidence retained"
        );
        ensure!(
            prepared
                .manifest
                .as_ref()
                .is_none_or(|stored| identity::digest(stored).is_ok_and(|stored| stored == digest)),
            "stored transfer manifest changed"
        );
        let payload = &manifest.payload;
        let preparation = &prepared.preparation.payload;
        let local = identity::existing_public(&self.directory)?;
        let destination = local.vessel_id;
        identity::verify_public(&local, &prepared.preparation)?;
        identity::verify(&self.directory, payload.source_vessel_id, &manifest)?;
        ensure!(
            identity::digest(&manifest)? == digest
                && payload.transfer_id == id
                && preparation.transfer_id == id
                && payload.destination_vessel_id == destination
                && preparation.destination_vessel_id == destination
                && payload.source_vessel_id == preparation.source_vessel_id
                && payload.session_id == preparation.session_id
                && payload.prepare_digest == identity::digest(&prepared.preparation)?
                && payload.artifact_bytes > 0
                && payload.artifact_bytes <= 16 * 1024 * 1024
                && payload.artifact_sha256.len() == 64
                && payload.generation > 0,
            "transfer status manifest binding changed"
        );
        ensure!(
            prepared.activation_command.is_none_or(|id| id == command),
            "transfer activation identity changed"
        );
        let original = VesselCommand::ActivateTransfer {
            command_id: command,
            transfer_id: id,
        };
        let observed = database::transfer_admission_observation(
            &self.directory,
            id,
            payload.session_id,
            Some((command, serde_json::to_vec(&original)?)),
            Some(registrations.namespace),
        )
        .await?;
        ensure!(
            observed
                .registration
                .as_ref()
                .is_none_or(|registration| registration.session_id == payload.session_id
                    && registration.peer_uids.is_none()),
            "transfer target registration is incompatible"
        );
        let mut status = TransferStatus {
            transfer_id: id,
            activate_command_id: command,
            manifest_digest: digest,
            source_vessel_id: payload.source_vessel_id,
            destination_vessel_id: destination,
            session_id: payload.session_id,
            artifact_sha256: payload.artifact_sha256.clone(),
            artifact_bytes: payload.artifact_bytes,
            received_bytes: None,
            state: TransferStatusState::Pending,
            completion: None,
        };
        if let Some(completion) = observed.completion {
            ensure!(
                observed.exact_admission
                    && completion.session_id == payload.session_id
                    && observed
                        .completed_registration
                        .as_ref()
                        .is_some_and(|registration| registration.incarnation
                            == completion.incarnation
                            && matches_registration(
                                &self.directory,
                                registration,
                                &prepared,
                                payload,
                                command
                            )),
                "transfer completion identity mismatch"
            );
            status.state = TransferStatusState::Complete;
            status.completion = Some(TransferCompletion {
                session_id: completion.session_id,
                incarnation: completion.incarnation,
            });
        } else if !observed.exact_admission
            && !observed.any_activation
            && prepared.catalogue_namespace == Some(registrations.namespace)
            && !observed.any_registration
            && !registrations.contains_key(&payload.session_id)
            && prepared.activation_command.is_none()
            && !prepared.activated
        {
            status.received_bytes = Some(artifact_bytes(&self.directory, id, payload)?);
            status.state = TransferStatusState::Receiving;
        }
        Ok(status)
    }
}
