//! Workspace-contained checkouts, separate from private voyage state.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};

const DIRECTORY: &str = ".voyage-worktrees";
const IGNORE: &[u8] = b"# Voyage managed checkouts; not project source.\n*\n";

#[derive(Clone, Debug)]
pub(super) struct ManagedRoot {
    workspace: PathBuf,
    // Cloned managers (including nested agents) initialize one scope serially.
    // Separate voyages have distinct scope directories and need no shared lock.
    initialization: Arc<Mutex<()>>,
}

impl WorktreeManager {
    /// Resolve only; discovery never creates directories or grants extra roots.
    /// The stable resource identity scopes allocation, but no checkout goes there.
    pub fn discover_managed(workspace: &Path, resources: &Path) -> Result<Option<Self>> {
        let workspace = workspace.canonicalize()?;
        let scope = hex::encode(Sha256::digest(resources.as_os_str().as_encoded_bytes()));
        let root = workspace.join(DIRECTORY).join(scope);
        let Some(mut manager) = Self::discover(&workspace, root)? else {
            return Ok(None);
        };
        // Existing retained leases are not relocated or implicitly authorized.
        let workspace_key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
        manager.legacy_root = Some(resources.join("worktrees").join(workspace_key));
        manager.managed = Some(ManagedRoot {
            workspace,
            initialization: Arc::new(Mutex::new(())),
        });
        Ok(Some(manager))
    }
}

impl ManagedRoot {
    pub(super) fn prepare(&self, manager: &WorktreeManager) -> Result<()> {
        let _lock = self
            .initialization
            .lock()
            .map_err(|_| anyhow::anyhow!("worktree initialization lock poisoned"))?;
        if let Some(policy) = &manager.policy {
            policy.check_current()?;
            policy.check_delegated_workspace(&manager.root)?;
            anyhow::ensure!(
                policy.access_mode() != crate::config::AccessMode::ReadOnly,
                "worktree creation is disabled in read-only access mode"
            );
        }
        // Never shadow tracked project data, including a tracked ignore file.
        let tracked = git_output_env(
            manager.environment.as_ref(),
            manager.policy.as_deref(),
            &manager.repository,
            ["ls-files", "--", DIRECTORY],
        )?;
        anyhow::ensure!(
            tracked.is_empty(),
            "managed worktree area collides with tracked project files"
        );
        let base = self.workspace.join(DIRECTORY);
        directory(&base)?;
        let created = directory(&manager.root)?;
        let marker = manager.root.join(".gitignore");
        if created {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&marker)?;
            file.write_all(IGNORE)?;
            file.sync_all()?;
        }
        // An existing unmanaged/incomplete area requires operator reconciliation;
        // never overwrite its ignore rules or adopt arbitrary contents silently.
        let metadata = std::fs::symlink_metadata(&marker)
            .context("managed worktree area has no ownership marker; preserve it and reconcile before retrying")?;
        anyhow::ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "invalid managed worktree marker"
        );
        let mut options = std::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
        }
        let mut bytes = Vec::new();
        options
            .open(&marker)?
            .take((IGNORE.len() + 1) as u64)
            .read_to_end(&mut bytes)?;
        anyhow::ensure!(
            bytes == IGNORE,
            "managed worktree marker differs; refusing to overwrite existing data"
        );
        // Recheck after directory preparation, before Git can create a checkout.
        if let Some(policy) = &manager.policy {
            policy.check_current()?;
            policy.check_delegated_workspace(&manager.root)?;
        }
        Ok(())
    }
}

fn directory(path: &Path) -> Result<bool> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    let created = match builder.create(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error.into()),
    };
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "managed worktree directory must be a real directory, not a symlink: {}",
        path.display()
    );
    Ok(created)
}
