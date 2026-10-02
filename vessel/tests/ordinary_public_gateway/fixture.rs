//! Owned ordinary CLI/public-router fixtures. Runtime IPC peers are scripted
//! readonly responders, never executing Voyages or evidence of native ownership.
use super::VESSEL;
use reqwest::{Client, Response, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    net::SocketAddr,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::{net::UnixListener, sync::oneshot, task::JoinHandle};
use uuid::Uuid;
use voyage_protocol::{
    notifications::NotificationOperation,
    process::{
        ApprovedWorkspace, ConnectionGrant, GrantBinding, LocalAccessCredential, PROCESS_PROTOCOL,
        ProcessRegistration, ProcessRight, ProcessState, RuntimeCommand, RuntimeRequest,
        RuntimeResponse, read_frame, write_frame,
    },
    vessel::{
        COMMAND_PATH, EVENTS_PATH, VESSEL_API_VERSION, VesselCommand, VesselEvent,
        VesselEventRequest, VesselEventSubscription, VesselRequest, VesselResponse, VoyageCommand,
        VoyageRequest,
    },
};

pub const ORIGIN: &str = "https://owned-gateway.invalid";
pub const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
pub const RUNTIME_TOKEN: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
pub const OPERATOR: &str = "synthetic-operator-only";
pub const WAIT: Duration = Duration::from_secs(5);
const LOG_BYTES: u64 = 1024 * 1024;

pub fn now() -> u64 {
    chrono::Utc::now().timestamp_millis().try_into().unwrap()
}
pub fn private_directory(path: &Path) {
    if !path.exists() {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .unwrap();
    }
    let metadata = fs::symlink_metadata(path).unwrap();
    assert!(metadata.is_dir() && !metadata.file_type().is_symlink());
    assert_eq!(metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(metadata.mode() & 0o077, 0);
}
fn file(path: &Path) -> File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .unwrap()
}
pub fn write_new(path: &Path, value: &impl serde::Serialize) {
    let mut output = file(path);
    output
        .write_all(&serde_json::to_vec(value).unwrap())
        .unwrap();
    output.sync_all().unwrap();
}
pub fn read_private(path: &Path, bound: u64) -> Vec<u8> {
    use std::io::Read;
    let input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .unwrap();
    let m = input.metadata().unwrap();
    assert!(m.is_file() && m.nlink() == 1 && m.len() <= bound);
    assert_eq!(m.uid(), unsafe { libc::geteuid() });
    assert_eq!(m.mode() & 0o077, 0);
    let mut bytes = Vec::new();
    input.take(bound + 1).read_to_end(&mut bytes).unwrap();
    assert!(bytes.len() as u64 <= bound);
    bytes
}
pub fn replace_owned(path: &Path, value: &impl serde::Serialize) {
    let _old = read_private(path, 16384);
    let temporary = path.with_extension(format!("fixture-{}", Uuid::new_v4()));
    write_new(&temporary, value);
    fs::rename(&temporary, path).unwrap();
    File::open(path.parent().unwrap())
        .unwrap()
        .sync_all()
        .unwrap();
}
fn start(pid: u32) -> Option<String> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

pub struct ChildOwner {
    child: Option<Child>,
    pid: u32,
    ticks: String,
    image: PathBuf,
    image_inode: (u64, u64),
    logs: [PathBuf; 2],
}
impl ChildOwner {
    pub fn spawn(env: &Environment, label: &str, args: &[String]) -> Self {
        assert_ne!(unsafe { libc::geteuid() }, 0, "ordinary fixtures only");
        let image = fs::canonicalize(VESSEL).expect("current instrumented Vessel entrypoint");
        let metadata = fs::metadata(&image).unwrap();
        let logs = [
            env.root.join(format!("{label}.stdout-private.log")),
            env.root.join(format!("{label}.stderr-private.log")),
        ];
        let mut command = Command::new(&image);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", env.root.join("h"))
            .env("XDG_CONFIG_HOME", env.root.join("c"))
            .env("XDG_DATA_HOME", env.root.join("d"))
            .env("XDG_STATE_HOME", env.root.join("s"))
            .env("XDG_CACHE_HOME", env.root.join("k"))
            .env("XDG_RUNTIME_DIR", env.root.join("r"))
            .env("VOYAGE_CREDENTIAL_KEY_FILE", env.key.join("key"))
            .env("TOKIO_WORKER_THREADS", "2")
            .env("RUST_BACKTRACE", "0")
            .current_dir(&env.root)
            .stdin(Stdio::null())
            .stdout(file(&logs[0]))
            .stderr(file(&logs[1]))
            .args(args);
        // The actual cargo-llvm-cov namespace must cover this real CLI child.
        // No provider, human account, general environment or wrapper is inherited.
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let child = command.spawn().unwrap();
        let pid = child.id();
        let ticks = start(pid).expect("owned child start identity");
        let owner = Self {
            child: Some(child),
            pid,
            ticks,
            image,
            image_inode: (metadata.dev(), metadata.ino()),
            logs,
        };
        assert_eq!(
            fs::metadata(format!("/proc/{pid}")).unwrap().uid(),
            unsafe { libc::geteuid() }
        );
        owner
    }
    pub fn check(&self) {
        assert_eq!(start(self.pid).as_ref(), Some(&self.ticks));
        assert_eq!(
            fs::metadata(format!("/proc/{}", self.pid)).unwrap().uid(),
            unsafe { libc::geteuid() }
        );
        assert_eq!(
            fs::read_link(format!("/proc/{}/exe", self.pid)).unwrap(),
            self.image
        );
        let m = fs::metadata(format!("/proc/{}/exe", self.pid)).unwrap();
        assert_eq!((m.dev(), m.ino()), self.image_inode);
        for path in &self.logs {
            assert!(fs::metadata(path).unwrap().len() <= LOG_BYTES);
        }
    }
    pub fn is_running(&mut self) -> bool {
        self.child.as_mut().unwrap().try_wait().unwrap().is_none()
    }
    pub fn owns_listener(&self, address: SocketAddr) {
        self.check();
        assert!(address.ip().is_loopback() && address.is_ipv4());
        let expected = format!("0100007F:{:04X}", address.port());
        let table = fs::read_to_string(format!("/proc/{}/net/tcp", self.pid)).unwrap();
        let sockets: Vec<_> = table
            .lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<_> = line.split_whitespace().collect();
                if fields.get(1) == Some(&expected.as_str()) && fields.get(3) == Some(&"0A") {
                    Some(format!("socket:[{}]", fields[9]))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(sockets.len(), 1, "one expected loopback listener");
        let owns = fs::read_dir(format!("/proc/{}/fd", self.pid))
            .unwrap()
            .any(|entry| {
                entry
                    .ok()
                    .and_then(|entry| fs::read_link(entry.path()).ok())
                    .is_some_and(|path| path.as_os_str() == std::ffi::OsStr::new(&sockets[0]))
            });
        assert!(owns, "listener must belong to this exact ordinary child");
    }
    async fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + Duration::from_secs(12);
        let status = loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                break status;
            }
            // The unreaped std::process::Child owns this PID even for a CLI
            // that exits before /proc/exe can be observed. No signal is issued
            // while waiting; persistent service signals still require check().
            for path in &self.logs {
                assert!(fs::metadata(path).unwrap().len() <= LOG_BYTES);
            }
            assert!(Instant::now() < deadline, "owned child retirement deadline");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        self.child.take();
        assert!(!Path::new(&format!("/proc/{}", self.pid)).exists());
        for path in &self.logs {
            assert!(fs::metadata(path).unwrap().len() <= LOG_BYTES);
        }
        status
    }
    pub async fn terminate(&mut self) {
        self.check();
        assert_eq!(unsafe { libc::kill(self.pid as i32, libc::SIGTERM) }, 0);
        assert!(
            self.wait().await.success(),
            "owned service exited unsuccessfully"
        );
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        // Failure cancellation is best effort and is not called observed cleanup.
        if let Some(mut child) = self.child.take()
            && start(self.pid).as_ref() == Some(&self.ticks)
            && fs::read_link(format!("/proc/{}/exe", self.pid))
                .ok()
                .as_ref()
                == Some(&self.image)
        {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct Environment {
    pub root: PathBuf,
    pub directory: PathBuf,
    pub workspace: PathBuf,
    key: PathBuf,
    cli_sequence: usize,
    completed: bool,
}
impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}
impl Environment {
    pub fn new() -> Self {
        assert_ne!(unsafe { libc::geteuid() }, 0, "ordinary fixtures only");
        // Short paths leave room for the production SESSION/runtime.sock name.
        let root = std::env::temp_dir().join(format!("vp-{}", Uuid::new_v4()));
        private_directory(&root);
        let root = fs::canonicalize(root).unwrap();
        for name in [
            "h",
            "c",
            "d",
            "d/helm",
            "s",
            "k",
            "r",
            "v",
            "v/sessions",
            "w",
        ] {
            private_directory(&root.join(name));
        }
        let key = PathBuf::from("/dev/shm").join(format!("vpg-key-{}", Uuid::new_v4()));
        private_directory(&key);
        let mut output = file(&key.join("key"));
        output.write_all(&[7; 32]).unwrap();
        output.sync_all().unwrap();
        Self {
            directory: root.join("v"),
            workspace: root.join("w"),
            root,
            key,
            cli_sequence: 0,
            completed: false,
        }
    }
    pub async fn cli(&mut self, args: &[String], success: bool) -> Vec<u8> {
        let label = format!("cli-{}", self.cli_sequence);
        self.cli_sequence += 1;
        let mut child = ChildOwner::spawn(self, &label, args);
        let status = child.wait().await;
        assert_eq!(
            status.success(),
            success,
            "fixed fixture CLI exit predicate"
        );
        read_private(&child.logs[0], LOG_BYTES)
    }
    pub fn assert_logs_private(&self, extra: &[&str]) {
        for entry in fs::read_dir(&self.root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("log") {
                continue;
            }
            let bytes = read_private(&path, LOG_BYTES);
            for secret in [TOKEN, RUNTIME_TOKEN, OPERATOR]
                .into_iter()
                .chain(extra.iter().copied())
            {
                assert!(
                    !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
                    "private fixture input appeared in child diagnostics"
                );
            }
        }
    }
    pub fn complete(&mut self) {
        self.assert_logs_private(&[]);
        self.completed = true;
    }
}
impl Drop for Environment {
    fn drop(&mut self) {
        // Runtime-key material is synthetic and is always retired. Failed
        // private logs/state remain for diagnosis; only success removes state.
        let _ = fs::remove_dir_all(&self.key);
        if self.completed {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    None,
    WrongSession,
    Malformed,
    LostReply,
    Refused,
    Unknown,
}
pub struct RuntimeState {
    pub calls: Vec<Value>,
    pub cursor: u64,
    pub fault: Fault,
}
pub struct RuntimePeer {
    pub registration: ProcessRegistration,
    pub state: Arc<Mutex<RuntimeState>>,
    path: PathBuf,
    stop: Option<oneshot::Sender<()>>,
    job: Option<JoinHandle<()>>,
}
impl RuntimePeer {
    async fn new(env: &Environment) -> Self {
        let registration = ProcessRegistration {
            protocol: PROCESS_PROTOCOL,
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: None,
            initialize: None,
            config_path: None,
            token: RUNTIME_TOKEN.into(),
            peer_uids: None,
            workspace: env.workspace.clone(),
            state: ProcessState::Live,
            name: Some("Synthetic readonly IPC peer".into()),
            executable: None,
        };
        let directory = env
            .directory
            .join("sessions")
            .join(registration.session_id.to_string());
        private_directory(&directory);
        write_new(&directory.join("registration.json"), &registration);
        let path = directory.join("runtime.sock");
        assert!(path.as_os_str().len() < 108);
        let listener = UnixListener::bind(&path).unwrap();
        let state = Arc::new(Mutex::new(RuntimeState {
            calls: Vec::new(),
            cursor: 11,
            fault: Fault::None,
        }));
        let captured = state.clone();
        let info = registration.clone();
        let (stop, mut retired) = oneshot::channel();
        let job = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _=&mut retired=>break,
                    Some(result)=children.join_next(),if !children.is_empty()=>result.unwrap(),
                    accept=listener.accept()=>{
                        let (mut stream,_)=accept.unwrap();
                        let info=info.clone();let state=captured.clone();
                        assert!(children.len()<128,"bounded owned IPC requests");
                        children.spawn(async move {
                            let request:RuntimeRequest=tokio::time::timeout(WAIT,read_frame(&mut stream)).await.unwrap().unwrap();
                            assert_eq!(request.protocol,PROCESS_PROTOCOL);
                            assert_eq!(request.session_id,info.session_id);
                            assert_eq!(request.incarnation,info.incarnation);
                            assert!(request.token==RUNTIME_TOKEN,"exact private fixture runtime token");
                            assert!(request.scope_authority.is_none(),"ordinary peer, no protected authority");
                            let command=serde_json::to_value(&request.command).unwrap();
                            let fault;
                            let result={
                                let mut state=state.lock().unwrap();assert!(state.calls.len()<8192);
                                // Only safe metadata/identity is retained, never token bytes.
                                state.calls.push(json!({"command":command,"authorization":request.authorization}));
                                fault=if matches!(request.command,RuntimeCommand::Health){Fault::None}else{state.fault};
                                match &request.command {
                                    RuntimeCommand::Health=>json!({"status":"ok"}),
                                    RuntimeCommand::Events{after,limit,wait_ms,projection}=>{
                                        assert_eq!(*limit,128);assert_eq!(*wait_ms,0);
                                        assert_eq!(projection.as_deref(),Some("public-v2"));
                                        let events=if *after<state.cursor {vec![json!({"session_id":info.session_id,"cursor":state.cursor,"revision":17,"kind":"session","payload":{"name":"Public fixture metadata"}})]}else{vec![]};
                                        json!({"projection":"public-v2","cursor":state.cursor,"latest_cursor":state.cursor,"has_more":false,"replay_gap":false,"events":events})
                                    }
                                    RuntimeCommand::Snapshot=>json!({"session_id":info.session_id,"revision":17,"model":"fixture","messages":[],"decisions":[],"total_messages":0}),
                                    RuntimeCommand::History{offset,limit,expected_revision}=>json!({"offset":offset,"limit":limit,"revision":expected_revision,"messages":[{"message_index":offset,"role":"user","content":"Canonical synthetic history Ω"}]}),
                                    RuntimeCommand::Receipt{command_id}=>json!({"command_id":command_id,"status":"unknown"}),
                                    RuntimeCommand::Controls{run_id,section}=>json!({"run_id":run_id,"section":section,"value":[]}),
                                    _=>panic!("no mutation, inference, terminal, tool or execution is permitted on a fixture IPC peer"),
                                }
                            };
                            if fault==Fault::LostReply{return;}
                            if fault==Fault::Malformed{
                                write_frame(&mut stream,&json!({"invalid":"not a RuntimeResponse"})).await.unwrap();return;
                            }
                            let response=RuntimeResponse{
                                protocol:PROCESS_PROTOCOL,
                                session_id:if fault==Fault::WrongSession{Uuid::new_v4()}else{info.session_id},
                                incarnation:info.incarnation,resumed_from:None,result,
                                error:(fault==Fault::Refused).then(||"synthetic readonly refusal".into()),
                                outcome_unknown:fault==Fault::Unknown,
                            };
                            tokio::time::timeout(WAIT,write_frame(&mut stream,&response)).await.unwrap().unwrap();
                        });
                    }
                }
            }
            drop(listener);
            while let Some(result) = children.join_next().await {
                result.unwrap();
            }
        });
        Self {
            registration,
            state,
            path,
            stop: Some(stop),
            job: Some(job),
        }
    }
    pub fn calls(&self, op: &str) -> Vec<Value> {
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| c["command"]["op"] == op)
            .cloned()
            .collect()
    }
    pub async fn finish(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        tokio::time::timeout(WAIT, self.job.take().unwrap())
            .await
            .unwrap()
            .unwrap();
        fs::remove_file(&self.path).unwrap();
        assert!(!self.path.exists());
        assert!(tokio::net::UnixStream::connect(&self.path).await.is_err());
    }
}
impl Drop for RuntimePeer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(job) = self.job.take() {
            job.abort();
        }
    }
}

pub struct Fixture {
    pub env: Environment,
    pub peers: Vec<RuntimePeer>,
    pub grant: ConnectionGrant,
    pub endpoint: String,
    pub address: SocketAddr,
    pub client: Client,
    pub local: LocalAccessCredential,
    service: Option<ChildOwner>,
    gateway: Option<ChildOwner>,
}
impl Fixture {
    pub async fn new(count: usize, operator: bool) -> Self {
        let env = Environment::new();
        let mut peers = Vec::new();
        for _ in 0..count {
            peers.push(RuntimePeer::new(&env).await);
        }
        let args = vec![
            "local-serve".into(),
            "--directory".into(),
            env.directory.to_string_lossy().into_owned(),
            "--voyage-binary".into(),
            env.root
                .join("no-voyage-execution")
                .to_string_lossy()
                .into_owned(),
        ];
        let mut service = ChildOwner::spawn(&env, "local-serve", &args);
        let local = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(
                    service.is_running(),
                    "ordinary local supervisor did not become ready; private logs retained"
                );
                service.check();
                if env.directory.join("process-http.json").exists() {
                    break serde_json::from_slice::<LocalAccessCredential>(&read_private(
                        &env.directory.join("process-http.json"),
                        4096,
                    ))
                    .unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let local_address: SocketAddr = local
            .endpoint
            .strip_prefix("http://")
            .unwrap()
            .parse()
            .unwrap();
        service.owns_listener(local_address);
        let caps: VesselResponse = client
            .post(format!("{}{}", local.endpoint, COMMAND_PATH))
            .bearer_auth(&local.token)
            .json(&VesselRequest {
                protocol: VESSEL_API_VERSION,
                command: VesselCommand::Capabilities,
            })
            .timeout(WAIT)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(caps.error.is_none() && !caps.outcome_unknown);
        let vessel_id = Uuid::parse_str(caps.result["vessel_id"].as_str().unwrap()).unwrap();
        let grant = ConnectionGrant {
            full_access: false,
            schema_version: 1,
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            vessel_id,
            revision: 1,
            rights: vec![
                ProcessRight::Catalogue,
                ProcessRight::Observe,
                ProcessRight::History,
            ],
            accounts: vec![],
            enrollment_connections: vec![],
            expires_at_ms: now() + 600_000,
            revoked: false,
            token_hash: format!("{:x}", Sha256::digest(TOKEN.as_bytes())),
            workspaces: vec![ApprovedWorkspace {
                id: Uuid::new_v4(),
                path: env.workspace.clone(),
                name: "Owned synthetic workspace".into(),
                provider_ready: None,
            }],
        };
        private_directory(&env.directory.join("access/connections"));
        write_new(
            &env.directory
                .join("access/connections")
                .join(format!("{}.json", grant.grant_id)),
            &grant,
        );
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        drop(reservation);
        let mut args = vec![
            "--bind".into(),
            address.to_string(),
            "--database".into(),
            env.root
                .join("gateway.sqlite3")
                .to_string_lossy()
                .into_owned(),
            "--process-directory".into(),
            env.directory.to_string_lossy().into_owned(),
            "--public-origin".into(),
            ORIGIN.into(),
        ];
        if operator {
            args.extend(["--operator-token".into(), OPERATOR.into()]);
        }
        let mut gateway = ChildOwner::spawn(&env, "gateway", &args);
        let endpoint = format!("http://{address}");
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                assert!(
                    gateway.is_running(),
                    "actual gateway CLI failed; private logs retained"
                );
                gateway.check();
                if let Ok(response) = client
                    .get(format!("{endpoint}/health"))
                    .timeout(Duration::from_millis(300))
                    .send()
                    .await
                    && response.status() == StatusCode::OK
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        gateway.owns_listener(address);
        Self {
            env,
            peers,
            grant,
            endpoint,
            address,
            client,
            local,
            service: Some(service),
            gateway: Some(gateway),
        }
    }
    pub fn grant_path(&self) -> PathBuf {
        self.env
            .directory
            .join("access/connections")
            .join(format!("{}.json", self.grant.grant_id))
    }
    pub fn change_grant(&mut self, change: impl FnOnce(&mut ConnectionGrant)) {
        change(&mut self.grant);
        replace_owned(&self.grant_path(), &self.grant);
    }
    pub fn authorized(&self, path: &str) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}{path}", self.endpoint))
            .bearer_auth(TOKEN)
            .header("x-voyage-grant", self.grant.grant_id.to_string())
            .header("x-voyage-vessel", self.grant.vessel_id.to_string())
            .header("Origin", ORIGIN)
    }
    pub async fn response(&self, command: VesselCommand) -> VesselResponse {
        let response = self
            .authorized(COMMAND_PATH)
            .json(&VesselRequest {
                protocol: VESSEL_API_VERSION,
                command,
            })
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        security(&response);
        let bytes = response.bytes().await.unwrap();
        assert!(bytes.len() <= 1024 * 1024);
        serde_json::from_slice(&bytes).unwrap()
    }
    pub async fn voyage(&self, index: usize, command: VoyageCommand) -> VesselResponse {
        let r = &self.peers[index].registration;
        let incarnation = command.requires_incarnation().then_some(r.incarnation);
        self.response(VesselCommand::Voyage(VoyageRequest {
            session_id: r.session_id,
            incarnation,
            command,
        }))
        .await
    }
    pub fn subscription(&self, index: usize, after: u64) -> VesselEventSubscription {
        let r = &self.peers[index].registration;
        VesselEventSubscription {
            session_id: r.session_id,
            incarnation: r.incarnation,
            after,
            projection: Some("public-v2".into()),
        }
    }
    pub async fn sse(&self, subscriptions: Vec<VesselEventSubscription>) -> Response {
        let response = self
            .authorized(EVENTS_PATH)
            .json(&VesselEventRequest {
                protocol: VESSEL_API_VERSION,
                subscriptions,
            })
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .unwrap();
        security(&response);
        response
    }
    pub async fn pair_invitation(&mut self) -> Value {
        let output = self
            .env
            .root
            .join(format!("invite-{}.json", self.env.cli_sequence));
        let args = vec![
            "pair-invite".into(),
            "--directory".into(),
            self.env.directory.to_string_lossy().into_owned(),
            "--endpoint".into(),
            ORIGIN.into(),
            "--principal".into(),
            Uuid::new_v4().to_string(),
            "--workspace".into(),
            self.env.workspace.to_string_lossy().into_owned(),
            "--rights".into(),
            "catalogue,observe,history".into(),
            "--output".into(),
            output.to_string_lossy().into_owned(),
            "--ttl-seconds".into(),
            "30".into(),
        ];
        self.env.cli(&args, true).await;
        serde_json::from_slice(&read_private(&output, 4096)).unwrap()
    }
    pub fn pairing_request(invitation: &Value, command: Uuid) -> Value {
        json!({"protocol":1,"command_id":command,"principal_id":invitation["principal_id"],"invitation_id":invitation["invitation_id"],"code":invitation["code"]})
    }
    pub async fn pair(&self, request: &Value) -> VesselResponse {
        let response = self
            .client
            .post(format!(
                "{}{}",
                self.endpoint,
                voyage_protocol::vessel::PAIR_PATH
            ))
            .header("Origin", ORIGIN)
            .header("x-voyage-vessel", self.grant.vessel_id.to_string())
            .json(request)
            .timeout(WAIT)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        security(&response);
        let bytes = response.bytes().await.unwrap();
        assert!(bytes.len() <= 16384);
        serde_json::from_slice(&bytes).unwrap()
    }
    pub fn assert_no_effects(&self) {
        assert!(!self.env.root.join("no-voyage-execution").exists());
        let db = rusqlite::Connection::open_with_flags(
            self.env.directory.join("catalogue.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let count: i64 = db
            .query_row("SELECT count(*) FROM lifecycle_commands", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "no execution command may be recorded by readonly public requests"
        );
        for peer in &self.peers {
            for call in &peer.state.lock().unwrap().calls {
                assert!(matches!(
                    call["command"]["op"].as_str(),
                    Some("health" | "snapshot" | "events" | "history" | "receipt" | "controls")
                ));
            }
        }
    }
    pub async fn stop_service(&mut self) {
        self.service.as_mut().unwrap().terminate().await;
        assert!(!self.env.directory.join("process-http.json").exists());
        assert!(
            self.client
                .post(format!("{}{}", self.local.endpoint, COMMAND_PATH))
                .timeout(Duration::from_millis(200))
                .send()
                .await
                .is_err()
        );
        self.service.take();
    }
    pub async fn finish(mut self) {
        self.assert_no_effects();
        self.gateway.as_mut().unwrap().terminate().await;
        self.gateway.take();
        assert!(tokio::net::TcpStream::connect(self.address).await.is_err());
        if self.service.is_some() {
            self.stop_service().await;
        }
        for peer in &mut self.peers {
            peer.finish().await;
        }
        self.env.assert_logs_private(&[&self.local.token]);
        self.env.complete();
    }
}
pub fn security(response: &Response) {
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
}
pub fn envelope(command: VesselCommand) -> Value {
    serde_json::to_value(VesselRequest {
        protocol: VESSEL_API_VERSION,
        command,
    })
    .unwrap()
}
pub struct Stream {
    body: Response,
    pending: Vec<u8>,
    total: usize,
}
impl Stream {
    pub fn new(response: Response) -> Self {
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        assert_eq!(response.headers()["x-accel-buffering"], "no");
        Self {
            body: response,
            pending: Vec::new(),
            total: 0,
        }
    }
    pub async fn next(&mut self) -> VesselEvent {
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(end) = self.pending.windows(2).position(|w| w == b"\n\n") {
                    let packet: Vec<_> = self.pending.drain(..end + 2).collect();
                    let text = std::str::from_utf8(&packet).unwrap();
                    if text.starts_with(':') {
                        continue;
                    }
                    assert!(text.lines().any(|line| line == "event: update"));
                    let data = text
                        .lines()
                        .find_map(|line| line.strip_prefix("data: "))
                        .unwrap();
                    return serde_json::from_str(data).unwrap();
                }
                let chunk = self
                    .body
                    .chunk()
                    .await
                    .unwrap()
                    .expect("owned SSE ended before event");
                self.total += chunk.len();
                assert!(self.total <= 1024 * 1024);
                self.pending.extend_from_slice(&chunk);
                assert!(self.pending.len() <= 65536);
            }
        })
        .await
        .expect("bounded actual SSE publication")
    }
    pub async fn ended(&mut self) {
        tokio::time::timeout(WAIT, async {
            while let Some(chunk) = self.body.chunk().await.unwrap() {
                self.total += chunk.len();
                assert!(self.total <= 1024 * 1024);
                assert!(
                    chunk.iter().all(u8::is_ascii_whitespace),
                    "terminal SSE must not publish another payload"
                );
            }
        })
        .await
        .unwrap();
        assert!(self.pending.iter().all(u8::is_ascii_whitespace));
    }
}
pub async fn until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
pub fn authorization(call: &Value, grant: &ConnectionGrant) -> GrantBinding {
    let binding: GrantBinding = serde_json::from_value(call["authorization"].clone()).unwrap();
    assert_eq!(binding.principal_id, grant.principal_id);
    assert_eq!(binding.revision, grant.revision);
    binding
}
pub fn attention() -> VesselCommand {
    VesselCommand::Notifications {
        operation: NotificationOperation::Attention,
    }
}
