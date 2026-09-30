//! Protected control coordinates are explicit; runtime paths never locate authority.
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};
use voyage_protocol::process::ProcessRegistration;

pub(super) fn has_bound_layout(root: &Path) -> bool {
    match std::fs::symlink_metadata(root.join("runtime-layout.json")) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn validate_bound_layout(root: &Path) -> Result<()> {
    let _ = runtime_root(root)?;
    Ok(())
}

/// Legacy user scope keeps its original location. Bound observations use only
/// administrator-provisioned layout metadata, never runtime-supplied paths.
pub(super) async fn directory(root: &Path, registration: &ProcessRegistration) -> Result<PathBuf> {
    if registration.peer_uids.is_none() {
        return Ok(super::registry::directory(root, registration.session_id));
    }
    #[cfg(target_os = "linux")]
    {
        let identity = super::database::bound_observer_identity(root, registration).await?;
        super::launch::validate_identity(&identity)?;
        bound_directory(root, registration.session_id, identity.uid, identity.gid)
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("bound runtime storage is unsupported on this host")
}

#[cfg(target_os = "linux")]
pub(super) fn bound_directory(
    root: &Path,
    session: uuid::Uuid,
    uid: u32,
    gid: u32,
) -> Result<PathBuf> {
    let (runtime, path) = runtime_root(root)?;
    let name = session.to_string();
    let _directory = runtime.session(name.as_ref(), uid, gid)?;
    Ok(path.join(name))
}

#[cfg(target_os = "linux")]
pub(super) fn provision_bound_directory(
    root: &Path,
    session: uuid::Uuid,
    uid: u32,
    gid: u32,
) -> Result<PathBuf> {
    let (runtime, path) = runtime_root(root)?;
    let name = session.to_string();
    let _directory = runtime.create_session(name.as_ref(), uid, gid)?;
    Ok(path.join(name))
}

#[cfg(target_os = "linux")]
fn runtime_root(root: &Path) -> Result<(voyage_storage::protected_linux::RuntimeRoot, PathBuf)> {
    use voyage_storage::protected_linux::{RootDirectory, RuntimeRoot};
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Layout {
        version: u32,
        runtime_root: PathBuf,
    }
    let control = RootDirectory::open(root)?;
    let layout: Layout =
        serde_json::from_slice(&control.read("runtime-layout.json".as_ref(), 4096)?)?;
    ensure!(layout.version == 1, "unsupported runtime layout");
    ensure!(
        layout.runtime_root != root
            && !layout.runtime_root.starts_with(root)
            && !root.starts_with(&layout.runtime_root),
        "control and runtime roots must be separate"
    );
    let runtime = RuntimeRoot::open(&layout.runtime_root)?;
    Ok((runtime, layout.runtime_root))
}

/// Lexical location only. It validates the protected layout without creating a
/// session directory or treating a directory's presence as launch authority.
#[cfg(target_os = "linux")]
pub(super) fn planned_bound_directory(root: &Path, session: uuid::Uuid) -> Result<PathBuf> {
    ensure!(!session.is_nil(), "invalid runtime identity");
    let (_, runtime) = runtime_root(root)?;
    Ok(runtime.join(session.to_string()))
}

#[cfg(target_os = "linux")]
pub(super) fn transfer_bound_directory(
    root: &Path,
    session: uuid::Uuid,
    source_uid: u32,
    source_gid: u32,
    target_uid: u32,
    target_gid: u32,
    inventory: &[(u64, u64, u32, u64)],
) -> Result<PathBuf> {
    let (runtime, path) = runtime_root(root)?;
    runtime.transfer_session(
        session.to_string().as_ref(),
        source_uid,
        source_gid,
        target_uid,
        target_gid,
        inventory,
    )?;
    Ok(path.join(session.to_string()))
}

#[cfg(target_os = "linux")]
pub(super) fn transfer_inventory(
    root: &Path,
    session: uuid::Uuid,
    uid: u32,
    gid: u32,
) -> Result<Vec<(u64, u64, u32, u64)>> {
    let (runtime, _) = runtime_root(root)?;
    runtime.transfer_inventory(session.to_string().as_ref(), uid, gid)
}

/// Copy only opaque root-owned signed transport bytes into the selected private
/// realm. Existing user-owned contents are never read or repaired as root.
#[cfg(target_os = "linux")]
pub(super) fn stage_initialization_artifact(
    root: &Path,
    session: uuid::Uuid,
    uid: u32,
    gid: u32,
    source: &Path,
    sha: &str,
) -> Result<PathBuf> {
    use sha2::Digest;
    use std::{
        io::Write,
        os::fd::{AsRawFd, FromRawFd},
    };
    let parent = source
        .parent()
        .ok_or_else(|| anyhow::anyhow!("artifact parent missing"))?;
    let name = source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("artifact name missing"))?;
    let bytes = voyage_storage::protected_linux::RootDirectory::open(parent)?
        .read(name, 16 * 1024 * 1024)?;
    ensure!(
        format!("{:x}", sha2::Sha256::digest(&bytes)) == sha,
        "signed artifact digest changed"
    );
    let (runtime, path) = runtime_root(root)?;
    let directory = runtime.session(session.to_string().as_ref(), uid, gid)?;
    let entry = c"initialization-artifact.json";
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            entry.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        let error = std::io::Error::last_os_error();
        ensure!(
            error.kind() == std::io::ErrorKind::AlreadyExists,
            "artifact staging failed"
        );
        // The identity helper will verify an existing private artifact against
        // the retained signed digest; an interrupted write is never repaired.
        return Ok(path
            .join(session.to_string())
            .join("initialization-artifact.json"));
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    ensure!(
        unsafe { libc::fchown(fd, uid, gid) } == 0,
        "artifact ownership publication failed"
    );
    file.write_all(&bytes)?;
    file.sync_all()?;
    directory.sync_all()?;
    Ok(path
        .join(session.to_string())
        .join("initialization-artifact.json"))
}
