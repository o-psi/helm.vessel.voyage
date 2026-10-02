//! Production ordinary serve()/courier in independent owned libtest children.
//! Voyage executes in its own current runnable process, never in Supervisor.
use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    process::{Child, Command, Stdio},
    time::Instant,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use voyage_protocol::{
    notifications::*,
    process::{RuntimeCommand, RuntimeRequest, RuntimeResponse, read_frame, write_frame},
};

const ROLE: &str = "VESSEL_OWNED_COURIER_ROLE_353";
const TEST: &str = "process::service::owned_service_courier_tests::ordinary_service_courier_ownership_receipts_restart_and_cleanup";
const KEY: &str = "owned-synthetic-courier-key";

#[derive(Clone)]
struct Release(tokio::sync::watch::Sender<bool>);
impl Release {
    fn new() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
    fn cancel(&self) {
        self.0.send_replace(true);
    }
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        while !*receiver.borrow_and_update() {
            receiver.changed().await.expect("owned release witness");
        }
    }
}

fn voyage_binary() -> PathBuf {
    let current = std::env::current_exe().unwrap();
    let deps = current.parent().unwrap();
    assert_eq!(deps.file_name().unwrap(), "deps");
    let binary = deps.parent().unwrap().join("voyage");
    assert!(
        binary.is_file(),
        "coordinator must build/audit the current instrumented Voyage entrypoint"
    );
    binary
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
fn owned(root: &Path) -> Vec<(u32, String)> {
    let binary = voyage_binary();
    let prefix = root.as_os_str().as_encoded_bytes();
    fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid = entry.file_name().to_str()?.parse().ok()?;
            if entry.metadata().ok()?.uid() != unsafe { libc::geteuid() }
                || fs::read_link(entry.path().join("exe")).ok()? != binary
            {
                return None;
            }
            let args = fs::read(entry.path().join("cmdline")).ok()?;
            if !args.split(|b| *b == 0).any(|arg| arg.starts_with(prefix)) {
                return None;
            }
            Some((pid, start(pid)?))
        })
        .collect()
}
fn process_role(pid: u32, role: &[u8]) -> bool {
    fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .is_some_and(|args| args.split(|byte| *byte == 0).nth(1) == Some(role))
}
fn parent_pid(pid: u32) -> Option<u32> {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()?
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
struct ChildOwner {
    child: Option<Child>,
    root: PathBuf,
    journey: bool,
    log: PathBuf,
}
impl ChildOwner {
    fn spawn(root: &Path, role: &str, mode: &str) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        let log = root.join(format!("child-{role}-{}.log", Uuid::new_v4().simple()));
        let output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&log)
            .unwrap();
        command
            .args(["--exact", TEST, "--nocapture", "--test-threads=1"])
            .env_clear()
            .env(ROLE, role)
            .env("VESSEL_OWNED_COURIER_MODE", mode)
            .env(
                "VESSEL_OWNED_COURIER_PARENT",
                std::process::id().to_string(),
            )
            .env("VESSEL_OWNED_COURIER_ROOT", root)
            .env("HOME", root)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output);
        for (key, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            let path = root.join(name);
            fs::create_dir_all(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            command.env(key, path);
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        Self {
            child: Some(command.spawn().unwrap()),
            root: root.into(),
            journey: role == "journey",
            log,
        }
    }
    fn pid(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }
    async fn terminate(&mut self) {
        assert_eq!(unsafe { libc::kill(self.pid() as i32, libc::SIGTERM) }, 0);
        self.wait().await;
    }
    async fn wait(&mut self) {
        self.wait_expected(true).await;
    }
    async fn wait_expected(&mut self, success: bool) {
        let child = self.child.as_mut().unwrap();
        let deadline = Instant::now() + Duration::from_secs(if self.journey { 120 } else { 15 });
        loop {
            assert!(
                fs::metadata(&self.log).unwrap().len() <= 1024 * 1024,
                "owned fixture output bound"
            );
            if let Some(status) = child.try_wait().unwrap() {
                assert_eq!(
                    status.success(),
                    success,
                    "owned service result changed; private failure evidence retained"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "owned service retirement unconfirmed"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        self.child.take();
    }
}
impl Drop for ChildOwner {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
            for (pid, ticks) in owned(&self.root) {
                if start(pid).as_ref() == Some(&ticks) {
                    let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }
        }
    }
}

struct Api {
    client: reqwest::Client,
    credential: LocalAccessCredential,
}
impl Api {
    async fn open(root: &Path) -> Self {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if let Ok(credential) = registry::load_local_access(root) {
                    let api = Self {
                        client: reqwest::Client::builder()
                            .timeout(Duration::from_secs(10))
                            .build()
                            .unwrap(),
                        credential,
                    };
                    if api.command(VesselCommand::Capabilities).await.is_ok() {
                        return api;
                    }
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }
    async fn response(&self, command: VesselCommand) -> VesselResponse {
        let response = self
            .client
            .post(format!(
                "{}{}",
                self.credential.endpoint,
                voyage_protocol::vessel::COMMAND_PATH
            ))
            .bearer_auth(&self.credential.token)
            .json(&VesselRequest {
                protocol: VESSEL_API_VERSION,
                command,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        response.json().await.unwrap()
    }
    async fn command(&self, command: VesselCommand) -> Result<Value> {
        let response = self.response(command).await;
        ensure!(response.protocol == VESSEL_API_VERSION, "changed protocol");
        ensure!(response.error.is_none(), "owned request refused");
        Ok(response.result)
    }
    async fn voyage(&self, p: &ProcessInfo, command: VoyageCommand) -> Result<Value> {
        let pinned = command.requires_incarnation().then_some(p.incarnation);
        let value = self
            .command(VesselCommand::Voyage(VoyageRequest {
                session_id: p.session_id,
                incarnation: pinned,
                command,
            }))
            .await?;
        ensure!(
            value["session_id"] == p.session_id.to_string(),
            "wrong observed session"
        );
        ensure!(
            value["incarnation"]
                .as_str()
                .is_some_and(|id| Uuid::parse_str(id).is_ok_and(|id| !id.is_nil())),
            "missing actual owner"
        );
        Ok(value["result"].clone())
    }
    async fn notification(&self, op: NotificationOperation) -> Result<Value> {
        self.command(VesselCommand::Notifications { operation: op })
            .await
    }
}

struct Provider {
    url: String,
    requests: Arc<std::sync::Mutex<Vec<Value>>>,
    release: Release,
    task: tokio::task::JoinHandle<()>,
}
impl Provider {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = requests.clone();
        let release = Release::new();
        let released = release.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {Some(result)=tasks.join_next(),if !tasks.is_empty()=>{result.unwrap();},accept=listener.accept()=>{let(mut stream,_)=accept.unwrap();let capture=capture.clone();let release=released.clone();tasks.spawn(async move{
                    let mut bytes=Vec::new();let end=loop{let mut buffer=[0u8;4096];let count=stream.read(&mut buffer).await.unwrap();assert!(count>0&&bytes.len()+count<=262144);bytes.extend_from_slice(&buffer[..count]);if let Some(i)=bytes.windows(4).position(|w|w==b"\r\n\r\n"){break i+4;}};
                    let header=String::from_utf8(bytes[..end].to_vec()).unwrap();assert!(header.to_ascii_lowercase().contains(&format!("authorization: bearer {KEY}")));
                    let length=header.lines().find_map(|l|{let(k,v)=l.split_once(':')?;k.eq_ignore_ascii_case("content-length").then(||v.trim().parse::<usize>().unwrap())}).unwrap_or(0);assert!(length<=131072);
                    while bytes.len()<end+length{let mut buffer=[0u8;4096];let count=stream.read(&mut buffer).await.unwrap();assert!(count>0);bytes.extend_from_slice(&buffer[..count]);}
                    let(body,kind)=if header.starts_with("GET /v1/models"){(json!({"data":[{"id":"owned-courier-model","input_modalities":["text"]}]}).to_string(),"application/json")}else{assert!(header.starts_with("POST /v1/chat/completions"));capture.lock().unwrap().push(serde_json::from_slice(&bytes[end..end+length]).unwrap());release.cancelled().await;(format!("data: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"owned-private-answer"},"finish_reason":"stop"}]})),"text/event-stream")};
                    let reply=format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());let _=stream.write_all(reply.as_bytes()).await;
                });}}
            }
        });
        Self {
            url,
            requests,
            release,
            task,
        }
    }
    fn config(&self, root: &Path) -> voyage_runtime::Config {
        let registry = voyage_runtime::accounts::Registry::default_host().unwrap();
        let connection = registry
            .add_connection(
                "owned-courier".into(),
                self.url.clone(),
                vec![voyage_protocol::accounts::Transport::OpenaiChat],
            )
            .unwrap();
        let account = registry
            .add_api(
                connection.id,
                "owned".into(),
                "Owned synthetic courier account".into(),
                voyage_runtime::accounts::ApiKeyInput::Stored(KEY.into()),
            )
            .unwrap();
        voyage_runtime::Config {
            provider: voyage_runtime::config::ProviderKind::OpenaiChat,
            model: "owned-courier-model".into(),
            workspace: Some(root.into()),
            base_url: Some(self.url.clone()),
            api_key_required: true,
            api_key_env: format!("OWNED_UNUSED_{}", Uuid::new_v4().simple()),
            account: Some(voyage_protocol::accounts::AccountBinding {
                account_id: account.id,
                connection_id: connection.id,
                identity_generation: account.identity_generation,
                connection_revision: connection.revision,
                transport: voyage_protocol::accounts::Transport::OpenaiChat,
            }),
            access: Some(voyage_runtime::config::AccessMode::ReadOnly),
            provider_retry_attempts: 1,
            provider_response_timeout_ms: 10000,
            provider_stream_idle_ms: 10000,
            ..Default::default()
        }
    }
    async fn wait(&self) {
        tokio::time::timeout(Duration::from_secs(15), async {
            while self.requests.lock().unwrap().is_empty() {
                assert!(!self.task.is_finished());
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn raw(root: &Path, command: RuntimeCommand) -> RuntimeResponse {
    let registration: ProcessRegistration =
        serde_json::from_slice(&fs::read(root.join("registration.json")).unwrap()).unwrap();
    let mut stream = tokio::net::UnixStream::connect(root.join("runtime.sock"))
        .await
        .unwrap();
    write_frame(
        &mut stream,
        &RuntimeRequest {
            protocol: PROCESS_PROTOCOL,
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            token: registration.token,
            authorization: None,
            scope_authority: None,
            command,
        },
    )
    .await
    .unwrap();
    let reply: RuntimeResponse =
        tokio::time::timeout(Duration::from_secs(10), read_frame(&mut stream))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(reply.session_id, registration.session_id);
    assert!(reply.error.is_none());
    reply
}
async fn inbox(api: &Api, id: Uuid) -> Value {
    api.notification(NotificationOperation::Inbox {
        destination_id: id,
        after: 0,
        limit: 100,
    })
    .await
    .unwrap()
}
async fn complete(api: &Api, p: &ProcessInfo) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snap = api.voyage(p, VoyageCommand::Snapshot).await.unwrap();
            if snap["run"]["state"] == "completed" && snap["pending_cleanup_run"].is_null() {
                return snap;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

async fn journey(root: &Path, mode: &str) {
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let mut runtimes = RuntimeOwner {
        root: root.into(),
        finished: false,
    };
    let directory = root.to_owned();
    let mut service = ChildOwner::spawn(root, "service", mode);
    let api = Api::open(&directory).await;
    if mode == "ordinary" {
        let before = registry::load_local_access(&directory).unwrap();
        let mut duplicate = ChildOwner::spawn(root, "service", mode);
        duplicate.wait_expected(false).await;
        let after = registry::load_local_access(&directory).unwrap();
        assert_eq!(after.token, before.token);
        assert_eq!(after.endpoint, before.endpoint);
        assert!(
            api.command(VesselCommand::Catalogue)
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
    let capabilities = api.command(VesselCommand::Capabilities).await.unwrap();
    let vessel = serde_json::from_value(capabilities["vessel_id"].clone()).unwrap();
    assert!(
        capabilities["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "notifications")
    );
    for (origin, token, expected) in [
        (
            Some("https://owned.invalid"),
            api.credential.token.clone(),
            reqwest::StatusCode::FORBIDDEN,
        ),
        (None, "a".repeat(64), reqwest::StatusCode::UNAUTHORIZED),
    ] {
        let mut request = api
            .client
            .post(format!(
                "{}{}",
                api.credential.endpoint,
                voyage_protocol::vessel::COMMAND_PATH
            ))
            .bearer_auth(token)
            .json(&VesselRequest {
                protocol: VESSEL_API_VERSION,
                command: VesselCommand::Capabilities,
            });
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        assert_eq!(request.send().await.unwrap().status(), expected);
    }
    let malformed = api
        .client
        .post(format!(
            "{}{}",
            api.credential.endpoint,
            voyage_protocol::vessel::COMMAND_PATH
        ))
        .bearer_auth(&api.credential.token)
        .json(&VesselRequest {
            protocol: 0,
            command: VesselCommand::Capabilities,
        })
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status(), reqwest::StatusCode::BAD_REQUEST);
    let provider = Provider::new().await;
    let workspace = root.join("workspace");
    fs::create_dir(&workspace).unwrap();
    let config = provider.config(&workspace);
    let launch = root.join("launch.json");
    fs::write(
        &launch,
        serde_json::to_vec(
            &voyage_runtime::launch_config::LaunchConfig::capture(&config, &workspace).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    fs::set_permissions(&launch, fs::Permissions::from_mode(0o600)).unwrap();
    let command = VesselCommand::StartConfigured {
        command_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        workspace: workspace.clone(),
        config_path: launch,
    };
    let process: ProcessInfo =
        serde_json::from_value(api.command(command.clone()).await.unwrap()).unwrap();
    let duplicate: ProcessInfo =
        serde_json::from_value(api.command(command).await.unwrap()).unwrap();
    assert_eq!(duplicate.session_id, process.session_id);
    let initial = api.voyage(&process, VoyageCommand::Snapshot).await.unwrap();
    assert!(initial["messages"].as_array().unwrap().is_empty());
    let now = super::super::access::store::now().unwrap();
    let quiet = if mode == "quiet" {
        let minute = ((now / 60000) % 1440) as u16;
        Some(QuietHoursUtc {
            start_minute: minute,
            end_minute: (minute + 2) % 1440,
        })
    } else {
        None
    };
    let destination = Destination {
        id: Uuid::new_v4(),
        recipient_grant_id: vessel,
        recipient_principal_id: vessel,
        recipient_grant_revision: 1,
        source_vessel_id: vessel,
        source_session_id: process.session_id,
        event_kinds: vec![NotificationKind::Test, NotificationKind::Completed],
        expires_at_ms: now + 60000,
        notification_ttl_ms: 30000,
        quiet_hours_utc: quiet,
    };
    let configure = NotificationOperation::Configure {
        command_id: Uuid::new_v4(),
        destination: destination.clone(),
    };
    let configured = api.notification(configure.clone()).await.unwrap();
    assert_eq!(api.notification(configure).await.unwrap(), configured);
    for case in 0..4 {
        let mut wrong = destination.clone();
        wrong.id = Uuid::new_v4();
        match case {
            0 => wrong.source_vessel_id = Uuid::new_v4(),
            1 => wrong.source_session_id = Uuid::new_v4(),
            2 => wrong.recipient_principal_id = Uuid::new_v4(),
            _ => wrong.recipient_grant_revision += 1,
        }
        assert!(
            api.notification(NotificationOperation::Configure {
                command_id: Uuid::new_v4(),
                destination: wrong
            })
            .await
            .is_err()
        );
    }
    assert!(
        inbox(&api, destination.id).await["page"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let accept = NotificationOperation::Accept {
        command_id: Uuid::new_v4(),
        destination_id: destination.id,
    };
    let accepted = api.notification(accept.clone()).await.unwrap();
    assert_eq!(api.notification(accept).await.unwrap(), accepted);
    let test = NotificationOperation::Test {
        command_id: Uuid::new_v4(),
        destination_id: destination.id,
    };
    let receipt = api.notification(test.clone()).await.unwrap();
    assert_eq!(api.notification(test.clone()).await.unwrap(), receipt);
    let event = serde_json::from_value(receipt["event_id"].clone()).unwrap();
    let page = inbox(&api, destination.id).await;
    assert_eq!(page["page"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(page["attention_deferred"], mode == "quiet");
    assert!(!page.to_string().contains(KEY));
    assert!(!page.to_string().contains("owned-private-answer"));
    let seen = api
        .notification(NotificationOperation::Receipt {
            destination_id: destination.id,
            event_id: event,
            state: ReceiptState::Seen,
        })
        .await
        .unwrap();
    let stale = api
        .notification(NotificationOperation::Receipt {
            destination_id: destination.id,
            event_id: event,
            state: ReceiptState::Available,
        })
        .await
        .unwrap();
    assert_eq!(seen, stale);
    let opened = api
        .notification(NotificationOperation::Open {
            destination_id: destination.id,
            event_id: event,
        })
        .await
        .unwrap();
    assert_eq!(opened["actionable"], false);
    assert_eq!(opened["execution_cleanup"], "not_implied");
    assert!(provider.requests.lock().unwrap().is_empty());
    let submit_id = Uuid::new_v4();
    let submitted = api
        .voyage(
            &process,
            VoyageCommand::Submit {
                budget: None,
                coordination: None,
                command_id: submit_id,
                expected_revision: initial["revision"].as_u64().unwrap(),
                expires_at_ms: now + 60000,
                prompt: "owned-private-user".into(),
            },
        )
        .await
        .unwrap();
    provider.wait().await;
    let runtime_root = directory
        .join("sessions")
        .join(process.session_id.to_string());
    let pids = owned(&runtime_root);
    let execution: Vec<_> = pids
        .iter()
        .filter(|(pid, _)| process_role(*pid, b"serve"))
        .collect();
    let guardians: Vec<_> = pids
        .iter()
        .filter(|(pid, _)| process_role(*pid, b"supervise"))
        .collect();
    assert_eq!(execution.len(), 1, "one exact execution owner");
    assert_eq!(guardians.len(), 1, "one separate owned cleanup guardian");
    let runtime_identity = (*execution[0]).clone();
    let guardian_identity = (*guardians[0]).clone();
    assert_ne!(runtime_identity.0, guardian_identity.0);
    assert_ne!(runtime_identity.0, service.pid());
    assert_eq!(parent_pid(runtime_identity.0), Some(guardian_identity.0));
    assert_eq!(parent_pid(guardian_identity.0), Some(service.pid()));
    if mode == "restart-held" {
        let token = api.credential.token.clone();
        service.terminate().await;
        assert!(!directory.join("process-http.json").exists());
        assert_eq!(
            start(runtime_identity.0).as_ref(),
            Some(&runtime_identity.1)
        );
        assert_eq!(
            start(guardian_identity.0).as_ref(),
            Some(&guardian_identity.1)
        );
        assert_eq!(parent_pid(runtime_identity.0), Some(guardian_identity.0));
        let snap = raw(&runtime_root, RuntimeCommand::Snapshot).await.result;
        assert_eq!(snap["run"]["run_id"], submitted["run_id"]);
        assert!(matches!(
            snap["run"]["state"].as_str(),
            Some("accepted" | "running")
        ));
        service = ChildOwner::spawn(root, "service", mode);
        let renewed = Api::open(&directory).await;
        assert_ne!(renewed.credential.token, token);
        assert_eq!(
            renewed.command(VesselCommand::Capabilities).await.unwrap()["vessel_id"],
            capabilities["vessel_id"]
        );
        provider.release.cancel();
        complete(&renewed, &process).await;
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
        renewed
            .command(VesselCommand::Stop {
                session_id: process.session_id,
                incarnation: process.incarnation,
            })
            .await
            .unwrap();
        service.terminate().await;
    } else {
        if mode == "revoke-held" {
            let revoke = NotificationOperation::Revoke {
                command_id: Uuid::new_v4(),
                destination_id: destination.id,
            };
            let refused = api.notification(revoke.clone()).await.unwrap();
            assert_eq!(refused["execution_cleanup"], "not_implied");
            api.notification(revoke).await.unwrap();
            assert!(
                inbox(&api, destination.id).await["page"]["entries"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        provider.release.cancel();
        let snap = complete(&api, &process).await;
        assert_eq!(
            snap["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|m| m["role"] == "user" && m["content"] == "owned-private-user")
                .count(),
            1
        );
        if mode != "revoke-held" {
            let delivered = tokio::time::timeout(Duration::from_secs(15), async {
                loop {
                    let page = inbox(&api, destination.id).await;
                    if page["page"]["entries"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|entry| entry["notification"]["kind"] == "completed")
                    {
                        return page;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            })
            .await
            .unwrap();
            assert!(
                !delivered.to_string().contains("owned-private-user")
                    && !delivered.to_string().contains("owned-private-answer")
                    && !delivered.to_string().contains(KEY)
            );
            assert_eq!(api.notification(test).await.unwrap(), seen);
            let inventory = api
                .notification(NotificationOperation::Destinations)
                .await
                .unwrap();
            assert_eq!(inventory["destinations"].as_array().unwrap().len(), 1);
            assert!(
                inventory["delivery"][0]["producer"]["after"]
                    .as_u64()
                    .unwrap()
                    > 0
            );
        } else {
            assert!(
                inbox(&api, destination.id).await["page"]["entries"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
        api.command(VesselCommand::Stop {
            session_id: process.session_id,
            incarnation: process.incarnation,
        })
        .await
        .unwrap();
        service.terminate().await;
    }
    tokio::time::timeout(Duration::from_secs(10), async {
        while !owned(&directory).is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_ne!(
        start(runtime_identity.0).as_ref(),
        Some(&runtime_identity.1)
    );
    assert_ne!(
        start(guardian_identity.0).as_ref(),
        Some(&guardian_identity.1)
    );
    assert!(!directory.join("process-http.json").exists());
    runtimes.finished = true;
}

struct RuntimeOwner {
    root: PathBuf,
    finished: bool,
}
impl Drop for RuntimeOwner {
    fn drop(&mut self) {
        if !self.finished {
            for (pid, ticks) in owned(&self.root) {
                if start(pid).as_ref() == Some(&ticks) {
                    let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }
        }
    }
}

#[test]
fn ordinary_service_courier_ownership_receipts_restart_and_cleanup() {
    let role = std::env::var(ROLE).ok();
    if let Some(role) = role {
        let root = PathBuf::from(std::env::var_os("VESSEL_OWNED_COURIER_ROOT").unwrap());
        let parent: u32 = std::env::var("VESSEL_OWNED_COURIER_PARENT")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::getppid() } as u32, parent);
        assert_eq!(
            fs::read_link(format!("/proc/{parent}/exe")).unwrap(),
            std::env::current_exe().unwrap()
        );
        assert_eq!(root.canonicalize().unwrap(), root);
        assert_eq!(fs::metadata(&root).unwrap().uid(), unsafe {
            libc::geteuid()
        });
        assert_eq!(fs::metadata(&root).unwrap().mode() & 0o077, 0);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mode = std::env::var("VESSEL_OWNED_COURIER_MODE").unwrap();
        match role.as_str() {
            "service" => runtime
                .block_on(serve(root.clone(), voyage_binary()))
                .unwrap(),
            "journey" => runtime.block_on(journey(&root, &mode)),
            _ => panic!("unowned role"),
        };
        return;
    }
    for mode in ["ordinary", "quiet", "revoke-held", "restart-held"] {
        let root = OwnedRoot::new();
        let mut child = ChildOwner::spawn(&root.path, "journey", mode);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(child.wait());
        assert!(owned(&root.path).is_empty());
        fs::remove_dir_all(&root.path).unwrap();
    }
}

struct OwnedRoot {
    path: PathBuf,
}
impl OwnedRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("vc-{}", Uuid::new_v4().simple()));
        registry::private_directory(&path).unwrap();
        Self {
            path: path.canonicalize().unwrap(),
        }
    }
}
// Failed private roots intentionally remain for concrete diagnostics; success
// removes them only after owned runtime and supervisor retirement.
