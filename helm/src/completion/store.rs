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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{completion::Obligation, todo::TodoId};

    fn store(root: &Path) -> RunLedgerStore {
        RunLedgerStore::open(
            root.join("runs"),
            RunScope::new(root, Uuid::new_v4()).unwrap(),
        )
        .unwrap()
    }
    fn adopt(ledger: &mut RunLedger) -> Result<()> {
        ledger.adopt(Obligation::Todo(TodoId(Uuid::new_v4())), ledger.revision())
    }
    fn rewrite(store: &RunLedgerStore, id: RunId, edit: impl FnOnce(&mut serde_json::Value)) {
        let path = store.path(id);
        let mut value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        edit(&mut value);
        fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
    }

    #[test]
    fn sealed_store_cannot_be_reopened_by_replacement_candidate() {
        let root = tempfile::tempdir().unwrap();
        let scope = RunScope::new(root.path(), Uuid::new_v4()).unwrap();
        let store = RunLedgerStore::open(root.path().join("ledgers"), scope).unwrap();
        let mut ledger = RunLedger::new();
        let readiness = super::super::Readiness {
            run_id: ledger.run_id(),
            revision: 0,
            fingerprint: "a".repeat(64),
            total: 0,
            accounted: 0,
            completed: 0,
            incomplete: 0,
            incomplete_obligations: vec![],
            unresolved: vec![],
            omitted_unresolved: 0,
        };
        ledger
            .seal(readiness, super::super::FinalOutcome::Completed, None)
            .unwrap();
        store.create(&ledger).unwrap();
        let before = fs::read(store.path(ledger.run_id())).unwrap();
        assert!(
            store
                .update(ledger.run_id(), ledger.revision(), |candidate| {
                    *candidate = RunLedger::with_id(ledger.run_id());
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(store.load(ledger.run_id()).unwrap(), ledger);
        assert_eq!(fs::read(store.path(ledger.run_id())).unwrap(), before);
    }

    #[test]
    fn durable_roundtrip_stale_updates_and_failed_mutation() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        let id = ledger.run_id();
        store.create(&ledger).unwrap();
        assert_eq!(store.load(id).unwrap(), ledger);
        let updated = store.update(id, 0, adopt).unwrap();
        let reopened = RunLedgerStore::open(store.directory.clone(), store.scope.clone()).unwrap();
        assert_eq!(reopened.load(id).unwrap(), updated);
        assert!(reopened.update(id, 0, adopt).is_err());
        let before = fs::read(store.path(id)).unwrap();
        assert!(
            store
                .update(id, 1, |candidate| {
                    adopt(candidate)?;
                    anyhow::bail!("injected failure")
                })
                .is_err()
        );
        assert!(
            store
                .update(id, 1, |candidate| {
                    *candidate = RunLedger::new();
                    Ok(())
                })
                .is_err()
        );
        assert!(
            store
                .update(id, 1, |candidate| {
                    let mut value = serde_json::to_value(&*candidate)?;
                    value["entries"] = serde_json::json!([]);
                    *candidate = RunLedger::from_json(&serde_json::to_vec(&value)?)?;
                    Ok(())
                })
                .is_err()
        );
        assert!(
            store
                .update(id, 1, |candidate| {
                    let mut value = serde_json::to_value(&*candidate)?;
                    value["entries"] = serde_json::json!([]);
                    value["revision"] = serde_json::json!(2);
                    *candidate = RunLedger::from_json(&serde_json::to_vec(&value)?)?;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(store.update(id, 1, |_| Ok(())).unwrap(), updated);
        assert_eq!(fs::read(store.path(id)).unwrap(), before);
        assert!(store.create(&ledger).is_err());
        assert_eq!(store.load(id).unwrap(), updated);
        // Temporary write files must not linger after ordinary failures/success.
        assert_eq!(fs::read_dir(&store.directory).unwrap().count(), 2);
    }

    #[test]
    fn known_runs_never_recover_as_empty() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        let id = ledger.run_id();
        assert!(store.load(id).is_err());
        assert!(store.update(id, 0, adopt).is_err());
        store.create(&ledger).unwrap();
        fs::write(store.path(id), b"{").unwrap();
        assert!(store.load(id).is_err());
        assert!(store.create(&ledger).is_err());
        assert_eq!(fs::read(store.path(id)).unwrap(), b"{");
        fs::remove_file(store.path(id)).unwrap();
        assert!(store.load(id).is_err());
    }

    #[test]
    fn rejects_wrong_scope_identity_versions_and_unknown_fields() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        let id = ledger.run_id();
        store.create(&ledger).unwrap();
        let original = fs::read(store.path(id)).unwrap();
        let edits: [fn(&mut serde_json::Value); 6] = [
            |v| v["version"] = 999.into(),
            |v| v["extra"] = true.into(),
            |v| v["scope"]["session_id"] = Uuid::new_v4().to_string().into(),
            |v| v["scope"]["workspace"] = "other".into(),
            |v| {
                let mut inner: serde_json::Value =
                    serde_json::from_str(v["ledger"].as_str().unwrap()).unwrap();
                inner["run_id"] = Uuid::new_v4().to_string().into();
                v["ledger"] = serde_json::to_string(&inner).unwrap().into();
            },
            |v| {
                v["ledger"] = v["ledger"]
                    .as_str()
                    .unwrap()
                    .replacen("\"version\":2", "\"version\":2,\"version\":2", 1)
                    .into();
            },
        ];
        for edit in edits {
            rewrite(&store, id, edit);
            assert!(store.load(id).is_err());
            fs::write(store.path(id), &original).unwrap();
        }
        let mut other = store.clone();
        other.scope.session_id = Uuid::new_v4();
        assert!(other.load(id).is_err());
        // Copying a real run under another session's filename does not authorize it.
        fs::copy(store.path(id), other.path(id)).unwrap();
        assert!(other.load(id).is_err());
    }

    #[test]
    fn oversized_store_is_rejected_before_unbounded_read() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        OpenOptions::new()
            .write(true)
            .open(store.path(ledger.run_id()))
            .unwrap()
            .set_len(MAX_STORE_BYTES as u64 + 1)
            .unwrap();
        assert!(
            store
                .load(ledger.run_id())
                .unwrap_err()
                .to_string()
                .contains("byte limit")
        );
    }

    #[test]
    fn lock_contention_fails_immediately_and_releases_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        let lock = store.lock().unwrap();
        assert!(store.load(ledger.run_id()).is_err());
        assert!(store.update(ledger.run_id(), 0, adopt).is_err());
        assert!(store.create(&RunLedger::new()).is_err());
        // A duplicated descriptor models inheritance during concurrent fork/exec.
        // Guard drop must explicitly unlock even while that descriptor survives.
        let inherited = lock.0.try_clone().unwrap();
        drop(lock);
        assert_eq!(store.load(ledger.run_id()).unwrap(), ledger);
        drop(inherited);
    }

    fn wait_bounded(mut child: std::process::Child) -> std::process::Output {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if child.try_wait().unwrap().is_some() {
                return child.wait_with_output().unwrap();
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("completion store subprocess exceeded test deadline");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn process_exit_releases_lock_without_discarding_obligations() {
        const CHILD: &str = "HELM_COMPLETION_CRASH_CHILD";
        if let Some(directory) = std::env::var_os(CHILD) {
            let scope: RunScope =
                serde_json::from_str(&std::env::var("HELM_COMPLETION_CRASH_SCOPE").unwrap())
                    .unwrap();
            let store = RunLedgerStore::open(PathBuf::from(directory), scope).unwrap();
            let _lock = store.lock().unwrap();
            fs::write(store.directory.join("ready"), b"locked").unwrap();
            // Test harness kills this process; no Rust destructors run.
            std::thread::sleep(std::time::Duration::from_secs(30));
            panic!("parent failed to terminate lock holder");
        }
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        let expected = store.update(ledger.run_id(), 0, adopt).unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "completion::store::tests::process_exit_releases_lock_without_discarding_obligations"])
            .env(CHILD, &store.directory)
            .env("HELM_COMPLETION_CRASH_SCOPE", serde_json::to_string(&store.scope).unwrap())
            .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null())
            .spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !store.directory.join("ready").exists() && std::time::Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let ready = store.directory.join("ready").exists();
        let blocked = store.load(ledger.run_id()).is_err();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "child did not acquire lock within deadline");
        assert!(blocked);
        assert_eq!(store.load(ledger.run_id()).unwrap(), expected);
    }

    #[test]
    fn cross_process_lock_and_restart() {
        const CHILD_PATH: &str = "HELM_COMPLETION_STORE_TEST_CHILD";
        if let Some(raw) = std::env::var_os(CHILD_PATH) {
            let scope: RunScope =
                serde_json::from_str(&std::env::var("HELM_COMPLETION_TEST_SCOPE").unwrap())
                    .unwrap();
            let id =
                RunId(Uuid::parse_str(&std::env::var("HELM_COMPLETION_TEST_ID").unwrap()).unwrap());
            let store = RunLedgerStore::open(PathBuf::from(raw), scope).unwrap();
            if std::env::var("HELM_COMPLETION_TEST_BUSY").unwrap() == "yes" {
                assert!(store.load(id).is_err());
            } else {
                assert_eq!(store.load(id).unwrap().revision(), 1);
                store.update(id, 1, adopt).unwrap();
            }
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        store.update(ledger.run_id(), 0, adopt).unwrap();
        let child = |busy: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "completion::store::tests::cross_process_lock_and_restart",
                    "--nocapture",
                ])
                .env(CHILD_PATH, &store.directory)
                .env(
                    "HELM_COMPLETION_TEST_SCOPE",
                    serde_json::to_string(&store.scope).unwrap(),
                )
                .env("HELM_COMPLETION_TEST_ID", ledger.run_id().0.to_string())
                .env("HELM_COMPLETION_TEST_BUSY", busy)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map(wait_bounded)
                .unwrap()
        };
        let lock = store.lock().unwrap();
        let result = child("yes");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        drop(lock);
        let result = child("no");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(store.load(ledger.run_id()).unwrap().revision(), 2);
        assert!(store.update(ledger.run_id(), 1, adopt).is_err());
    }

    #[test]
    fn failed_write_preserves_prior_ledger_and_failed_temp_is_removed() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        let before = fs::read(store.path(ledger.run_id())).unwrap();
        // Replace the destination with a directory after reading the old ledger.
        // Unlike renaming the store directory, this works on Windows while the
        // independent writer.lock file is open. The destination must not be
        // removed by a failed atomic replacement.
        let destination = store.path(ledger.run_id());
        let moved = root.path().join("prior-ledger.json");
        let mut injected = false;
        assert!(
            store
                .update(ledger.run_id(), 0, |candidate| {
                    adopt(candidate)?;
                    fs::rename(&destination, &moved)?;
                    fs::create_dir(&destination)?;
                    injected = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(injected, "fixture must reach the failed-write phase");
        assert!(destination.is_dir(), "failed write removed the destination");
        assert_eq!(fs::read(&moved).unwrap(), before);
        fs::remove_dir(&destination).unwrap();
        fs::rename(&moved, &destination).unwrap();
        assert_eq!(fs::read(store.path(ledger.run_id())).unwrap(), before);
        assert_eq!(store.load(ledger.run_id()).unwrap(), ledger);
        assert_eq!(fs::read_dir(&store.directory).unwrap().count(), 2);
    }

    #[test]
    fn rejects_nonregular_targets_without_losing_data() {
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        fs::create_dir(store.path(ledger.run_id())).unwrap();
        assert!(store.create(&ledger).is_err());
        assert!(store.load(ledger.run_id()).is_err());
        assert!(store.path(ledger.run_id()).is_dir());
        fs::remove_file(store.directory.join("writer.lock")).unwrap();
        fs::create_dir(store.directory.join("writer.lock")).unwrap();
        assert!(store.create(&RunLedger::new()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_creation_and_symlink_rejection() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let root = tempfile::tempdir().unwrap();
        let store = store(root.path());
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        for path in [
            &store.directory,
            &store.path(ledger.run_id()),
            &store.directory.join("writer.lock"),
        ] {
            assert_eq!(fs::metadata(path).unwrap().permissions().mode() & 0o077, 0);
        }
        let victim = root.path().join("victim");
        fs::write(&victim, b"keep me").unwrap();
        fs::remove_file(store.path(ledger.run_id())).unwrap();
        symlink(&victim, store.path(ledger.run_id())).unwrap();
        assert!(store.load(ledger.run_id()).is_err());
        assert!(store.create(&ledger).is_err());
        fs::remove_file(store.directory.join("writer.lock")).unwrap();
        symlink(&victim, store.directory.join("writer.lock")).unwrap();
        assert!(store.create(&RunLedger::new()).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"keep me");
        let alias = root.path().join("alias");
        symlink(&store.directory, &alias).unwrap();
        assert!(RunLedgerStore::open(alias, store.scope.clone()).is_err());
        fs::set_permissions(&store.directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(RunLedgerStore::open(store.directory, store.scope).is_err());
    }
}
