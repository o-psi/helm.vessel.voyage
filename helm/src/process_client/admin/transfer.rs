//! A resumable courier for signed ownership artifacts; it never grants authority.
use super::{args::MoveArgs, read_json};
use crate::process_client::{local, transport::Client};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::vessel::*;

#[derive(Serialize, Deserialize)]
struct Journal {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    phase: Option<Phase>,
    args: MoveArgs,
    source: VesselIdentity,
    destination: VesselIdentity,
    transfer_id: Uuid,
    prepare_command: Uuid,
    export_command: Uuid,
    activate_command: Uuid,
    expires_at_ms: u64,
    preparation: Option<SignedArtifact<TransferPreparation>>,
    manifest: Option<SignedArtifact<TransferManifest>>,
    result: Option<Value>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Receiving,
    ActivationPending,
}

pub(super) async fn run(source: &Client, args: MoveArgs) -> Result<Value> {
    ensure!(
        source.access_file.is_none(),
        "ownership administration requires explicit account authority"
    );
    ensure!(
        args.journal.is_absolute()
            && args.destination_directory.is_absolute()
            && args.workspace.is_absolute(),
        "courier journal and destination paths must be absolute"
    );
    let parent = args
        .journal
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal parent missing"))?;
    local::check_private_directory(parent)?;
    let _lock = lock(&args.journal)?;
    // Read before contacting either endpoint: a retired remote route must never
    // be reinterpreted as a local destination, even when paths happen to match.
    let saved = match std::fs::symlink_metadata(&args.journal) {
        Ok(_) => {
            check_file(&args.journal)?;
            let value: Value = read_json(&args.journal)?;
            ensure!(
                value
                    .get("args")
                    .and_then(|v| v.get("destination_ssh"))
                    .is_none_or(Value::is_null),
                "SSH courier journals are no longer supported; original journal preserved"
            );
            Some(serde_json::from_value::<Journal>(value)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if let Some(saved) = &saved {
        ensure!(
            saved.version <= 1 && (saved.version == 0 || saved.phase.is_some()),
            "unsupported courier journal version; original retained"
        );
        ensure!(
            saved.phase != Some(Phase::ActivationPending) || saved.manifest.is_some(),
            "pending activation has no retained manifest"
        );
        ensure!(
            saved.version != 0 || saved.result.is_none() || saved.manifest.is_some(),
            "legacy result lacks a retained manifest; original evidence preserved"
        );
    }
    let recovering_manifest = saved.as_ref().is_some_and(|saved| saved.manifest.is_some());
    let destination = Client::local(args.destination_directory.clone());
    let source_id: VesselIdentity =
        serde_json::from_value(source.request(VesselCommand::Identity).await?)?;
    let destination_id: VesselIdentity =
        serde_json::from_value(destination.request(VesselCommand::Identity).await?)?;
    ensure!(
        source_id.vessel_id != destination_id.vessel_id,
        "source and destination must be distinct Vessels"
    );
    let mut journal = if let Some(saved) = saved {
        ensure!(
            saved.args == args
                && serde_json::to_vec(&saved.source)? == serde_json::to_vec(&source_id)?
                && serde_json::to_vec(&saved.destination)? == serde_json::to_vec(&destination_id)?,
            "courier operation or pinned endpoint identity changed"
        );
        saved
    } else {
        let now: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .try_into()?;
        let saved = Journal {
            version: 1,
            phase: Some(Phase::Receiving),
            args: args.clone(),
            source: source_id,
            destination: destination_id,
            transfer_id: Uuid::new_v4(),
            prepare_command: Uuid::new_v4(),
            export_command: Uuid::new_v4(),
            activate_command: Uuid::new_v4(),
            expires_at_ms: now.saturating_add(300000),
            preparation: None,
            manifest: None,
            result: None,
        };
        save(&args.journal, &saved)?;
        saved
    };
    if let Some(result) = &journal.result
        && journal.version == 1
    {
        ensure!(
            journal.phase == Some(Phase::ActivationPending),
            "cached courier result has no activation checkpoint"
        );
        return Ok(result.clone());
    }
    if journal.preparation.is_none() {
        let preparation = destination
            .request(VesselCommand::PrepareTransfer {
                command_id: journal.prepare_command,
                transfer_id: journal.transfer_id,
                source_vessel_id: journal.source.vessel_id,
                session_id: args.session,
                workspace: args.workspace.clone(),
                config_path: args.config_path.clone(),
                expires_at_ms: journal.expires_at_ms,
            })
            .await?;
        journal.preparation = Some(serde_json::from_value(preparation)?);
        save(&args.journal, &journal)?;
    }
    if journal.manifest.is_none() {
        eprintln!(
            "Relinquishing source owner for transfer {}. The private courier journal retains exact recovery identities.",
            journal.transfer_id
        );
        let manifest = source
            .request(VesselCommand::ExportTransfer {
                command_id: journal.export_command,
                session_id: args.session,
                incarnation: args.incarnation,
                expected_revision: args.expected_revision,
                expires_at_ms: journal.expires_at_ms,
                preparation: journal
                    .preparation
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("preparation missing"))?,
            })
            .await?;
        journal.manifest = Some(serde_json::from_value(manifest)?);
        save(&args.journal, &journal)?;
    }
    let manifest = journal
        .manifest
        .clone()
        .ok_or_else(|| anyhow::anyhow!("manifest missing"))?;
    let mut artifact_ready = false;
    if recovering_manifest || journal.phase == Some(Phase::ActivationPending) {
        let status = status(&destination, &journal, &manifest).await?;
        match status.state {
            TransferStatusState::Pending => anyhow::bail!(
                "Transfer activation remains pending; original IDs retained, no upload or activation replay"
            ),
            TransferStatusState::Complete => {
                let completion = status
                    .completion
                    .ok_or_else(|| anyhow::anyhow!("transfer completion missing"))?;
                let result = serde_json::json!({"transfer_id":journal.transfer_id,
                    "activate_command_id":journal.activate_command,"status":"complete",
                    "session_id":completion.session_id,"incarnation":completion.incarnation});
                journal.result = Some(result.clone());
                save(&args.journal, &journal)?;
                return Ok(result);
            }
            TransferStatusState::Receiving => {
                ensure!(
                    journal.result.is_none(),
                    "legacy cached result conflicts with receiving proof; original retained without effects"
                );
                artifact_ready = status.received_bytes == Some(manifest.payload.artifact_bytes);
                journal.version = 1;
                journal.phase = Some(Phase::Receiving);
                save(&args.journal, &journal)?;
            }
        }
    } else {
        journal.version = 1;
        journal.phase = Some(Phase::Receiving);
        save(&args.journal, &journal)?;
    }
    destination
        .request(VesselCommand::AcceptTransfer {
            manifest: manifest.clone(),
        })
        .await?;
    let activate = VesselCommand::ActivateTransfer {
        command_id: journal.activate_command,
        transfer_id: journal.transfer_id,
    };
    // Never probe activation before uploading: even an artifact precondition
    // error is conservatively unknown on older Vessels. Recovery above uses an
    // exact readonly proof, not a mutative request or an absence guess.
    let mut offset = if artifact_ready {
        manifest.payload.artifact_bytes
    } else {
        0
    };
    while offset < manifest.payload.artifact_bytes {
        let chunk = source
            .request(VesselCommand::TransferChunk {
                transfer_id: journal.transfer_id,
                offset,
                limit: 65536,
            })
            .await?;
        ensure!(
            chunk["transfer_id"] == journal.transfer_id.to_string()
                && chunk["offset"] == offset
                && chunk["total_bytes"] == manifest.payload.artifact_bytes,
            "courier chunk identity mismatch"
        );
        let next = chunk["next_offset"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("chunk boundary missing"))?;
        ensure!(
            next > offset && next <= manifest.payload.artifact_bytes && next - offset <= 65536,
            "invalid courier chunk boundary"
        );
        let data = chunk["data"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("chunk data missing"))?
            .to_owned();
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        ensure!(
            data.len() <= 90000,
            "courier chunk data exceeds encoded bound"
        );
        let decoded = STANDARD
            .decode(&data)
            .map_err(|_| anyhow::anyhow!("invalid courier chunk data"))?;
        ensure!(
            decoded.len() as u64 == next - offset && decoded.len() <= 65536,
            "courier chunk data does not match its boundary"
        );
        let acknowledgement = destination
            .request(VesselCommand::UploadTransferChunk {
                transfer_id: journal.transfer_id,
                offset,
                data,
            })
            .await?;
        ensure!(
            acknowledgement["transfer_id"] == journal.transfer_id.to_string()
                && acknowledgement["next_offset"] == next
                && acknowledgement["stored_bytes"]
                    .as_u64()
                    .is_some_and(|bytes| bytes >= next && bytes <= manifest.payload.artifact_bytes),
            "courier upload acknowledgement changed; original operation retained"
        );
        offset = next;
    }
    journal.version = 1;
    journal.phase = Some(Phase::ActivationPending);
    save(&args.journal, &journal)?;
    let result = destination.request(activate).await?;
    journal.result = Some(result.clone());
    save(&args.journal, &journal)?;
    Ok(result)
}

async fn status(
    destination: &Client,
    journal: &Journal,
    manifest: &SignedArtifact<TransferManifest>,
) -> Result<TransferStatus> {
    use sha2::{Digest, Sha256};
    let capabilities = destination.request(VesselCommand::Capabilities).await?;
    ensure!(
        capabilities["features"]
            .as_array()
            .is_some_and(|features| features
                .iter()
                .any(|feature| feature == "signed_transfer_status_v1")),
        "Destination upgrade required for readonly transfer recovery; original journal and uncertainty retained"
    );
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(manifest)?));
    let status: TransferStatus = serde_json::from_value(
        destination
            .request(VesselCommand::TransferStatus {
                transfer_id: journal.transfer_id,
                activate_command_id: journal.activate_command,
                expected_manifest_digest: digest.clone(),
                manifest: manifest.clone(),
            })
            .await?,
    )?;
    ensure!(
        status.transfer_id == journal.transfer_id
            && status.activate_command_id == journal.activate_command
            && status.manifest_digest == digest
            && status.source_vessel_id == journal.source.vessel_id
            && status.destination_vessel_id == journal.destination.vessel_id
            && status.session_id == journal.args.session
            && status.artifact_sha256 == manifest.payload.artifact_sha256
            && status.artifact_bytes == manifest.payload.artifact_bytes,
        "transfer recovery binding changed"
    );
    match status.state {
        TransferStatusState::Receiving => ensure!(
            status.completion.is_none()
                && status
                    .received_bytes
                    .is_some_and(|bytes| bytes <= status.artifact_bytes),
            "invalid receiving transfer proof"
        ),
        TransferStatusState::Pending => ensure!(
            status.completion.is_none() && status.received_bytes.is_none(),
            "invalid pending transfer proof"
        ),
        TransferStatusState::Complete => ensure!(
            status.received_bytes.is_none()
                && status
                    .completion
                    .as_ref()
                    .is_some_and(|receipt| receipt.session_id == journal.args.session
                        && !receipt.incarnation.is_nil()),
            "invalid transfer completion proof"
        ),
    }
    Ok(status)
}

fn save(path: &Path, journal: &Journal) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("journal parent missing"))?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    let bytes = serde_json::to_vec(journal)?;
    ensure!(bytes.len() <= 65536, "courier journal exceeds bound");
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
#[cfg(unix)]
fn check_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file()
            && !meta.file_type().is_symlink()
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "unsafe courier journal"
    );
    Ok(())
}
#[cfg(unix)]
fn lock(path: &Path) -> Result<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let path = path.with_extension("courier-lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    check_file(&path)?;
    ensure!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
        "courier operation already running"
    );
    Ok(file)
}
#[cfg(not(unix))]
fn check_file(_path: &Path) -> Result<()> {
    anyhow::bail!("private courier journal unsupported on this platform")
}
#[cfg(not(unix))]
fn lock(_path: &Path) -> Result<std::fs::File> {
    anyhow::bail!("private courier lock unsupported on this platform")
}

#[cfg(all(test, unix))]
#[path = "courier_journey_tests.rs"]
mod journey_tests;
