//! Opt-in private measurement of the existing native WSS application boundary.
//! No frame content, credential, URL, command ID or routing input is retained.
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Default)]
struct Counts {
    route: Option<Uuid>,
    multiple_routes: bool,
    attempts: u64,
    connections: u64,
    active: u64,
    disconnected: u64,
    handshake_failures: u64,
    last_received: Option<Instant>,
    sent_bytes: u64,
    received_bytes: u64,
    sent_frames: u64,
    received_frames: u64,
    transport_failures: u64,
}
pub(super) struct Attempt {
    meter: Arc<Meter>,
    ready: bool,
}
impl Attempt {
    pub(super) fn connected(&mut self) {
        self.meter.connected();
        self.ready = true;
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.ready {
            let mut c = self.meter.counts.lock().unwrap();
            c.handshake_failures = c.handshake_failures.saturating_add(1);
            c.transport_failures = c.transport_failures.saturating_add(1);
        }
    }
}
pub(super) struct Meter {
    counts: Mutex<Counts>,
    file: Mutex<File>,
    path: PathBuf,
    label: String,
    pid: u32,
    start_ticks: u64,
    started: Instant,
}
impl Meter {
    fn open(path: &Path, label: String) -> Result<Self> {
        ensure!(path.is_absolute(), "qualification output must be absolute");
        ensure!(
            (1..=48).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b)),
            "invalid qualification label"
        );
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing qualification directory"))?;
        ensure!(
            parent.canonicalize()? == parent,
            "qualification directory changed"
        );
        let meta = std::fs::symlink_metadata(parent)?;
        ensure!(
            meta.is_dir() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
            "qualification directory must be owned and private"
        );
        let file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let pid = std::process::id();
        let raw = std::fs::read_to_string("/proc/self/stat")?;
        let start_ticks = raw
            .rsplit_once(')')
            .ok_or_else(|| anyhow::anyhow!("missing process identity"))?
            .1
            .split_whitespace()
            .nth(19)
            .ok_or_else(|| anyhow::anyhow!("missing start identity"))?
            .parse()?;
        Ok(Self {
            counts: Mutex::new(Counts::default()),
            file: Mutex::new(file),
            path: path.into(),
            label,
            pid,
            start_ticks,
            started: Instant::now(),
        })
    }
    pub(super) fn route(&self, id: Uuid) {
        let mut c = self.counts.lock().unwrap();
        match c.route {
            None => c.route = Some(id),
            Some(first) if first != id => c.multiple_routes = true,
            _ => (),
        }
    }
    pub(super) fn attempt(self: &Arc<Self>) -> Attempt {
        let mut c = self.counts.lock().unwrap();
        c.attempts = c.attempts.saturating_add(1);
        Attempt {
            meter: self.clone(),
            ready: false,
        }
    }
    fn connected(&self) {
        let mut c = self.counts.lock().unwrap();
        c.connections = c.connections.saturating_add(1);
        c.active = c.active.saturating_add(1);
        c.last_received = Some(Instant::now());
    }
    pub(super) fn sent(&self, length: usize) {
        let mut c = self.counts.lock().unwrap();
        c.sent_bytes = c.sent_bytes.saturating_add(length as u64);
        c.sent_frames = c.sent_frames.saturating_add(1);
    }
    pub(super) fn received(&self, length: usize) {
        let mut c = self.counts.lock().unwrap();
        c.received_bytes = c.received_bytes.saturating_add(length as u64);
        c.received_frames = c.received_frames.saturating_add(1);
    }
    pub(super) fn heard(&self) {
        self.counts.lock().unwrap().last_received = Some(Instant::now());
    }
    pub(super) fn retired(&self, failed: bool) {
        let mut c = self.counts.lock().unwrap();
        c.active = c.active.saturating_sub(1);
        c.disconnected = c.disconnected.saturating_add(1);
        if failed {
            c.transport_failures = c.transport_failures.saturating_add(1);
        }
    }
    fn snapshot(&self) -> Value {
        let c = self.counts.lock().unwrap();
        let expired = self.started.elapsed() >= Duration::from_secs(600);
        json!({"schema":1,"scope":"native WSS Text application bytes; excludes HTTP upgrade, TLS/TCP and control frames",
            "label":self.label,"pid":self.pid,"start_ticks":self.start_ticks,
            "captured_at_ms":SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "elapsed_ms":self.started.elapsed().as_millis(),"connections":c.connections,"routes":if c.multiple_routes {2} else {usize::from(c.route.is_some())},
            "attempts":c.attempts,"active":c.active,"disconnected":c.disconnected,"handshake_failures":c.handshake_failures,
            "source_age_ms":c.last_received.map(|at|at.elapsed().as_millis()),
            "sent_bytes":c.sent_bytes,"received_bytes":c.received_bytes,"sent_frames":c.sent_frames,"received_frames":c.received_frames,
            "transport_failures":c.transport_failures,
            "status":if expired {"expired"} else if c.transport_failures>0 || c.multiple_routes {"unknown"} else if c.active==0 {"disconnected"} else {"observed"}})
    }
    fn flush(&self) -> Result<()> {
        let value = serde_json::to_vec(&self.snapshot())?;
        ensure!(value.len() <= 4096, "qualification output bound");
        let mut file = self.file.lock().unwrap();
        let held = file.metadata()?;
        let named = std::fs::symlink_metadata(&self.path)?;
        ensure!(
            named.is_file()
                && held.dev() == named.dev()
                && held.ino() == named.ino()
                && held.nlink() == 1
                && held.uid() == unsafe { libc::geteuid() }
                && held.mode() & 0o077 == 0,
            "qualification output identity changed"
        );
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&value)?;
        file.set_len(value.len() as u64)?;
        file.flush()?;
        Ok(())
    }
}
pub(super) fn from_env(route: Uuid) -> Option<Arc<Meter>> {
    static METER: OnceLock<Option<Arc<Meter>>> = OnceLock::new();
    let meter = METER
        .get_or_init(|| {
            let path = std::env::var_os("HELM_QUALIFICATION_WSS_OUTPUT")?;
            let label = std::env::var("HELM_QUALIFICATION_WSS_LABEL").ok()?;
            let meter = Arc::new(Meter::open(Path::new(&path), label).ok()?);
            meter.flush().ok()?;
            let writer = meter.clone();
            let _writer = std::thread::Builder::new()
                .name("helm-byte-observer".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(Duration::from_millis(200));
                        if writer.flush().is_err()
                            || writer.started.elapsed() >= Duration::from_secs(600)
                        {
                            break;
                        }
                    }
                })
                .ok()?;
            Some(meter)
        })
        .clone()?;
    meter.route(route);
    Some(meter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn meter(root: &Path) -> Meter {
        std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
        Meter::open(&root.join("private-counts.json"), "native-a".into()).unwrap()
    }
    #[test]
    fn exact_text_bytes_have_no_content_and_unknown_transport_cannot_pass() {
        let root = tempfile::tempdir().unwrap();
        let m = meter(root.path());
        m.route(Uuid::new_v4());
        m.connected();
        let private = "synthetic credential and private 日本語";
        m.sent(private.len());
        m.received(123);
        m.flush().unwrap();
        let raw = std::fs::read(&m.path).unwrap();
        let v: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(v["sent_bytes"], private.len());
        assert_eq!(v["received_bytes"], 123);
        assert_eq!(v["sent_frames"], 1);
        assert_eq!(v["status"], "observed");
        assert!(!String::from_utf8(raw).unwrap().contains(private));
        m.retired(true);
        assert_eq!(m.snapshot()["status"], "unknown");
        assert_eq!(m.snapshot()["active"], 0);
    }
    #[test]
    fn another_route_is_unknown_and_private_output_substitution_refuses() {
        let root = tempfile::tempdir().unwrap();
        let m = meter(root.path());
        m.route(Uuid::new_v4());
        m.connected();
        m.route(Uuid::new_v4());
        assert_eq!(m.snapshot()["status"], "unknown");
        std::fs::rename(&m.path, root.path().join("retained.json")).unwrap();
        std::fs::write(&m.path, b"unrelated output").unwrap();
        assert!(m.flush().is_err());
        assert_eq!(std::fs::read(&m.path).unwrap(), b"unrelated output");
    }
    #[test]
    fn public_directory_existing_output_and_symlink_do_not_admit_a_meter() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(Meter::open(&root.path().join("counts.json"), "native-a".into()).is_err());
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(root.path().join("retained.json"), b"retained").unwrap();
        assert!(Meter::open(&root.path().join("retained.json"), "native-a".into()).is_err());
        std::os::unix::fs::symlink("retained.json", root.path().join("counts.json")).unwrap();
        assert!(Meter::open(&root.path().join("counts.json"), "native-a".into()).is_err());
        assert_eq!(
            std::fs::read(root.path().join("retained.json")).unwrap(),
            b"retained"
        );
    }
    #[test]
    fn clean_close_and_unsuccessful_reconnect_do_not_leave_historical_health() {
        let root = tempfile::tempdir().unwrap();
        let m = Arc::new(meter(root.path()));
        m.route(Uuid::new_v4());
        {
            let mut a = m.attempt();
            a.connected();
        }
        assert_eq!(m.snapshot()["active"], 1);
        assert_eq!(m.snapshot()["attempts"], 1);
        m.retired(false);
        assert_eq!(m.snapshot()["status"], "disconnected");
        assert_eq!(m.snapshot()["disconnected"], 1);
        {
            let _failed_reconnect = m.attempt();
        }
        assert_eq!(m.snapshot()["status"], "unknown");
        assert_eq!(m.snapshot()["handshake_failures"], 1);
        assert_eq!(m.snapshot()["active"], 0);
        assert_eq!(m.snapshot()["connections"], 1);
    }
}
