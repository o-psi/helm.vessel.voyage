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
    let destination = Client {
        directory: args.destination_directory.clone(),
        ssh: args.destination_ssh.clone(),
        access_file: None,
    };
    let source_id: VesselIdentity =
        serde_json::from_value(source.request(VesselCommand::Identity).await?)?;
    let destination_id: VesselIdentity =
        serde_json::from_value(destination.request(VesselCommand::Identity).await?)?;
    ensure!(
        source_id.vessel_id != destination_id.vessel_id,
        "source and destination must be distinct Vessels"
    );
    let mut journal = if args.journal.exists() {
        check_file(&args.journal)?;
        let saved: Journal = read_json(&args.journal)?;
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
    if let Some(result) = &journal.result {
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
    destination
        .request(VesselCommand::AcceptTransfer {
            manifest: manifest.clone(),
        })
        .await?;
    let activate = VesselCommand::ActivateTransfer {
        command_id: journal.activate_command,
        transfer_id: journal.transfer_id,
    };
    // A completed activation can precede the courier's final local checkpoint.
    if let Ok(result) = destination.request(activate.clone()).await {
        journal.result = Some(result.clone());
        save(&args.journal, &journal)?;
        return Ok(result);
    }
    let mut offset = 0u64;
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
        destination
            .request(VesselCommand::UploadTransferChunk {
                transfer_id: journal.transfer_id,
                offset,
                data,
            })
            .await?;
        offset = next;
    }
    let result = destination.request(activate).await?;
    journal.result = Some(result.clone());
    save(&args.journal, &journal)?;
    Ok(result)
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
