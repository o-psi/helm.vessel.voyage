//! Executing-host browser ownership. Neither worker stdout nor human input is history.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{Mutex, Semaphore, mpsc, oneshot},
};
use uuid::Uuid;
use voyage_protocol::host_browser::*;
const MAX_FRAME: usize = 4 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(25);
type Pending = Arc<std::sync::Mutex<HashMap<Uuid, oneshot::Sender<Value>>>>;

#[derive(Debug, thiserror::Error)]
#[error(
    "Browser action refused before interaction ({0}); inspect the page for fresh visible elements"
)]
pub(crate) struct BeforeEffectRefusal(&'static str);

fn worker_reply(reply: Value) -> Result<Value> {
    if reply["ok"] == true {
        return Ok(reply["result"].clone());
    }
    if reply["error"]["state"] == "refused" {
        let code = match reply["error"]["code"].as_str() {
            Some("stale_reference") => Some("stale_reference"),
            Some("element_hidden") => Some("element_hidden"),
            Some("element_disabled") => Some("element_disabled"),
            Some("element_not_editable") => Some("element_not_editable"),
            _ => None,
        };
        if let Some(code) = code {
            return Err(BeforeEffectRefusal(code).into());
        }
    }
    anyhow::bail!("browser operation refused or outcome unknown")
}

/// Loaded exclusively from executing-host configuration, never portable launch settings.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Launch {
    pub node: PathBuf,
    pub worker: PathBuf,
    pub chromium: PathBuf,
    #[serde(default = "default_config")]
    pub config: Value,
}
impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostBrowserLaunch").finish_non_exhaustive()
    }
}
fn default_config() -> Value {
    json!({"public_web":true,"origins":[],"ice_servers":[],"relay_only":false,"width":1280,"height":720})
}
impl Launch {
    pub fn discover() -> Option<Self> {
        let executable = std::env::current_exe().ok()?;
        let worker = executable
            .parent()?
            .join("../share/voyage/browser/worker.mjs")
            .canonicalize()
            .ok()?;
        let node = ["/usr/bin/node", "/usr/local/bin/node"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())?;
        let chromium = [
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/usr/bin/google-chrome",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())?;
        Some(Self {
            node,
            worker,
            chromium,
            config: default_config(),
        })
    }
}

fn validate_viewer_ice(config: &Value) -> Result<()> {
    let Some(value) = config.get("viewer_rtc_configuration") else {
        return Ok(());
    };
    let object = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("invalid viewer ICE configuration"))?;
    ensure!(
        object
            .keys()
            .all(|k| matches!(k.as_str(), "iceServers" | "iceTransportPolicy")),
        "invalid viewer ICE field"
    );
    ensure!(
        value
            .get("iceTransportPolicy")
            .is_none_or(|p| p == "all" || p == "relay"),
        "invalid viewer ICE policy"
    );
    let servers = value["iceServers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("viewer ICE servers required"))?;
    ensure!(servers.len() <= 4, "viewer ICE server bound");
    for server in servers {
        let fields = server
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("invalid ICE server"))?;
        ensure!(
            fields
                .keys()
                .all(|k| matches!(k.as_str(), "urls" | "username" | "credential")),
            "invalid ICE server field"
        );
        let urls = server["urls"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("ICE URLs required"))?;
        ensure!(!urls.is_empty() && urls.len() <= 4, "ICE URL bound");
        for url in urls {
            let url = url
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid ICE URL"))?;
            ensure!(
                url.len() <= 2048
                    && (url.starts_with("turn:") || url.starts_with("turns:"))
                    && !url.chars().any(|c| c.is_whitespace() || c.is_control()),
                "invalid ICE URL"
            );
        }
        for key in ["username", "credential"] {
            ensure!(
                server[key]
                    .as_str()
                    .is_some_and(|v| v.len() <= 4096 && !v.chars().any(char::is_control)),
                "invalid ICE credential"
            );
        }
    }
    ensure!(
        value["iceTransportPolicy"] != "relay" || !servers.is_empty(),
        "relay-only viewer needs TURN"
    );
    Ok(())
}

/// Pipe ownership is independent of a caller's cancellation. An interrupt command
/// can be written while a prior ordinary operation awaits its reply.
struct Worker {
    writer: mpsc::Sender<Vec<u8>>,
    pending: Pending,
    capacity: Arc<Semaphore>,
    failed: Arc<AtomicBool>,
    child: Mutex<tokio::process::Child>,
    temporary: PathBuf,
    cleanup_marker: PathBuf,
    _reader: tokio::task::JoinHandle<()>,
    _writer: tokio::task::JoinHandle<()>,
}
struct ExchangeGuard {
    failed: Arc<AtomicBool>,
    complete: bool,
}
impl Drop for ExchangeGuard {
    fn drop(&mut self) {
        if !self.complete {
            self.failed.store(true, Ordering::Release);
        }
    }
}
impl Worker {
    fn spawn(launch: &Launch, root: &std::path::Path) -> Result<Arc<Self>> {
        ensure!(
            launch.node.is_absolute()
                && launch.worker.is_absolute()
                && launch.chromium.is_absolute(),
            "browser distribution paths must be absolute"
        );
        ensure!(
            launch.node.is_file() && launch.worker.is_file() && launch.chromium.is_file(),
            "browser distribution unavailable"
        );
        // Chromium uses Unix sockets beneath TMPDIR; session journal paths can
        // exceed sockaddr_un. Own a short private directory until observed cleanup.
        let temporary = tempfile::Builder::new()
            .prefix("vhb-")
            .tempdir_in("/tmp")?
            .keep();
        let guardian = launch.worker.with_file_name("guardian.py");
        ensure!(
            guardian.is_file(),
            "browser guardian distribution unavailable"
        );
        let mut command = tokio::process::Command::new("/usr/bin/python3");
        command
            .arg(&guardian)
            .arg(&launch.node)
            .arg(&launch.worker)
            .arg(&temporary)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root)
            .env("TMPDIR", &temporary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        // The worker and descendants have one owned group. Forced termination is
        // not reported as observed browser cleanup (worker must acknowledge it).
        #[cfg(unix)]
        {
            command.process_group(0);
            unsafe {
                command.pre_exec(|| {
                    // Bound individual files/descriptors and forbid private core dumps.
                    // Aggregate memory/disk containment still needs the host guardian.
                    for (resource, value) in [
                        (libc::RLIMIT_FSIZE, 64 * 1024 * 1024),
                        (libc::RLIMIT_NOFILE, 1024),
                        (libc::RLIMIT_CORE, 0),
                    ] {
                        let limit = libc::rlimit {
                            rlim_cur: value,
                            rlim_max: value,
                        };
                        if libc::setrlimit(resource, &limit) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                    libc::umask(0o077);
                    Ok(())
                });
            }
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(_) => {
                std::fs::remove_dir_all(&temporary)?;
                anyhow::bail!("browser worker unavailable");
            }
        };
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("browser pipe missing"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("browser pipe missing"))?;
        let pending: Pending = Arc::new(std::sync::Mutex::new(HashMap::new()));
        let failed = Arc::new(AtomicBool::new(false));
        let (writer, mut rx) = mpsc::channel::<Vec<u8>>(16);
        let failure = failed.clone();
        let waiting = pending.clone();
        let writer_task = tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if input.write_all(&bytes).await.is_err() || input.flush().await.is_err() {
                    break;
                }
            }
            failure.store(true, Ordering::Release);
            waiting.lock().unwrap().clear();
        });
        let waiting = pending.clone();
        let failure = failed.clone();
        let reader = tokio::spawn(async move {
            let mut output = BufReader::new(output);
            loop {
                let mut bytes = Vec::new();
                let frame = async {
                    loop {
                        let available = output.fill_buf().await?;
                        ensure!(!available.is_empty(), "browser pipe ended");
                        let n = available
                            .iter()
                            .position(|b| *b == b'\n')
                            .map(|n| n + 1)
                            .unwrap_or(available.len());
                        ensure!(bytes.len() + n <= MAX_FRAME, "browser frame bound");
                        let done = available[n - 1] == b'\n';
                        bytes.extend_from_slice(&available[..n]);
                        output.consume(n);
                        if done {
                            break;
                        }
                    }
                    let value: Value = serde_json::from_slice(&bytes)?;
                    let id: Uuid = serde_json::from_value(value["id"].clone())?;
                    ensure!(value["ok"].is_boolean(), "browser reply malformed");
                    let tx = waiting
                        .lock()
                        .unwrap()
                        .remove(&id)
                        .ok_or_else(|| anyhow::anyhow!("browser reply unknown"))?;
                    let _ = tx.send(value);
                    Ok::<_, anyhow::Error>(())
                }
                .await;
                if frame.is_err() {
                    break;
                }
            }
            failure.store(true, Ordering::Release);
            waiting.lock().unwrap().clear();
        });
        Ok(Arc::new(Self {
            writer,
            pending,
            capacity: Arc::new(Semaphore::new(12)),
            failed,
            child: Mutex::new(child),
            temporary,
            cleanup_marker: root.join("guardian-cleanup.json"),
            _reader: reader,
            _writer: writer_task,
        }))
    }
    async fn exchange(&self, request: Value) -> Result<Value> {
        ensure!(
            !self.failed.load(Ordering::Acquire),
            "browser transport unresolved"
        );
        // Reserve four slots for control/disconnect even when agent work is pending.
        let interrupt = matches!(
            request["op"].as_str(),
            Some("control" | "disconnect" | "shutdown" | "close")
        ) || (request["op"] == "input"
            && (request["action"]["kind"] == "dialog"
                || (request["action"]["kind"] == "history"
                    && request["action"]["direction"] == "stop")));
        let _permit = if interrupt {
            None
        } else {
            Some(
                self.capacity
                    .clone()
                    .try_acquire_owned()
                    .map_err(|_| anyhow::anyhow!("browser capacity busy"))?,
            )
        };
        let id: Uuid = serde_json::from_value(request["id"].clone())?;
        let mut bytes = serde_json::to_vec(&request)?;
        ensure!(bytes.len() < MAX_FRAME, "browser frame bound");
        bytes.push(b'\n');
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            ensure!(
                pending.len() < 16 && !pending.contains_key(&id),
                "browser pending capacity or identity conflict"
            );
            pending.insert(id, tx);
        }
        if self.writer.try_send(bytes).is_err() {
            self.pending.lock().unwrap().remove(&id);
            anyhow::bail!("browser queue full; request not dispatched");
        }
        let mut guard = ExchangeGuard {
            failed: self.failed.clone(),
            complete: false,
        };
        let received = tokio::time::timeout(DEADLINE, rx).await;
        guard.complete = true;
        match received {
            Ok(Ok(reply)) => worker_reply(reply),
            _ => {
                self.failed.store(true, Ordering::Release);
                anyhow::bail!("browser outcome unknown; never replay")
            }
        }
    }
    async fn stop(&self) -> Result<()> {
        let graceful = self
            .exchange(json!({"id":Uuid::new_v4(),"op":"shutdown"}))
            .await
            .is_ok();
        // EOF is an independent authority fence for the guardian, including when
        // the worker transport was poisoned and cannot accept shutdown.
        self._writer.abort();
        let mut child = self.child.lock().await;
        let mut exited = matches!(
            tokio::time::timeout(Duration::from_secs(12), child.wait()).await,
            Ok(Ok(_))
        );
        if !exited {
            #[cfg(unix)]
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            exited = matches!(
                tokio::time::timeout(Duration::from_secs(12), child.wait()).await,
                Ok(Ok(_))
            );
        }
        ensure!(exited, "browser guardian cleanup unconfirmed");
        let metadata = std::fs::symlink_metadata(&self.cleanup_marker)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
                "unsafe guardian evidence"
            );
        }
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink() && metadata.len() <= 4096,
            "invalid guardian evidence"
        );
        let evidence: Value = serde_json::from_slice(&std::fs::read(&self.cleanup_marker)?)?;
        ensure!(
            evidence["observed"] == true,
            "browser descendant cleanup unconfirmed"
        );
        ensure!(
            !self.temporary.exists(),
            "browser private profile cleanup unconfirmed"
        );
        // A forced teardown resolves resource ownership, not website success.
        if !graceful {
            self.failed.store(true, Ordering::Release);
        }
        Ok(())
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        // Closing the forwarding pipe wakes the independent subreaper guardian.
        // Killing the guardian here could strand descendants in other groups.
        self._reader.abort();
        self._writer.abort();
    }
}

struct Viewer {
    signal_sequence: u64,
    input_sequence: u64,
    socket: Uuid,
    principal: Uuid,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    seen: std::time::Instant,
}
/// Bounded conservative tombstone set. False positives refuse, never dispatch.
/// Retired identities cannot become fresh input after receipt compaction.
struct RetiredIds {
    bits: Vec<u64>,
}
impl Default for RetiredIds {
    fn default() -> Self {
        Self {
            bits: vec![0; 262144],
        }
    }
}
impl RetiredIds {
    fn indices(id: Uuid) -> [usize; 4] {
        let bytes = Sha256::digest(id.as_bytes());
        std::array::from_fn(|i| {
            u32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()) as usize % (262144 * 64)
        })
    }
    fn insert(&mut self, id: Uuid) {
        for n in Self::indices(id) {
            self.bits[n / 64] |= 1u64 << (n % 64);
        }
    }
    fn contains(&self, id: Uuid) -> bool {
        Self::indices(id)
            .iter()
            .all(|n| self.bits[n / 64] & (1u64 << (n % 64)) != 0)
    }
}

struct Inner {
    worker: Option<Arc<Worker>>,
    reservation: Option<crate::host_resources::Reservation>,
    capacity: Option<crate::host_browser_capacity::Capacity>,
    status: Value,
    viewers: HashMap<Uuid, Viewer>,
    disconnected: HashSet<Uuid>,
    private_owner: Option<(Uuid, Uuid)>,
    live_receipts: HashMap<Uuid, (Uuid, Uuid, String, String)>,
    input_receipts: HashSet<Uuid>,
    retired_input_ids: RetiredIds,
    starting: bool,
}
pub struct HostBrowser {
    directory: PathBuf,
    session: Uuid,
    incarnation: Uuid,
    launch: Option<Launch>,
    inner: Mutex<Inner>,
    /// Any human fence invalidates an agent result even before worker acknowledgement.
    fence: AtomicU64,
    agent_gate: Mutex<()>,
    lifecycle: Mutex<()>,
}
impl std::fmt::Debug for HostBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostBrowser")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}
impl HostBrowser {
    pub fn new(
        directory: PathBuf,
        session: Uuid,
        incarnation: Uuid,
        launch: Option<Launch>,
    ) -> Arc<Self> {
        Arc::new(Self {
            directory,
            session,
            incarnation,
            launch,
            inner: Mutex::new(Inner {
                worker: None,
                reservation: None,
                capacity: None,
                status: Value::Null,
                viewers: HashMap::new(),
                disconnected: HashSet::new(),
                private_owner: None,
                live_receipts: HashMap::new(),
                input_receipts: HashSet::new(),
                retired_input_ids: RetiredIds::default(),
                starting: false,
            }),
            fence: AtomicU64::new(0),
            agent_gate: Mutex::new(()),
            lifecycle: Mutex::new(()),
        })
    }
    pub async fn blocks_suspension(&self) -> bool {
        let inner = self.inner.lock().await;
        inner.worker.is_some() || inner.reservation.is_some() || inner.starting
    }
    fn root(&self) -> Result<PathBuf> {
        crate::attachment::journal::prepare_directory(self.directory.join("host-browser"))
    }
    async fn start(&self) -> Result<Arc<Worker>> {
        let _lifecycle = self.lifecycle.lock().await;
        {
            let inner = self.inner.lock().await;
            if let Some(worker) = &inner.worker {
                ensure!(
                    !worker.failed.load(Ordering::Acquire),
                    "browser transport unresolved; close required"
                );
                ensure!(!inner.status.is_null(), "browser startup unresolved");
                return Ok(worker.clone());
            }
        }
        let launch = self.launch.as_ref().ok_or_else(|| {
            anyhow::anyhow!("host browser unavailable; install packaged worker, Node and Chromium")
        })?;
        validate_viewer_ice(&launch.config)?;
        let root = self.root()?;
        // Only this exclusive Voyage owner may retire its own prior guardian
        // evidence before a new launch. A worker lock still refuses uncertain reuse.
        let marker = root.join("guardian-cleanup.json");
        if marker.exists() {
            std::fs::remove_file(&marker)?;
        }
        // Reservations survive failed launch/cleanup, including process death.
        let capacity = crate::host_browser_capacity::Capacity::acquire(
            &crate::config::default_data_dir().join("host-browser-capacity"),
            self.session,
            4,
        )?;
        let reservation =
            match crate::host_resources::Reservation::acquire("executors", self.session, 3) {
                Ok(r) => r,
                Err(error) => {
                    capacity.release()?;
                    return Err(error);
                }
            };
        {
            let mut inner = self.inner.lock().await;
            inner.capacity = Some(capacity);
            inner.reservation = Some(reservation);
            inner.starting = true;
        }
        let worker = match Worker::spawn(launch, &root) {
            Ok(w) => w,
            Err(e) => {
                let mut i = self.inner.lock().await;
                i.starting = false;
                if let Some(r) = &i.reservation {
                    r.release_observed()?;
                }
                i.reservation = None;
                if let Some(capacity) = i.capacity.take() {
                    capacity.release()?;
                }
                return Err(e);
            }
        };
        {
            let mut inner = self.inner.lock().await;
            inner.worker = Some(worker.clone());
            inner.starting = false;
        }
        let mut config = launch.config.clone();
        config["root"] = json!(root);
        config["executable"] = json!(launch.chromium);
        worker
            .exchange(json!({"id":Uuid::new_v4(),"op":"init","config":config}))
            .await?;
        let status = worker
            .exchange(json!({"id":Uuid::new_v4(),"op":"open"}))
            .await?;
        self.update_status(&status).await;
        if self.inner.lock().await.status.is_null() {
            self.refresh(&worker).await?;
        }
        Ok(worker)
    }
    async fn update_status(&self, result: &Value) {
        let status = if result["status"].is_object() {
            &result["status"]
        } else {
            result
        };
        if status["browser"].is_string() && status["epochs"].is_object() {
            let mut inner = self.inner.lock().await;
            let old = inner.status["epochs"]["control"].as_u64().unwrap_or(0);
            let new = status["epochs"]["control"].as_u64().unwrap_or(0);
            if new >= old
                && ["document", "viewport", "capture", "tab"]
                    .iter()
                    .all(|key| {
                        status["epochs"][*key].as_u64().unwrap_or(0)
                            >= inner.status["epochs"][*key].as_u64().unwrap_or(0)
                    })
            {
                inner.status = status.clone();
            }
        }
    }
    async fn refresh(&self, worker: &Worker) -> Result<Value> {
        let value = worker
            .exchange(json!({"id":Uuid::new_v4(),"op":"status"}))
            .await?;
        self.update_status(&value).await;
        Ok(if value["status"].is_object() {
            value["status"].clone()
        } else {
            value
        })
    }
    pub async fn close(&self) -> Result<()> {
        self.fence.fetch_add(1, Ordering::AcqRel);
        let _lifecycle = self.lifecycle.lock().await;
        let worker = self.inner.lock().await.worker.clone();
        if let Some(worker) = worker {
            worker.stop().await?;
        }
        let mut inner = self.inner.lock().await;
        if let Some(r) = &inner.reservation {
            r.release_observed()?;
        }
        if let Some(capacity) = inner.capacity.take() {
            capacity.release()?;
        }
        inner.worker = None;
        inner.reservation = None;
        inner.status = Value::Null;
        inner.viewers.clear();
        let retired: Vec<_> = inner.live_receipts.keys().copied().collect();
        for id in retired {
            inner.retired_input_ids.insert(id);
        }
        inner.live_receipts.clear();
        inner.input_receipts.clear();
        inner.private_owner = None;
        Ok(())
    }
    fn binding(&self, status: &Value, attachment: Uuid) -> Result<HostBrowserBinding> {
        Ok(HostBrowserBinding {
            incarnation: self.incarnation,
            browser_id: serde_json::from_value(status["browser"].clone())?,
            attachment_id: attachment,
            tab_id: serde_json::from_value(status["tab"].clone())?,
            document_epoch: status["epochs"]["document"].as_u64().unwrap_or(0),
            viewport_epoch: status["epochs"]["viewport"].as_u64().unwrap_or(0),
            controller_epoch: status["epochs"]["control"].as_u64().unwrap_or(0),
            capture_epoch: status["epochs"]["capture"].as_u64().unwrap_or(0),
        })
    }
    async fn projection(&self, attachment: Uuid) -> Value {
        let i = self.inner.lock().await;
        let private = i.status["mode"] == "private";
        let controller = i.status["controller"]
            .as_str()
            .is_some_and(|id| id == attachment.to_string());
        let disclose = !private || (controller && i.viewers.contains_key(&attachment));
        json!({"available":self.launch.is_some(),"running":i.status["open"].as_bool().unwrap_or(false),"binding":self.binding(&i.status,attachment).ok(),"mode":i.status["mode"],"controller":i.status["controller"],"tabs":if disclose {i.status["tabs"].clone()} else {json!([])},"agent_active":i.status["agent_active"].as_bool().unwrap_or(false),"agent_action":if disclose {i.status["agent_action"].clone()} else {Value::Null},"agent_cursor":if disclose {i.status["agent_cursor"].clone()} else {Value::Null},"page":if disclose {i.status["page"].clone()} else {Value::Null},"tab_details":if disclose {i.status["tab_details"].clone()} else {json!([])},"dialog":if disclose {i.status["dialog"].clone()} else {Value::Null},"input_sequence":i.viewers.get(&attachment).map(|v| v.input_sequence).unwrap_or(0)})
    }
    // No payloads, URLs, SDP, private input or observations enter durable receipts.
    fn receipt(
        &self,
        id: Uuid,
        principal: Uuid,
        digest: Option<&str>,
        state: Option<&str>,
    ) -> Result<Option<Value>> {
        use rusqlite::{OptionalExtension, params};
        let path = self.root()?.join("receipts.sqlite3");
        let _file = crate::attachment::journal::open_private_file(&path)?;
        let mut db = rusqlite::Connection::open(&path)?;
        db.busy_timeout(Duration::from_secs(1))?;
        db.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS receipts(id TEXT PRIMARY KEY,principal TEXT NOT NULL,digest TEXT NOT NULL,state TEXT NOT NULL);")?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let prior: Option<(String, String, String)> = tx
            .query_row(
                "SELECT principal,digest,state FROM receipts WHERE id=?1",
                [id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((owner, hash, outcome)) = prior {
            ensure!(
                owner == principal.to_string() && digest.is_none_or(|d| d == hash),
                "browser command identity conflict"
            );
            if let Some(state) = state {
                tx.execute(
                    "UPDATE receipts SET state=?2 WHERE id=?1",
                    params![id.to_string(), state],
                )?;
            }
            tx.commit()?;
            return Ok(Some(
                json!({"command_id":id,"state":if outcome == "dispatched" { "unknown" } else { &outcome },"content_withheld":true}),
            ));
        }
        if let Some(hash) = digest {
            let count: u64 = tx.query_row("SELECT COUNT(*) FROM receipts", [], |r| r.get(0))?;
            ensure!(
                count < 100_000,
                "browser durable receipt capacity exhausted"
            );
            tx.execute(
                "INSERT INTO receipts VALUES(?1,?2,?3,'dispatched')",
                params![id.to_string(), principal.to_string(), hash],
            )?;
        }
        tx.commit()?;
        Ok(None)
    }
    pub async fn human(
        &self,
        operation: HostBrowserOperation,
        socket: Uuid,
        principal: Uuid,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    ) -> Result<Value> {
        ensure!(
            operation.valid() && !socket.is_nil() && !principal.is_nil(),
            "invalid browser operation"
        );
        if let Some(a) = &authority {
            a.check()?;
        }
        {
            let mut i = self.inner.lock().await;
            ensure!(
                !i.disconnected.contains(&socket),
                "browser socket disconnected"
            );
            for v in i
                .viewers
                .values_mut()
                .filter(|v| v.socket == socket && v.principal == principal)
            {
                v.seen = std::time::Instant::now();
            }
        }
        if let HostBrowserOperation::Receipt { command_id } = operation {
            if let Some((owner, _, _, state)) =
                self.inner.lock().await.live_receipts.get(&command_id)
            {
                ensure!(*owner == principal, "browser receipt principal mismatch");
                return Ok(
                    json!({"receipt":{"command_id":command_id,"state":state,"content_withheld":true}}),
                );
            }
            return Ok(json!({"receipt":self.receipt(command_id,principal,None,None)?}));
        }
        if matches!(operation, HostBrowserOperation::Status {}) {
            let worker = self.inner.lock().await.worker.clone();
            if let Some(worker) = worker {
                ensure!(
                    !worker.failed.load(Ordering::Acquire),
                    "browser transport unresolved"
                );
            }
            let active_worker = { self.inner.lock().await.worker.clone() };
            if let Some(worker) = active_worker {
                self.refresh(&worker).await?;
            }
            let attachment = self
                .inner
                .lock()
                .await
                .viewers
                .iter()
                .find(|(_, v)| v.socket == socket && v.principal == principal)
                .map(|(id, _)| *id)
                .unwrap_or(Uuid::nil());
            return Ok(json!({"status":self.projection(attachment).await}));
        }
        let id = operation
            .mutation_id()
            .ok_or_else(|| anyhow::anyhow!("missing browser mutation"))?;
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(
                &json!({"operation":operation,"socket":socket,"principal":principal})
            )?)
        );
        let ephemeral = matches!(
            &operation,
            HostBrowserOperation::Input { .. } | HostBrowserOperation::Signal { .. }
        );
        if ephemeral {
            let mut inner = self.inner.lock().await;
            ensure!(
                !inner.retired_input_ids.contains(id),
                "browser input identity retired; never replay"
            );
            if let Some((owner, bound, hash, state)) = inner.live_receipts.get(&id) {
                ensure!(
                    *owner == principal && *bound == socket && *hash == digest,
                    "browser live receipt identity conflict"
                );
                return Ok(
                    json!({"receipt":{"command_id":id,"state":state,"content_withheld":true}}),
                );
            }
            ensure!(
                inner.live_receipts.len() < 8192,
                "browser in-flight receipt capacity exhausted"
            );
            if inner.live_receipts.len() >= 4096 {
                let retired: Vec<_> = inner
                    .live_receipts
                    .iter()
                    .filter(|(id, r)| r.3 != "dispatched" && inner.input_receipts.contains(id))
                    .take(1024)
                    .map(|(id, _)| *id)
                    .collect();
                for id in retired {
                    inner.retired_input_ids.insert(id);
                    inner.live_receipts.remove(&id);
                    inner.input_receipts.remove(&id);
                }
            }
            if matches!(operation, HostBrowserOperation::Input { .. }) {
                inner.input_receipts.insert(id);
            }
            inner
                .live_receipts
                .insert(id, (principal, socket, digest.clone(), "dispatched".into()));
        } else if let Some(prior) = self.receipt(id, principal, Some(&digest), None)? {
            return Ok(json!({"receipt":prior,"content_withheld":true}));
        }
        let result = self
            .human_effect(operation, socket, principal, authority)
            .await;
        let outcome = if result.is_ok() {
            "completed"
        } else {
            "unknown"
        };
        if ephemeral {
            if let Some(receipt) = self.inner.lock().await.live_receipts.get_mut(&id) {
                receipt.3 = outcome.into();
            }
        } else {
            self.receipt(id, principal, Some(&digest), Some(outcome))?;
        }
        result
    }
    async fn human_effect(
        &self,
        operation: HostBrowserOperation,
        socket: Uuid,
        principal: Uuid,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    ) -> Result<Value> {
        if let HostBrowserOperation::Start { incarnation, .. } = operation {
            ensure!(incarnation == self.incarnation, "stale browser incarnation");
            self.start().await?;
            return Ok(json!({"status":self.projection(Uuid::nil()).await}));
        }
        let binding = operation
            .binding()
            .ok_or_else(|| anyhow::anyhow!("browser binding required"))?
            .clone();
        let worker = self
            .inner
            .lock()
            .await
            .worker
            .clone()
            .ok_or_else(|| anyhow::anyhow!("browser not running"))?;
        let status = self.inner.lock().await.status.clone();
        let mut media_binding = binding.clone();
        let live_binding = self.binding(&status, binding.attachment_id)?;
        // Navigation/resize may overlap media setup. They do not change viewer
        // authority or the capture stream. Input retains every exact fence.
        if matches!(
            operation,
            HostBrowserOperation::Attach { .. }
                | HostBrowserOperation::Signal { .. }
                | HostBrowserOperation::Detach { .. }
        ) {
            media_binding.document_epoch = live_binding.document_epoch;
            media_binding.viewport_epoch = live_binding.viewport_epoch;
        }
        ensure!(media_binding == live_binding, "stale browser binding");
        let attach = matches!(operation, HostBrowserOperation::Attach { .. });
        let attachment = if attach {
            ensure!(
                !binding.attachment_id.is_nil(),
                "attach requires fresh attachment identity"
            );
            let mut i = self.inner.lock().await;
            ensure!(
                !i.disconnected.contains(&socket) && i.viewers.len() < 4,
                "browser viewer admission unavailable"
            );
            ensure!(
                !i.viewers.values().any(|v| v.socket == socket),
                "socket already attached"
            );
            let id = if status["mode"] == "private" {
                let (id, owner) = i
                    .private_owner
                    .ok_or_else(|| anyhow::anyhow!("private owner unavailable"))?;
                ensure!(
                    owner == principal,
                    "private browser belongs to another principal"
                );
                id
            } else {
                binding.attachment_id
            };
            ensure!(!i.viewers.contains_key(&id), "attachment already live");
            // Own before dispatch, including uncertain join.
            i.viewers.insert(
                id,
                Viewer {
                    signal_sequence: 0,
                    input_sequence: 0,
                    socket,
                    principal,
                    authority: authority.clone(),
                    seen: std::time::Instant::now(),
                },
            );
            id
        } else {
            let i = self.inner.lock().await;
            let viewer = i
                .viewers
                .get(&binding.attachment_id)
                .ok_or_else(|| anyhow::anyhow!("browser attachment missing"))?;
            ensure!(
                viewer.socket == socket
                    && viewer.principal == principal
                    && !i.disconnected.contains(&socket),
                "browser attachment authority mismatch"
            );
            binding.attachment_id
        };
        if let Some(a) = &authority {
            a.check()?;
        }
        if matches!(&operation, HostBrowserOperation::Control { .. })
            && let Some(viewer) = self.inner.lock().await.viewers.get_mut(&attachment)
        {
            viewer.authority = authority;
        }
        let mut request = json!({"id":operation.mutation_id(),"browser":status["browser"],"epochs":status["epochs"],"viewer":attachment});
        let detach = matches!(operation, HostBrowserOperation::Detach { .. });
        let request_offer = matches!(
            &operation,
            HostBrowserOperation::Signal {
                signal: HostBrowserSignal::RequestOffer {},
                ..
            }
        );
        if matches!(operation, HostBrowserOperation::Signal { .. }) {
            let mut inner = self.inner.lock().await;
            let viewer = inner
                .viewers
                .get_mut(&attachment)
                .ok_or_else(|| anyhow::anyhow!("browser viewer missing"))?;
            viewer.signal_sequence = viewer
                .signal_sequence
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("browser signal sequence exhausted"))?;
            request["signal_seq"] = json!(viewer.signal_sequence);
        }
        match operation {
            HostBrowserOperation::Attach { .. } => request["op"] = json!("join"),
            HostBrowserOperation::Detach { .. } => {
                if status["mode"] == "private" && status["controller"] == json!(attachment) {
                    self.inner.lock().await.private_owner = Some((attachment, principal));
                }
                self.fence.fetch_add(1, Ordering::AcqRel);
                request["op"] = json!("disconnect");
            }
            HostBrowserOperation::Control { mode, .. } => {
                self.fence.fetch_add(1, Ordering::AcqRel);
                request["op"] = json!("control");
                request["mode"] = serde_json::to_value(mode)?;
            }
            HostBrowserOperation::Signal { signal, .. } => match signal {
                HostBrowserSignal::RequestOffer {} => request["op"] = json!("offer"),
                HostBrowserSignal::Answer { sdp } => {
                    request["op"] = json!("answer");
                    request["description"] = json!({"type":"answer","sdp":sdp});
                }
                HostBrowserSignal::Ice { .. } => {
                    anyhow::bail!("complete ICE negotiation required; trickle unsupported")
                }
            },
            HostBrowserOperation::Input {
                sequence, input, ..
            } => {
                {
                    let mut inner = self.inner.lock().await;
                    let viewer = inner
                        .viewers
                        .get_mut(&attachment)
                        .ok_or_else(|| anyhow::anyhow!("browser viewer missing"))?;
                    ensure!(
                        sequence
                            == viewer
                                .input_sequence
                                .checked_add(1)
                                .ok_or_else(|| anyhow::anyhow!("input sequence exhausted"))?,
                        "browser input sequence mismatch"
                    );
                    viewer.input_sequence = sequence;
                }
                request["op"] = json!("input");
                request["seq"] = json!(sequence);
                request["action"] = input_action(input)?;
            }
            HostBrowserOperation::Close { .. } => {
                self.close().await?;
                return Ok(json!({"status":self.projection(Uuid::nil()).await}));
            }
            _ => anyhow::bail!("invalid browser operation"),
        }
        let value = worker.exchange(request).await?;
        self.update_status(&value).await;
        if detach {
            let mut inner = self.inner.lock().await;
            inner.viewers.remove(&attachment);
            let retired: Vec<_> = inner
                .live_receipts
                .iter()
                .filter(|(_, (_, bound, _, _))| *bound == socket)
                .map(|(id, _)| *id)
                .collect();
            for id in retired {
                inner.retired_input_ids.insert(id);
                inner.live_receipts.remove(&id);
            }
            let kept: HashSet<_> = inner.live_receipts.keys().copied().collect();
            inner.input_receipts.retain(|id| kept.contains(id));
        }
        // Worker effects carry current status; do not queue a status request behind an in-flight agent.

        // Socket loss or grant withdrawal can occur while SDP/inputs are pending.
        {
            let i = self.inner.lock().await;
            ensure!(
                !i.disconnected.contains(&socket),
                "browser result withheld after disconnect"
            );
            if let Some(v) = i.viewers.get(&attachment)
                && let Some(a) = &v.authority
            {
                a.check()?;
            }
        }
        let mut output = value.get("value").unwrap_or(&value).clone();
        if request_offer {
            // Explicit host-provided viewer ICE configuration is private signaling,
            // never status/history. Do not disclose encoder-only TURN credentials.
            if let Some(config) = self
                .launch
                .as_ref()
                .and_then(|l| l.config.get("viewer_rtc_configuration"))
            {
                output["rtc_configuration"] = config.clone();
            }
        }
        Ok(
            json!({"status":self.projection(if detach {Uuid::nil()}else{attachment}).await,"value":output}),
        )
    }
    pub async fn disconnect(&self, socket: Uuid) -> Result<()> {
        self.fence.fetch_add(1, Ordering::AcqRel);
        let (worker, viewers) = {
            let mut i = self.inner.lock().await;
            ensure!(
                i.disconnected.len() < 100_000 || i.disconnected.contains(&socket),
                "browser disconnected socket capacity exhausted"
            );
            i.disconnected.insert(socket);
            let retired: Vec<_> = i
                .live_receipts
                .iter()
                .filter(|(_, (_, bound, _, _))| *bound == socket)
                .map(|(id, _)| *id)
                .collect();
            for id in retired {
                i.retired_input_ids.insert(id);
                i.live_receipts.remove(&id);
            }
            let kept: HashSet<_> = i.live_receipts.keys().copied().collect();
            i.input_receipts.retain(|id| kept.contains(id));
            if i.status["mode"] == "private"
                && let Ok(controller) =
                    serde_json::from_value::<Uuid>(i.status["controller"].clone())
                && let Some(v) = i.viewers.get(&controller)
            {
                i.private_owner = Some((controller, v.principal));
            }
            (
                i.worker.clone(),
                i.viewers
                    .iter()
                    .filter(|(_, v)| v.socket == socket)
                    .map(|(id, _)| *id)
                    .collect::<Vec<_>>(),
            )
        };
        if let Some(worker) = worker {
            for viewer in viewers {
                let status = self.inner.lock().await.status.clone();
                let reply=worker.exchange(json!({"id":Uuid::new_v4(),"op":"disconnect","browser":status["browser"],"epochs":status["epochs"],"viewer":viewer})).await?;
                self.update_status(&reply).await;
                self.inner.lock().await.viewers.remove(&viewer);
            }
        }
        Ok(())
    }
    /// Revocation and lost gateway notification are independently fail-closed.
    /// Viewer status polling renews a bounded lease; no socket liveness inference.
    pub async fn maintain(&self) -> Result<()> {
        let expired = {
            let i = self.inner.lock().await;
            i.viewers
                .values()
                .filter(|v| {
                    v.seen.elapsed() > Duration::from_secs(20)
                        || v.authority.as_ref().is_some_and(|a| a.check().is_err())
                })
                .map(|v| v.socket)
                .collect::<Vec<_>>()
        };
        for socket in expired {
            if self.disconnect(socket).await.is_err() {
                self.close().await?;
            }
        }
        Ok(())
    }
    pub async fn agent(
        &self,
        id: Uuid,
        action: Value,
        context: &crate::tools::ToolContext,
    ) -> Result<Value> {
        let _agent = self.agent_gate.lock().await;
        context.policy.check_execution_authority()?;
        ensure!(
            !context.cancellation.is_cancelled(),
            "browser call cancelled"
        );
        let worker = self.start().await?;
        let status = self.refresh(&worker).await?;
        ensure!(
            status["mode"] == "agent",
            "browser controlled by human or private operator"
        );
        let stamp = self.fence.load(Ordering::Acquire);
        context.policy.check_execution_authority()?;
        let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&action)?));
        if let Some(receipt) = self.receipt(id, self.session, Some(&digest), None)? {
            return Ok(json!({"receipt":receipt,"content_withheld":true}));
        }
        let request = json!({"id":id,"op":"agent","browser":status["browser"],"epochs":status["epochs"],"action":action});
        let result = tokio::select! {biased; _=context.cancellation.cancelled()=>Err(anyhow::anyhow!("browser outcome unknown after cancellation")),result=worker.exchange(request)=>result};
        self.receipt(
            id,
            self.session,
            Some(&digest),
            Some(
                if result
                    .as_ref()
                    .is_err_and(|e| e.is::<BeforeEffectRefusal>())
                {
                    "refused"
                } else if result.is_ok() {
                    "completed"
                } else {
                    "unknown"
                },
            ),
        )?;
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                if error.is::<BeforeEffectRefusal>() {
                    return Err(error);
                }
                worker.failed.store(true, Ordering::Release);
                let _ = self.close().await;
                return Err(error);
            }
        };
        context.policy.check_execution_authority()?;
        ensure!(
            stamp == self.fence.load(Ordering::Acquire) && !context.cancellation.is_cancelled(),
            "browser observation withheld after fence"
        );
        self.update_status(&value).await;
        Ok(value.get("value").unwrap_or(&value).clone())
    }
    /// Run completion does not close the human's shared session browser. It fences
    /// abandoned work by closing only when transport outcome is unresolved.
    pub async fn finish_run(&self) -> Result<()> {
        let failed = self.inner.lock().await.worker.as_ref().is_some_and(|w| {
            w.failed.load(Ordering::Acquire) || !w.pending.lock().unwrap().is_empty()
        });
        if failed {
            self.close().await?;
        }
        Ok(())
    }
}
fn input_action(input: HostBrowserInput) -> Result<Value> {
    Ok(match input {
        HostBrowserInput::History { direction } => json!({"kind":"history","direction":direction}),
        HostBrowserInput::Pointer {
            x,
            y,
            button,
            pressed,
        } => {
            json!({"kind":"pointer","type":if button.is_none(){"move"}else if pressed{"down"}else{"up"},"x":x,"y":y,"button":button.unwrap_or(HostBrowserButton::Left)})
        }
        HostBrowserInput::Key { key, pressed } => {
            json!({"kind":"key","type":if pressed{"down"}else{"up"},"key":key})
        }
        HostBrowserInput::Text { text } => json!({"kind":"text","text":text}),
        HostBrowserInput::Scroll { delta_x, delta_y } => {
            json!({"kind":"scroll","x":delta_x,"y":delta_y})
        }
        HostBrowserInput::Resize { width, height } => {
            json!({"kind":"resize","width":width,"height":height})
        }
        HostBrowserInput::Dialog { accept, text } => {
            json!({"kind":"dialog","accept":accept,"text":text})
        }
        HostBrowserInput::Navigate { url } => json!({"kind":"navigate","url":url}),
        HostBrowserInput::Tab { operation, tab_id } => {
            json!({"kind":"tabs","operation":operation,"tab":tab_id})
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_proven_pre_effect_element_refusals_are_recoverable() {
        for code in [
            "stale_reference",
            "element_hidden",
            "element_disabled",
            "element_not_editable",
        ] {
            let error = worker_reply(json!({"ok":false,"error":{"state":"refused","code":code}}))
                .unwrap_err();
            assert!(error.is::<BeforeEffectRefusal>());
            for state in ["unknown", "dispatched", "completed"] {
                let error = worker_reply(json!({"ok":false,"error":{"state":state,"code":code}}))
                    .unwrap_err();
                assert!(!error.is::<BeforeEffectRefusal>());
            }
        }
        let error = worker_reply(
            json!({"ok":false,"error":{"state":"refused","code":"arbitrary private text"}}),
        )
        .unwrap_err();
        assert!(!error.is::<BeforeEffectRefusal>());
        assert!(!error.to_string().contains("private text"));
    }
    #[tokio::test]
    async fn unavailable_does_not_allocate() {
        let dir = tempfile::tempdir().unwrap();
        let browser = HostBrowser::new(dir.path().into(), Uuid::new_v4(), Uuid::new_v4(), None);
        assert!(browser.start().await.is_err());
        assert!(!browser.blocks_suspension().await);
        browser.close().await.unwrap();
    }
    #[test]
    fn durable_identity_never_replays() {
        let dir = tempfile::tempdir().unwrap();
        let b = HostBrowser::new(dir.path().into(), Uuid::new_v4(), Uuid::new_v4(), None);
        let id = Uuid::new_v4();
        let p = Uuid::new_v4();
        assert!(b.receipt(id, p, Some("abc"), None).unwrap().is_none());
        assert_eq!(
            b.receipt(id, p, Some("abc"), None).unwrap().unwrap()["state"],
            "unknown"
        );
        assert!(b.receipt(id, p, Some("changed"), None).is_err());
        assert!(b.receipt(id, Uuid::new_v4(), None, None).is_err());
    }
    #[tokio::test]
    async fn projection_binds_active_tab_and_ignores_old_fences() {
        let dir = tempfile::tempdir().unwrap();
        let browser = HostBrowser::new(dir.path().into(), Uuid::new_v4(), Uuid::new_v4(), None);
        let id = Uuid::new_v4();
        let tab = Uuid::new_v4();
        let attachment = Uuid::new_v4();
        let newer = json!({"status":{"browser":id,"tab":tab,"tabs":[tab],"open":true,"mode":"private","epochs":{"tab":1,"document":1,"viewport":1,"control":3,"capture":3}}});
        browser.update_status(&newer).await;
        let mut older = newer.clone();
        older["status"]["mode"] = json!("agent");
        older["status"]["epochs"]["control"] = json!(2);
        browser.update_status(&older).await;
        let projected = browser.projection(attachment).await;
        assert_eq!(projected["mode"], "private");
        assert_eq!(projected["binding"]["tab_id"], json!(tab));
        assert_eq!(projected["binding"]["attachment_id"], json!(attachment));
    }
    #[test]
    fn cancelled_exchange_poison_is_sticky() {
        let failed = Arc::new(AtomicBool::new(false));
        drop(ExchangeGuard {
            failed: failed.clone(),
            complete: false,
        });
        assert!(failed.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn disconnect_tombstone_refuses_late_status() {
        let dir = tempfile::tempdir().unwrap();
        let b = HostBrowser::new(dir.path().into(), Uuid::new_v4(), Uuid::new_v4(), None);
        let socket = Uuid::new_v4();
        b.disconnect(socket).await.unwrap();
        assert!(
            b.human(
                HostBrowserOperation::Status {},
                socket,
                Uuid::new_v4(),
                None
            )
            .await
            .is_err()
        );
    }
}

#[cfg(test)]
mod launch_privacy_tests {
    use super::*;
    #[test]
    fn diagnostic_configuration_withholds_turn_credentials() {
        let launch = Launch {
            node: "/usr/bin/node".into(),
            worker: "/private/worker.mjs".into(),
            chromium: "/usr/bin/chromium".into(),
            config: json!({"ice_servers":[{"credential":"turn-secret-sentinel"}]}),
        };
        assert!(!format!("{launch:?}").contains("turn-secret-sentinel"));
        let config = crate::Config {
            host_browser_launch: Some(launch),
            ..Default::default()
        };
        let display = config.diagnostic_toml().unwrap();
        assert!(!display.contains("turn-secret-sentinel"));
        assert!(!display.contains("/private/worker"));
    }
}

#[cfg(test)]
mod retired_input_tests {
    use super::*;
    #[test]
    fn bounded_tombstones_never_forget_inserted_ids() {
        let mut ids = RetiredIds::default();
        let inserted: Vec<_> = (0..12000).map(|_| Uuid::new_v4()).collect();
        for id in &inserted {
            ids.insert(*id);
        }
        for id in inserted {
            assert!(ids.contains(id));
        }
        assert_eq!(ids.bits.len() * 8, 2 * 1024 * 1024);
    }
}

#[cfg(test)]
mod viewer_metadata_tests {
    use super::*;
    #[tokio::test]
    async fn private_page_metadata_is_visible_only_to_attached_controller() {
        let root = tempfile::tempdir().unwrap();
        let manager = HostBrowser::new(root.path().into(), Uuid::new_v4(), Uuid::new_v4(), None);
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        let tab = Uuid::new_v4();
        {
            let mut inner = manager.inner.lock().await;
            inner.status = json!({"browser":Uuid::new_v4(),"tab":tab,"tabs":[tab],"open":true,"mode":"private","controller":owner,"page":{"title":"PRIVATE TITLE","url":"https://private.invalid/secret"},"tab_details":[{"id":tab,"title":"PRIVATE TITLE"}],"dialog":{"message":"PRIVATE DIALOG"},"epochs":{"document":1,"viewport":1,"control":1,"capture":1}});
            inner.viewers.insert(
                owner,
                Viewer {
                    signal_sequence: 0,
                    input_sequence: 0,
                    socket: Uuid::new_v4(),
                    principal: Uuid::new_v4(),
                    authority: None,
                    seen: std::time::Instant::now(),
                },
            );
        }
        for id in [other, Uuid::nil()] {
            let projection = manager.projection(id).await;
            assert!(projection["page"].is_null());
            assert!(projection["dialog"].is_null());
            assert_eq!(projection["tab_details"], json!([]));
            assert!(!projection.to_string().contains("PRIVATE"));
        }
        assert_eq!(
            manager.projection(owner).await["page"]["title"],
            "PRIVATE TITLE"
        );
        manager.inner.lock().await.viewers.remove(&owner);
        assert!(manager.projection(owner).await["page"].is_null());
    }
}
