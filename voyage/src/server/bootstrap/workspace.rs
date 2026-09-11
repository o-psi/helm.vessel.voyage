//! Recover the registered directory, never a replacement workspace or its files.
use super::*;
use std::path::Path;

pub(crate) const NOTICE: &str = "The working directory was deleted and recreated empty. Conversation history is preserved, but previous files were not restored. Git access is blocked until you restore or initialize this directory explicitly.";
const GIT_GUARD: &[u8] = b"gitdir: .voyage-missing-checkout\n";

pub(crate) fn prepare(directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    let workspace = &registration.workspace;
    match std::fs::symlink_metadata(workspace) {
        Ok(_) => return Ok(()), // Ordinary startup retains its existing validation.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    ensure!(
        registration.restart_from.is_some(),
        "new workspace is missing"
    );
    let journal = Journal::open(directory.join("journal"))?;
    let _execution = journal.acquire_execution(registration.session_id)?;
    let saved = journal.load_session(registration.session_id)?;
    ensure!(
        saved.session.workspace == *workspace,
        "saved workspace mismatch"
    );
    // Require retained host settings. Startup still resolves the current policy
    // and grants after creation; no settings or roots are replaced by defaults.
    let settings = journal
        .initial_configuration(registration.session_id)?
        .context("missing workspace has no retained configuration")?;
    serde_json::from_str::<crate::launch_config::LaunchConfig>(&settings)?
        .observation_config(workspace)?;
    let parent = workspace.parent().context("workspace parent missing")?;
    ensure!(
        parent.canonicalize()? == parent,
        "workspace parent changed or is missing"
    );
    let name = workspace.file_name().context("workspace name missing")?;

    #[cfg(target_os = "linux")]
    {
        use std::{
            ffi::CString,
            io::Write,
            os::{
                fd::AsRawFd,
                unix::{
                    ffi::OsStrExt,
                    fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
                },
            },
        };
        let parent_handle = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(parent)?;
        let name = CString::new(name.as_bytes())?;
        // Publish a complete directory atomically: a crash must never leave a
        // bare directory that Git could mistake for part of its parent checkout.
        let candidate = tempfile::Builder::new()
            .prefix(".voyage-recreate-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir_in(format!("/proc/self/fd/{}", parent_handle.as_raw_fd()))?;
        let mut guard = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(candidate.path().join(".git"))?;
        guard.write_all(GIT_GUARD)?;
        guard.sync_all()?;
        let metadata = candidate.path().metadata()?;
        let mut marker = tempfile::NamedTempFile::new_in(directory)?;
        serde_json::to_writer(
            &mut marker,
            &serde_json::json!({"session_id":registration.session_id,"workspace":workspace,"device":metadata.dev(),"inode":metadata.ino()}),
        )?;
        marker.flush()?;
        marker.as_file().sync_all()?;
        marker.persist(directory.join("workspace-recreated.json"))?;
        std::fs::File::open(directory)?.sync_all()?;
        std::fs::File::open(candidate.path())?.sync_all()?;
        let source = CString::new(
            candidate
                .path()
                .file_name()
                .context("recovery name missing")?
                .as_bytes(),
        )?;
        let current_parent = parent.metadata()?;
        let pinned_parent = parent_handle.metadata()?;
        ensure!(
            parent.canonicalize()? == parent
                && current_parent.dev() == pinned_parent.dev()
                && current_parent.ino() == pinned_parent.ino(),
            "workspace parent changed during recreation"
        );
        ensure!(
            unsafe {
                libc::renameat2(
                    parent_handle.as_raw_fd(),
                    source.as_ptr(),
                    parent_handle.as_raw_fd(),
                    name.as_ptr(),
                    libc::RENAME_NOREPLACE,
                )
            } == 0,
            "workspace recreation failed; restore the registered directory before retrying"
        );
        parent_handle.sync_all()?;
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("automatic workspace recreation is unavailable on this platform");
    Ok(())
}

/// Match the created inode and Git guard, so an explicit restoration ends the notice.
pub(crate) fn recreated(directory: &Path, registration: &ProcessRegistration) -> bool {
    #[cfg(unix)]
    {
        use std::{
            io::Read,
            os::unix::fs::{MetadataExt, OpenOptionsExt},
        };
        let Ok(bytes) = super::read_private_artifact(&directory.join("workspace-recreated.json"))
        else {
            return false;
        };
        let Ok(marker) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return false;
        };
        let Ok(metadata) = registration.workspace.metadata() else {
            return false;
        };
        let git = registration.workspace.join(".git");
        let guarded = (|| -> Result<bool> {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(git)?;
            let metadata = file.metadata()?;
            if !metadata.is_file() || metadata.len() != GIT_GUARD.len() as u64 {
                return Ok(false);
            }
            let mut bytes = Vec::new();
            file.take(GIT_GUARD.len() as u64 + 1)
                .read_to_end(&mut bytes)?;
            Ok(bytes == GIT_GUARD)
        })();
        if !guarded.unwrap_or(false) {
            return false;
        }
        marker["session_id"] == registration.session_id.to_string()
            && marker["workspace"] == registration.workspace.to_string_lossy().as_ref()
            && marker["device"] == metadata.dev()
            && marker["inode"] == metadata.ino()
    }
    #[cfg(not(unix))]
    false
}

pub(crate) fn annotate(
    snapshot: &mut serde_json::Value,
    directory: &Path,
    registration: &ProcessRegistration,
) {
    let workspace = &registration.workspace;
    let notice = if !workspace.exists() {
        Some(
            if workspace.parent().is_some_and(|parent| {
                parent
                    .canonicalize()
                    .is_ok_and(|resolved| resolved == parent)
            }) {
                "The working directory is missing. Your conversation is preserved. A new message will attempt to recreate the directory; deleted files will not be restored."
            } else {
                "The working directory is missing. Restore its parent directory at the original path, then resend your draft. Your conversation is preserved."
            },
        )
    } else if recreated(directory, registration) {
        Some(NOTICE)
    } else {
        None
    };
    if let Some(notice) = notice {
        snapshot["recovery_notice"] =
            serde_json::json!(match snapshot["recovery_notice"].as_str() {
                Some(previous) => format!("{previous} {notice}"),
                None => notice.into(),
            });
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};

    fn saved(root: &Path) -> Result<(PathBuf, ProcessRegistration)> {
        let workspace = root.join("parent/workspace");
        std::fs::create_dir_all(&workspace)?;
        let directory = crate::attachment::journal::prepare_directory(root.join("runtime"))?;
        let mut journal = Journal::open(directory.join("journal"))?;
        let config = Config::default();
        let session = Session::new(workspace.clone(), config.model.clone());
        journal.create_session(&session)?;
        let guard = journal.acquire_execution(session.id)?;
        let settings = serde_json::to_string(&crate::launch_config::LaunchConfig::capture(
            &config, &workspace,
        )?)?;
        journal.retain_initial_configuration(&guard, settings)?;
        let registration = ProcessRegistration {
            executable: None,
            protocol: voyage_protocol::process::PROCESS_PROTOCOL,
            session_id: session.id,
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: Some(Uuid::new_v4()),
            initialize: None,
            config_path: None,
            token: "fixture".into(),
            workspace,
            state: voyage_protocol::process::ProcessState::Starting,
            name: None,
        };
        Ok((directory, registration))
    }

    #[test]
    fn missing_workspace_is_private_and_does_not_restore_files() -> Result<()> {
        let root = tempfile::tempdir()?;
        let (directory, registration) = saved(root.path())?;
        std::fs::remove_dir(&registration.workspace)?;
        prepare(&directory, &registration)?;
        assert_eq!(registration.workspace.metadata()?.mode() & 0o777, 0o700);
        assert_eq!(
            std::fs::read(registration.workspace.join(".git"))?,
            GIT_GUARD
        );
        assert!(recreated(&directory, &registration));
        std::fs::remove_file(registration.workspace.join(".git"))?;
        assert!(!recreated(&directory, &registration));
        Ok(())
    }

    #[test]
    fn live_owner_and_new_session_cannot_recreate_workspace() -> Result<()> {
        let root = tempfile::tempdir()?;
        let (directory, mut registration) = saved(root.path())?;
        std::fs::remove_dir(&registration.workspace)?;
        let journal = Journal::open(directory.join("journal"))?;
        let guard = journal.acquire_execution(registration.session_id)?;
        assert!(prepare(&directory, &registration).is_err());
        assert!(!registration.workspace.exists());
        drop(guard);
        registration.restart_from = None;
        assert!(prepare(&directory, &registration).is_err());
        assert!(!registration.workspace.exists());
        Ok(())
    }

    #[test]
    fn missing_or_redirected_parent_is_not_created_or_followed() -> Result<()> {
        let root = tempfile::tempdir()?;
        let (directory, registration) = saved(root.path())?;
        std::fs::remove_dir(&registration.workspace)?;
        let parent = registration.workspace.parent().unwrap();
        std::fs::remove_dir(parent)?;
        assert!(prepare(&directory, &registration).is_err());
        assert!(!parent.exists());
        let other = root.path().join("other");
        std::fs::create_dir(&other)?;
        symlink(&other, parent)?;
        assert!(prepare(&directory, &registration).is_err());
        assert_eq!(std::fs::read_dir(other)?.count(), 0);
        Ok(())
    }

    #[test]
    fn existing_path_is_never_overwritten() -> Result<()> {
        let root = tempfile::tempdir()?;
        let (directory, registration) = saved(root.path())?;
        let keep = registration.workspace.join("keep.txt");
        std::fs::write(&keep, "preserved")?;
        prepare(&directory, &registration)?;
        assert_eq!(std::fs::read_to_string(keep)?, "preserved");
        assert!(!registration.workspace.join(".git").exists());
        assert!(!recreated(&directory, &registration));
        Ok(())
    }
}
