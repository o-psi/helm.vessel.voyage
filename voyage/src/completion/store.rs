//! Durable ledger storage, not a transaction across todo/agent/session stores.
//!
//! The caller supplies a trusted private data directory. Cooperating processes
//! serialize ledger reads/writes with a stable sidecar lock; contention fails
//! immediately (never wait on an operator or hold an async executor blocked).
//! Do not delete/replace the lock file while any store handle is in use.
//! Windows uses a flushed temporary file and a same-directory write-through
//! MoveFileExW operation. Filesystem/device flush guarantees still apply.
use super::{MAX_LEDGER_BYTES, RunId, RunLedger};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

const STORE_VERSION: u32 = 1;
const MAX_STORE_BYTES: usize = 2 * MAX_LEDGER_BYTES + 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunScope {
    workspace: PathBuf,
    session_id: Uuid,
}
impl RunScope {
    /// Resolve workspace aliases once, before persisting any run. A session ID
    /// alone is insufficient to authorize access to a workspace's obligations.
    pub fn new(workspace: &Path, session_id: Uuid) -> Result<Self> {
        let workspace = workspace.canonicalize().context("resolve run workspace")?;
        ensure!(workspace.is_dir(), "run workspace is not a directory");
        Ok(Self {
            workspace,
            session_id,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredRun {
    version: u32,
    scope: RunScope,
    // Keep the inner JSON intact so strict ledger decoding sees duplicate fields.
    ledger: String,
}

#[derive(Clone, Debug)]
pub struct RunLedgerStore {
    directory: PathBuf,
    scope: RunScope,
}
struct WriterLock(File);
impl Drop for WriterLock {
    fn drop(&mut self) {
        // Explicit unlock releases descriptors briefly inherited during fork/exec.
        let _ = self.0.unlock();
    }
}

impl RunLedgerStore {
    /// `directory` is a dedicated child of a trusted data directory; its parent
    /// must already exist. Existing directories must be private on Unix. This
    /// is application persistence, not isolation from hostile same-user code.
    pub fn open(directory: PathBuf, scope: RunScope) -> Result<Self> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        #[cfg(not(unix))]
        let _ = &mut builder;
        match builder.create(&directory) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error).context("create completion store directory"),
        }
        let metadata = fs::symlink_metadata(&directory)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "completion store must be a real directory"
        );
        ensure_private(&metadata)?;
        let directory = directory.canonicalize()?;
        // Sync on every open, including a retry after an earlier sync failure.
        // Syncing only the new directory does not persist its name in the parent.
        #[cfg(unix)]
        File::open(
            directory
                .parent()
                .context("completion directory has no parent")?,
        )?
        .sync_all()
        .context("sync completion store parent directory")?;
        Ok(Self { directory, scope })
    }

    /// Create only: never replace an existing run, even if its file is malformed.
    /// Persist this before publishing a reference in a canonical session.
    pub fn create(&self, ledger: &RunLedger) -> Result<()> {
        let _lock = self.lock()?;
        self.write(ledger, false)
    }

    /// A known run must exist. Missing/corrupt state is an error, never a clean run.
    pub fn load(&self, run_id: RunId) -> Result<RunLedger> {
        let _lock = self.lock()?;
        self.read(run_id)
    }

    /// Reload under the cross-process lock, validate the caller's revision, then
    /// commit the entire candidate. Closure errors leave the original untouched.
    /// A post-rename durability error is an uncertain commit: reload and inspect,
    /// never blindly retry consequential work. No provider/tools run in this lock.
    pub fn update(
        &self,
        run_id: RunId,
        expected_revision: u64,
        operation: impl FnOnce(&mut RunLedger) -> Result<()>,
    ) -> Result<RunLedger> {
        let _lock = self.lock()?;
        let mut ledger = self.read(run_id)?;
        ensure!(
            ledger.revision() == expected_revision,
            "stale completion ledger revision"
        );
        let original = ledger.clone();
        ledger.ensure_open()?;
        operation(&mut ledger)?;
        ensure!(ledger.run_id() == run_id, "completion run identity changed");
        ensure!(
            ledger.revision() >= expected_revision,
            "completion revision regressed"
        );
        ensure!(
            ledger == original || ledger.revision() > expected_revision,
            "changed completion ledger must advance revision"
        );
        ensure!(
            ledger.entries.len() >= original.entries.len()
                && original
                    .entries
                    .iter()
                    .zip(&ledger.entries)
                    .all(|(old, new)| old.obligation == new.obligation
                        && new.dispositions.starts_with(&old.dispositions)),
            "completion update cannot remove membership or review history"
        );
        if ledger != original {
            self.write(&ledger, true)?;
        }
        Ok(ledger)
    }

    fn path(&self, run_id: RunId) -> PathBuf {
        self.directory
            .join(format!("{}-{}.json", self.scope.session_id, run_id.0))
    }

    fn lock(&self) -> Result<WriterLock> {
        let path = self.directory.join("writer.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        safe_options(&mut options);
        reject_non_file(&path)?;
        let file = options.open(path).context("open completion store lock")?;
        ensure!(
            file.metadata()?.is_file(),
            "completion lock is not a regular file"
        );
        ensure_private(&file.metadata()?)?;
        file.try_lock()
            .context("completion store busy or file locking unavailable")?;
        Ok(WriterLock(file)) // Process exit also releases the lock.
    }

    fn read(&self, run_id: RunId) -> Result<RunLedger> {
        let path = self.path(run_id);
        reject_non_file(&path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        safe_options(&mut options);
        let file = options
            .open(path)
            .context("known completion run is unavailable")?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file(),
            "completion ledger is not a regular file"
        );
        ensure_private(&metadata)?;
        ensure!(
            metadata.len() <= MAX_STORE_BYTES as u64,
            "completion store exceeds byte limit"
        );
        let mut bytes = Vec::new();
        file.take(MAX_STORE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= MAX_STORE_BYTES,
            "completion store exceeds byte limit"
        );
        let stored: StoredRun =
            serde_json::from_slice(&bytes).context("malformed completion store")?;
        ensure!(
            stored.version == STORE_VERSION,
            "unsupported completion store version"
        );
        ensure!(
            stored.scope == self.scope,
            "completion store scope mismatch"
        );
        let ledger = RunLedger::from_json(stored.ledger.as_bytes())?;
        ensure!(
            ledger.run_id() == run_id,
            "completion store run identity mismatch"
        );
        Ok(ledger)
    }

    fn write(&self, ledger: &RunLedger, replace: bool) -> Result<()> {
        let bytes = serde_json::to_vec(&StoredRun {
            version: STORE_VERSION,
            scope: self.scope.clone(),
            ledger: String::from_utf8(ledger.to_json()?)?,
        })?;
        ensure!(
            bytes.len() <= MAX_STORE_BYTES,
            "completion store exceeds byte limit"
        );
        let path = self.path(ledger.run_id());
        reject_non_file(&path)?;
        let mut temp = tempfile::NamedTempFile::new_in(&self.directory)?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        // NamedTempFile is owner-only on Unix from creation, not chmod-after-write.
        #[cfg(windows)]
        persist_windows(temp, &path, replace)?;
        #[cfg(not(windows))]
        if replace {
            temp.persist(path).context("replace completion ledger")?;
        } else {
            temp.persist_noclobber(path)
                .context("create completion ledger without replacement")?;
        }
        #[cfg(unix)]
        File::open(&self.directory)?
            .sync_all()
            .context("sync completion directory; commit may already be visible")?;
        Ok(())
    }
}

/// Request same-volume write-through publication, with no copy/delete fallback.
/// MoveFileExW's WRITE_THROUGH flag is the documented Windows flush boundary:
/// https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw
#[cfg(windows)]
fn persist_windows(temp: tempfile::NamedTempFile, destination: &Path, replace: bool) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    fn wide(path: &Path) -> Result<Vec<u16>> {
        let mut value: Vec<_> = path.as_os_str().encode_wide().collect();
        ensure!(
            !value.contains(&0),
            "completion path contains a null character"
        );
        value.push(0);
        Ok(value)
    }
    // Close the file before renaming, keeping TempPath cleanup on every error.
    let source = temp.into_temp_path();
    let existing = wide(&source)?;
    let new = wide(destination)?;
    const MOVEFILE_REPLACE_EXISTING: u32 = 1;
    const MOVEFILE_WRITE_THROUGH: u32 = 8;
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // SAFETY: both pointers refer to live, NUL-terminated UTF-16 buffers for the
    // duration of the synchronous call; flags request only documented operations.
    let result = unsafe { MoveFileExW(existing.as_ptr(), new.as_ptr(), flags) };
    if result == 0 {
        return Err(std::io::Error::last_os_error())
            .context("write-through completion publication");
    }
    Ok(())
}

fn reject_non_file(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "completion path is not a regular file"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
    }
    Ok(())
}
fn safe_options(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(not(unix))]
    let _ = options;
}
fn ensure_private(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "completion store permissions are not private"
        );
    }
    #[cfg(not(unix))]
    let _ = metadata; // Inherits the trusted data directory's Windows ACL.
    Ok(())
}
