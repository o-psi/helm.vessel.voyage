//! Cross-process browser capacity. Reservations survive uncertain cleanup/crashes.
//! A dropped handle does not prove that browser descendants stopped.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

/// Operator recovery is deliberately separate from dropping a reservation. It
/// records the observed cleanup and retains the stale worker lock as evidence.
#[cfg(target_os = "linux")]
pub(crate) fn recover(
    session_dir: &Path,
    session: Uuid,
    observed_no_descendants: bool,
    reason: &str,
) -> Result<serde_json::Value> {
    use serde_json::{Value, json};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    ensure!(
        !session.is_nil() && observed_no_descendants,
        "explicit session and observed descendant cleanup required"
    );
    ensure!(
        !reason.trim().is_empty() && reason.len() <= 512,
        "a short recovery reason is required"
    );
    ensure!(session_dir.is_dir(), "session directory missing");
    crate::attachment::journal::prepare_directory(session_dir.to_path_buf())?;
    // Hold both session locks while checking state and changing capacity. A
    // concurrent Vessel resume cannot start another worker in this interval.
    let guardian =
        crate::attachment::journal::open_private_file(&session_dir.join("guardian.lock"))?;
    guardian
        .try_lock()
        .context("session guardian still owned")?;
    let startup = crate::attachment::journal::open_private_file(&session_dir.join("startup.lock"))?;
    startup.try_lock().context("session startup still owned")?;
    let read_private = |path: &Path| -> Result<Vec<u8>> {
        let meta = fs::symlink_metadata(path)?;
        ensure!(
            meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.nlink() == 1
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.len() <= 16_384,
            "recovery evidence is not a bounded private file"
        );
        Ok(fs::read(path)?)
    };
    let registration: Value =
        serde_json::from_slice(&read_private(&session_dir.join("registration.json"))?)?;
    let stopped: Value = serde_json::from_slice(&read_private(&session_dir.join("stopped.json"))?)?;
    ensure!(
        registration["session_id"] == session.to_string()
            && stopped["session_id"] == session.to_string()
            && stopped["suspended"] == true
            && stopped["cleanup_observed"] == true,
        "session is not suspended with observed cleanup"
    );
    let worker_dir = session_dir.join("journal/host-browser");
    crate::attachment::journal::prepare_directory(worker_dir.clone())?;
    let worker_path = worker_dir.join("worker.lock");
    let worker: Value = serde_json::from_slice(&read_private(&worker_path)?)?;
    let pid = worker["pid"].as_u64().context("worker lock lacks PID")?;
    ensure!(pid > 1 && pid <= i32::MAX as u64, "invalid worker PID");
    ensure!(
        !Path::new(&format!("/proc/{pid}")).exists(),
        "worker PID is present; cleanup unconfirmed"
    );
    let root = crate::config::default_data_dir().join("host-browser-capacity");
    crate::attachment::local_actor::storage::Directory::open(&root)?.verify()?;
    let mut slots = Vec::new();
    for index in 0..4 {
        let path = root.join(format!("browser-slot-{index}"));
        match fs::symlink_metadata(&path) {
            Ok(_) if read_private(&path)? == session.to_string().as_bytes() => {
                slots.push((index, path))
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    ensure!(!slots.is_empty(), "session has no browser capacity slots");
    let recovery = Uuid::new_v4();
    let retained = worker_dir.join(format!("worker.lock.recovered-{recovery}"));
    let audit = session_dir.join(format!("browser-capacity-recovery-{recovery}.json"));
    let record = json!({"recovery_id": recovery, "session_id": session,
        "worker_pid": pid, "slots": slots.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
        "reason": reason, "observed_no_descendants": true,
        "external_effects_reconciled": false,
        "observed_at_utc": chrono::Utc::now().to_rfc3339()});
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(&audit)?;
    file.write_all(serde_json::to_string_pretty(&record)?.as_bytes())?;
    file.sync_all()?;
    fs::rename(&worker_path, &retained)?;
    File::open(&worker_dir)?.sync_all()?;
    for (_, path) in &slots {
        ensure!(
            read_private(path)? == session.to_string().as_bytes(),
            "browser slot changed during recovery"
        );
        fs::remove_file(path)?;
    }
    File::open(&root)?.sync_all()?;
    File::open(session_dir)?.sync_all()?;
    Ok(record)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn recover(_: &Path, _: Uuid, _: bool, _: &str) -> Result<serde_json::Value> {
    anyhow::bail!("browser capacity recovery requires Linux process evidence")
}

#[derive(Debug)]
pub struct Capacity {
    path: PathBuf,
    file: File,
    owner: Uuid,
}
impl Capacity {
    pub fn acquire(root: &Path, owner: Uuid, slots: usize) -> Result<Self> {
        ensure!(
            !owner.is_nil() && (1..=64).contains(&slots),
            "invalid browser capacity configuration"
        );
        let private = crate::attachment::local_actor::storage::Directory::open(root)?;
        private.verify()?;
        for index in 0..slots {
            let path = root.join(format!("browser-slot-{index}"));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    // A failed write retains the slot: uncertain ownership is not free capacity.
                    file.write_all(owner.to_string().as_bytes())?;
                    file.sync_all()?;
                    #[cfg(unix)]
                    File::open(root)?.sync_all()?;
                    return Ok(Self { path, file, owner });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        anyhow::bail!("browser capacity is in use or awaiting observed cleanup")
    }
    /// Only call after worker + browser cleanup has been positively observed.
    pub fn release(self) -> Result<()> {
        let current = fs::symlink_metadata(&self.path)?;
        let held = self.file.metadata()?;
        ensure!(
            current.is_file() && !current.file_type().is_symlink(),
            "browser capacity reservation changed"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                current.dev() == held.dev()
                    && current.ino() == held.ino()
                    && current.uid() == unsafe { libc::geteuid() }
                    && current.mode() & 0o077 == 0,
                "browser capacity reservation replaced"
            );
        }
        ensure!(
            fs::read(&self.path).context("browser capacity reservation unavailable")?
                == self.owner.to_string().as_bytes(),
            "browser capacity owner changed"
        );
        fs::remove_file(&self.path)?;
        #[cfg(unix)]
        File::open(self.path.parent().unwrap())?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservations_are_exclusive_and_drop_never_attests_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("capacity");
        let first = Capacity::acquire(&root, Uuid::new_v4(), 1).unwrap();
        assert!(Capacity::acquire(&root, Uuid::new_v4(), 1).is_err());
        first.release().unwrap();
        let second = Capacity::acquire(&root, Uuid::new_v4(), 1).unwrap();
        drop(second);
        assert!(Capacity::acquire(&root, Uuid::new_v4(), 1).is_err());
    }
    #[test]
    fn changed_reservation_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("capacity");
        let slot = Capacity::acquire(&root, Uuid::new_v4(), 1).unwrap();
        fs::write(&slot.path, b"replacement").unwrap();
        assert!(slot.release().is_err());
        assert_eq!(
            fs::read(root.join("browser-slot-0")).unwrap(),
            b"replacement"
        );
    }
}
