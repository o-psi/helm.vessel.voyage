//! Cross-process browser capacity. Reservations survive uncertain cleanup/crashes.
//! A dropped handle does not prove that browser descendants stopped.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

const BASE_BROWSER_SLOTS: usize = 4;
const MAX_BROWSER_SLOTS: usize = 16;

#[cfg(target_os = "linux")]
fn slots_for_available_memory(available: u64) -> usize {
    const GIB: u64 = 1024 * 1024 * 1024;
    BASE_BROWSER_SLOTS
        + (available.saturating_sub(4 * GIB) / (2 * GIB))
            .min((MAX_BROWSER_SLOTS - BASE_BROWSER_SLOTS) as u64) as usize
}

/// A generous reserve keeps the initial four-slot behavior on constrained
/// hosts, while a host with spare memory need not refuse a fifth live voyage.
/// The cgroup limit takes precedence when the service runs in a container.
#[cfg(target_os = "linux")]
pub fn admission_slots() -> usize {
    let mem_available = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("MemAvailable:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
                    .and_then(|kib| kib.checked_mul(1024))
            })
        })
        .unwrap_or(0);
    slots_for_available_memory(mem_available.min(cgroup_available().unwrap_or(u64::MAX)))
}

#[cfg(target_os = "linux")]
fn cgroup_available() -> Option<u64> {
    let membership = fs::read_to_string("/proc/self/cgroup").ok()?;
    let group = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let mut path = PathBuf::from("/sys/fs/cgroup");
    for component in Path::new(group).components() {
        if let std::path::Component::Normal(segment) = component {
            path.push(segment);
        } else if component != std::path::Component::RootDir {
            return None;
        }
    }
    let root = Path::new("/sys/fs/cgroup");
    let mut least = None;
    loop {
        if let (Ok(max), Ok(current)) = (
            fs::read_to_string(path.join("memory.max")),
            fs::read_to_string(path.join("memory.current")),
        ) {
            if max.trim() != "max" {
                let headroom = max
                    .trim()
                    .parse::<u64>()
                    .ok()?
                    .saturating_sub(current.trim().parse::<u64>().ok()?);
                least = Some(least.map_or(headroom, |prior: u64| prior.min(headroom)));
            }
        }
        if path == root {
            return least;
        }
        path = path.parent()?.to_path_buf();
    }
}

fn slot_owner(bytes: &[u8]) -> Option<Uuid> {
    std::str::from_utf8(bytes).ok()?.parse().ok()
}

#[cfg(target_os = "linux")]
fn read_private(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file()
            && !meta.file_type().is_symlink()
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= limit,
        "browser recovery evidence is not a bounded private file"
    );
    Ok(fs::read(path)?)
}

/// A stale slot is released only after the browser's independent subreaper has
/// positively reported descendant and private-profile cleanup. A dead PID or
/// a stopped Voyage on its own is never sufficient.
#[cfg(target_os = "linux")]
fn reclaim_observed_slot(path: &Path, sessions: &Path) -> Result<bool> {
    use serde_json::{Value, json};
    let owner = match slot_owner(&read_private(path, 128)?) {
        Some(owner) if !owner.is_nil() => owner,
        _ => return Ok(false),
    };
    let session_dir = sessions.join(owner.to_string());
    if !session_dir.is_dir() {
        return Ok(false);
    }
    crate::attachment::journal::prepare_directory(session_dir.clone())?;
    let guardian =
        crate::attachment::journal::open_private_file(&session_dir.join("guardian.lock"))?;
    if guardian.try_lock().is_err() {
        return Ok(false);
    }
    let startup = crate::attachment::journal::open_private_file(&session_dir.join("startup.lock"))?;
    if startup.try_lock().is_err() {
        return Ok(false);
    }
    let execution = crate::attachment::journal::open_private_file(
        &session_dir
            .join("journal")
            .join(format!("{owner}.execution.lock")),
    )?;
    if execution.try_lock().is_err() {
        return Ok(false);
    }
    let registration: Value = serde_json::from_slice(&read_private(
        &session_dir.join("registration.json"),
        16_384,
    )?)?;
    ensure!(
        registration["session_id"] == owner.to_string(),
        "browser slot session identity mismatch"
    );
    let worker_dir = session_dir.join("journal/host-browser");
    crate::attachment::journal::prepare_directory(worker_dir.clone())?;
    let worker_path = worker_dir.join("worker.lock");
    let worker: Value = serde_json::from_slice(&read_private(&worker_path, 4096)?)?;
    let pid = worker["pid"]
        .as_u64()
        .context("browser worker lock lacks PID")?;
    ensure!(
        pid > 1 && pid <= i32::MAX as u64,
        "invalid browser worker PID"
    );
    if Path::new(&format!("/proc/{pid}")).exists() {
        return Ok(false);
    }
    let marker: Value = serde_json::from_slice(&read_private(
        &worker_dir.join("guardian-cleanup.json"),
        4096,
    )?)?;
    if ![
        "observed",
        "cleanup_complete",
        "descendants_terminated",
        "descendants_reaped",
        "temporary_cleaned",
    ]
    .iter()
    .all(|field| marker[*field] == true)
    {
        return Ok(false);
    }
    // Re-read while both session fences are held. A concurrent recovery must
    // not let an old owner remove a new reservation.
    ensure!(
        slot_owner(&read_private(path, 128)?) == Some(owner),
        "browser slot changed during recovery"
    );
    let recovery = Uuid::new_v4();
    let audit = session_dir.join(format!("browser-capacity-recovery-{recovery}.json"));
    let record = json!({"recovery_id":recovery,"session_id":owner,"worker_pid":pid,
        "reason":"automatic guardian-observed browser cleanup",
        "observed_no_descendants":true,"external_effects_reconciled":false,
        "observed_at_utc":chrono::Utc::now().to_rfc3339()});
    write_audit(&audit, &record)?;
    File::open(&session_dir)?.sync_all()?;
    // Releasing capacity precedes retiring the old worker lock. An interrupted
    // recovery can leave a locked old session, but cannot double-book a slot.
    fs::remove_file(path)?;
    File::open(path.parent().context("browser capacity root missing")?)?.sync_all()?;
    fs::rename(
        &worker_path,
        worker_dir.join(format!("worker.lock.recovered-{recovery}")),
    )?;
    File::open(&worker_dir)?.sync_all()?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn write_audit(path: &Path, record: &serde_json::Value) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options.open(path)?;
    file.write_all(serde_json::to_string_pretty(record)?.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

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
    let read_private = |path: &Path| read_private(path, 16_384);
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
    for index in 0..MAX_BROWSER_SLOTS {
        let path = root.join(format!("browser-slot-{index}"));
        match fs::symlink_metadata(&path) {
            Ok(_) if slot_owner(&read_private(&path)?) == Some(session) => {
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
    write_audit(&audit, &record)?;
    fs::rename(&worker_path, &retained)?;
    File::open(&worker_dir)?.sync_all()?;
    for (_, path) in &slots {
        ensure!(
            slot_owner(&read_private(path)?) == Some(session),
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
    #[cfg(target_os = "linux")]
    pub fn acquire_for_session(
        root: &Path,
        session_dir: &Path,
        owner: Uuid,
        slots: usize,
    ) -> Result<Self> {
        let sessions = session_dir.parent().context("session parent missing")?;
        match Self::acquire(root, owner, slots) {
            Ok(capacity) => return Ok(capacity),
            Err(error)
                if error.to_string()
                    != "browser capacity is in use or awaiting observed cleanup" =>
            {
                return Err(error);
            }
            Err(_) => {}
        }
        for index in 0..slots {
            let path = root.join(format!("browser-slot-{index}"));
            if path.exists() && reclaim_observed_slot(&path, sessions).unwrap_or(false) {
                return Self::acquire(root, owner, slots);
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
    #[cfg(target_os = "linux")]
    #[test]
    fn spare_memory_expands_admission_but_never_exceeds_bound() {
        const GIB: u64 = 1024 * 1024 * 1024;
        assert_eq!(slots_for_available_memory(0), 4);
        assert_eq!(slots_for_available_memory(5 * GIB), 4);
        assert_eq!(slots_for_available_memory(6 * GIB), 5);
        assert_eq!(slots_for_available_memory(40 * GIB), 16);
        assert_eq!(slots_for_available_memory(u64::MAX), 16);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn observed_guardian_cleanup_recovers_stale_slot_without_attesting_effects() {
        use serde_json::json;
        use std::os::unix::fs::PermissionsExt;
        let private_write = |path: &Path, value: String| {
            fs::write(path, value).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        };
        let tmp = tempfile::tempdir().unwrap();
        let sessions =
            crate::attachment::journal::prepare_directory(tmp.path().join("sessions")).unwrap();
        let old = Uuid::new_v4();
        let current = Uuid::new_v4();
        let old_dir =
            crate::attachment::journal::prepare_directory(sessions.join(old.to_string())).unwrap();
        let journal =
            crate::attachment::journal::prepare_directory(old_dir.join("journal")).unwrap();
        let worker_dir =
            crate::attachment::journal::prepare_directory(journal.join("host-browser")).unwrap();
        private_write(
            &old_dir.join("registration.json"),
            json!({"session_id":old}).to_string(),
        );
        private_write(
            &worker_dir.join("worker.lock"),
            json!({"pid":i32::MAX}).to_string(),
        );
        let capacity_root = tmp.path().join("capacity");
        let slot = Capacity::acquire(&capacity_root, old, 1).unwrap();
        drop(slot);
        assert!(
            Capacity::acquire_for_session(
                &capacity_root,
                &sessions.join(current.to_string()),
                current,
                1
            )
            .is_err()
        );
        private_write(
            &worker_dir.join("guardian-cleanup.json"),
            json!({"observed":true,
                "cleanup_complete":true,"descendants_terminated":true,
                "descendants_reaped":true,"temporary_cleaned":true})
            .to_string(),
        );
        let execution = crate::attachment::journal::open_private_file(
            &journal.join(format!("{old}.execution.lock")),
        )
        .unwrap();
        execution.try_lock().unwrap();
        assert!(
            Capacity::acquire_for_session(
                &capacity_root,
                &sessions.join(current.to_string()),
                current,
                1
            )
            .is_err()
        );
        drop(execution);
        let next = Capacity::acquire_for_session(
            &capacity_root,
            &sessions.join(current.to_string()),
            current,
            1,
        )
        .unwrap();
        assert_eq!(
            fs::read(&next.path).unwrap(),
            current.to_string().as_bytes()
        );
        assert!(!worker_dir.join("worker.lock").exists());
        assert_eq!(
            fs::read_dir(&old_dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("browser-capacity-recovery-"))
                .count(),
            1
        );
        next.release().unwrap();
    }
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
