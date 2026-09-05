//! Explicit, local transfer foundation; deliberately not wired to a frontend.
//! Callers must establish operator consent and stop all legacy journal processes.
use crate::{completion::runtime::Coordinator, session::SessionStore};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct TransferRequest {
    pub transfer_id: Uuid,
    pub session_id: Uuid,
    pub expected_revision: u64,
    pub source_sha256: String,
}

#[derive(Debug)]
pub struct TransferReceipt {
    pub transfer_id: Uuid,
    pub session_id: Uuid,
    pub journal_revision: u64,
    pub duplicate: bool,
}

/// Internal immutable evidence, never a remote DTO. Paths stay local.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Provenance {
    pub transfer_id: Uuid,
    pub session_id: Uuid,
    pub source_revision: u64,
    pub source_sha256: String,
    pub source: PathBuf,
    pub destination: PathBuf,
    pub backup: PathBuf,
    pub workspace: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    format: String,
    version: u32,
    provenance: Provenance,
}

pub async fn transfer(
    store: SessionStore,
    coordinator: Coordinator,
    journal_directory: PathBuf,
    request: TransferRequest,
    cancel: CancellationToken,
) -> Result<TransferReceipt> {
    #[cfg(unix)]
    {
        transfer_inner(
            store,
            coordinator,
            journal_directory,
            request,
            cancel,
            Boundary::None,
        )
        .await
    }
    #[cfg(not(unix))]
    {
        let _ = (store, coordinator, journal_directory, request, cancel);
        anyhow::bail!(
            "session transfer unsupported on this platform: durable private publication required"
        )
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Boundary {
    None,
    BackupPublished,
    BackupDurable,
    MarkerPublished,
    MarkerDurable,
    BeforeCommit,
    AfterCommit,
}

#[cfg(unix)]
async fn transfer_inner(
    store: SessionStore,
    coordinator: Coordinator,
    directory: PathBuf,
    request: TransferRequest,
    cancel: CancellationToken,
    fault: Boundary,
) -> Result<TransferReceipt> {
    anyhow::ensure!(
        !request.transfer_id.is_nil() && !request.session_id.is_nil(),
        "nil transfer identity"
    );
    anyhow::ensure!(
        request.source_sha256.len() == 64
            && request
                .source_sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "invalid source hash"
    );
    let child = cancel.child_token();
    let _cancel_on_drop = child.clone().drop_guard();
    let owner = tokio::select! {
        biased;
        _ = child.cancelled() => anyhow::bail!("transfer cancelled"),
        owner = store.with_execution(request.session_id) => owner?,
    };
    tokio::task::spawn_blocking(move || {
        unix::execute(owner, coordinator, directory, request, child, fault)
    })
    .await?
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::{
        attachment::journal::Journal,
        session::{Session, reject_symlinks},
    };
    use anyhow::{Context, ensure};
    use sha2::{Digest, Sha256};
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        path::Path,
    };

    const MAX_SOURCE: u64 = 16 * 1024 * 1024;
    fn private(path: &Path, directory: bool) -> Result<()> {
        reject_symlinks(path)?;
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "transfer path must be private and owned by current user"
        );
        ensure!(
            if directory {
                metadata.is_dir()
            } else {
                metadata.is_file() && metadata.nlink() == 1
            },
            "invalid transfer path type or hard link"
        );
        Ok(())
    }
    fn read(path: &Path) -> Result<Vec<u8>> {
        private(path, false)?;
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file() && metadata.nlink() == 1 && metadata.len() <= MAX_SOURCE,
            "invalid transfer file or capacity exceeded"
        );
        let mut bytes = Vec::new();
        file.take(MAX_SOURCE + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SOURCE,
            "transfer source capacity exceeded"
        );
        Ok(bytes)
    }
    fn hash(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }
    fn boundary(fault: Boundary, here: Boundary, cancel: &CancellationToken) -> Result<()> {
        ensure!(fault != here, "injected transfer interruption");
        ensure!(!cancel.is_cancelled(), "transfer cancelled");
        Ok(())
    }
    pub(super) fn execute(
        owner: SessionStore,
        coordinator: Coordinator,
        directory: PathBuf,
        request: TransferRequest,
        cancel: CancellationToken,
        fault: Boundary,
    ) -> Result<TransferReceipt> {
        ensure!(!cancel.is_cancelled(), "transfer cancelled");
        let source = owner.transfer_source_path(request.session_id)?;
        let source_directory = source.parent().context("missing source parent")?;
        private(source_directory, true)?;
        let source_bytes = read(&source)?;
        let existing_marker = serde_json::from_slice::<Marker>(&source_bytes).ok();
        let original = if let Some(marker) = &existing_marker {
            ensure!(
                marker.format == "helm.session-transfer" && marker.version == 1,
                "unsupported migration marker"
            );
            // Resolve only a deterministic path, never trust a marker-provided backup path.
            let backup = directory
                .join("imports")
                .join(format!("{}.json", request.transfer_id));
            read(&backup)?
        } else {
            source_bytes.clone()
        };
        ensure!(
            hash(&original) == request.source_sha256,
            "source hash mismatch"
        );
        let session: Session =
            serde_json::from_slice(&original).context("invalid original session")?;
        ensure!(
            session.id == request.session_id && session.revision == request.expected_revision,
            "source identity or revision mismatch"
        );
        ensure!(
            !session
                .messages
                .iter()
                .any(|m| m.role == crate::model::Role::System),
            "legacy runtime system messages require explicit canonical cleanup before transfer"
        );
        let workspace = session.workspace.canonicalize()?;
        let data = source_directory.parent().context("missing data parent")?;
        let key = hash(session.workspace.as_os_str().as_encoded_bytes());
        let expected_coordinator = data.join("completion").join(key);
        let (coordinator_directory, coordinator_workspace) = coordinator.transfer_identity();
        private(&expected_coordinator, true)?;
        ensure!(
            expected_coordinator.canonicalize()? == coordinator_directory
                && workspace == coordinator_workspace,
            "incorrect workspace execution fence"
        );
        let _workspace_owner = coordinator.acquire_agent_writer()?;
        ensure!(!cancel.is_cancelled(), "transfer cancelled");
        // Validate before Journal::open can create anything. Its parent must be private.
        let parent = directory.parent().context("missing journal parent")?;
        private(parent, true)?;
        reject_symlinks(&directory)?;
        let mut journal = Journal::open(directory.clone())?;
        let directory = directory.canonicalize()?;
        private(&directory, true)?;
        private(&directory.join("journal.sqlite3"), false)?;
        let imports = directory.join("imports");
        match fs::DirBuilder::new().mode(0o700).create(&imports) {
            Ok(()) => File::open(&directory)?.sync_all()?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        private(&imports, true)?;
        let backup = imports.join(format!("{}.json", request.transfer_id));
        let provenance = Provenance {
            transfer_id: request.transfer_id,
            session_id: session.id,
            source_revision: session.revision,
            source_sha256: request.source_sha256,
            source: source.clone(),
            destination: directory,
            backup: backup.clone(),
            workspace,
        };
        let existing = journal.preflight_import(&provenance)?;
        if let Some(marker) = existing_marker {
            ensure!(
                marker.provenance == provenance,
                "migration marker provenance mismatch"
            );
            ensure!(read(&backup)? == original, "backup changed");
            File::open(&imports)?.sync_all()?;
            File::open(source_directory)?.sync_all()?;
        } else {
            ensure!(
                existing.is_none(),
                "active JSON conflicts with completed journal authority"
            );
            if backup.exists() {
                ensure!(read(&backup)? == original, "backup collision");
            } else {
                let mut temporary = tempfile::NamedTempFile::new_in(&imports)?;
                temporary.write_all(&original)?;
                temporary.as_file().sync_all()?;
                temporary.persist_noclobber(&backup)?;
            }
            boundary(fault, Boundary::BackupPublished, &cancel)?;
            File::open(&imports)?.sync_all()?;
            boundary(fault, Boundary::BackupDurable, &cancel)?;
            let marker = Marker {
                format: "helm.session-transfer".into(),
                version: 1,
                provenance: provenance.clone(),
            };
            let mut temporary = tempfile::NamedTempFile::new_in(source_directory)?;
            temporary.write_all(&serde_json::to_vec(&marker)?)?;
            temporary.as_file().sync_all()?;
            ensure!(
                read(&source)? == source_bytes,
                "source changed under execution fence"
            );
            temporary.persist(&source)?;
            boundary(fault, Boundary::MarkerPublished, &cancel)?;
            File::open(source_directory)?.sync_all()?;
        }
        boundary(fault, Boundary::MarkerDurable, &cancel)?;
        boundary(fault, Boundary::BeforeCommit, &cancel)?;
        let (journal_revision, duplicate) = journal.import_session(&session, &provenance)?;
        boundary(fault, Boundary::AfterCommit, &cancel)?;
        Ok(TransferReceipt {
            transfer_id: provenance.transfer_id,
            session_id: session.id,
            journal_revision,
            duplicate,
        })
    }
}

#[cfg(test)]
mod tests;
