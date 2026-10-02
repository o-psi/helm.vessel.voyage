//! Real ordinary Vessel/Voyage processes; only inference and response-loss are fixtures.
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use voyage_protocol::{
    duplex::{ClientFrame, SUBPROTOCOL, ServerFrame},
    vessel::{ProcessState, VESSEL_API_VERSION, VesselResponse},
};

const CHILD: &str = "HELM_OWNED_FRONTEND_353";
const SYNTHETIC_KEY: &str = "owned-frontend-synthetic-local-only";
const TEST: &str = "process_client::frontend::owned_process_tests::owned_ordinary_frontend_preserves_canonical_history_exact_delivery_and_cleanup";

fn binary(name: &str) -> PathBuf {
    let current = std::env::current_exe().unwrap();
    let directory = current.parent().unwrap();
    assert_eq!(directory.file_name().unwrap(), "deps");
    let binary = directory.parent().unwrap().join(name);
    assert!(
        binary.is_file(),
        "current workspace runnable {name} is required; do not skip this fixture"
    );
    binary
}
fn pid_start(pid: u32) -> Option<String> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    text.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}
fn owned_pids(directory: &Path, executable: &Path) -> Vec<(u32, String)> {
    use std::os::unix::fs::MetadataExt;
    let needle = directory.as_os_str().as_encoded_bytes();
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
            if entry.metadata().ok()?.uid() != unsafe { libc::geteuid() } {
                return None;
            }
            if std::fs::read_link(entry.path().join("exe")).ok()? != executable {
                return None;
            }
            let command = std::fs::read(entry.path().join("cmdline")).ok()?;
            if !command
                .split(|byte| *byte == 0)
                .any(|argument| argument.starts_with(needle))
            {
                return None;
            }
            Some((pid, pid_start(pid)?))
        })
        .collect()
}
struct OwnedVessel {
    child: Child,
    directory: PathBuf,
    voyage: PathBuf,
    observed: Vec<(u32, String)>,
    finished: bool,
}
impl OwnedVessel {
    fn start(directory: PathBuf) -> Self {
        let voyage = binary("voyage");
        let log_path = directory.parent().unwrap().join("frontend-vessel.log");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        let log = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(log_path)
            .unwrap();
        let child = Command::new(binary("vessel"))
            .args(["local-serve", "--directory"])
            .arg(&directory)
            .arg("--voyage-binary")
            .arg(&voyage)
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        Self {
            child,
            directory,
            voyage,
            observed: Vec::new(),
            finished: false,
        }
    }
    fn observe_runtime(&mut self, session: Uuid) {
        let pids = owned_pids(
            &self.directory.join("sessions").join(session.to_string()),
            &self.voyage,
        )
        .into_iter()
        .filter(|(pid, _)| {
            let arguments = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
            matches!(
                arguments.split(|byte| *byte == 0).nth(1),
                Some(b"serve" | b"serve-bound")
            )
        })
        .collect::<Vec<_>>();
        assert_eq!(
            pids.len(),
            1,
            "one independent Voyage must own the held run"
        );
        assert_ne!(pids[0].0, self.child.id());
        assert_ne!(pids[0].0, std::process::id());
        self.observed.extend(pids);
    }
    async fn finish(&mut self, client: &Client) {
        let processes = catalogue(client).await;
        for process in processes {
            let has_executor = owned_pids(
                &self
                    .directory
                    .join("sessions")
                    .join(process.session_id.to_string()),
                &self.voyage,
            )
            .into_iter()
            .any(|(pid, _)| {
                let arguments = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                matches!(
                    arguments.split(|byte| *byte == 0).nth(1),
                    Some(b"serve" | b"serve-bound")
                )
            });
            if has_executor {
                client
                    .request(VesselCommand::Stop {
                        session_id: process.session_id,
                        incarnation: process.incarnation,
                    })
                    .await
                    .unwrap();
            }
        }
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if owned_pids(&self.directory, &self.voyage).is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        for (pid, start) in &self.observed {
            assert_ne!(
                pid_start(*pid).as_ref(),
                Some(start),
                "owned Voyage still exists after cleanup"
            );
        }
        for process in catalogue(client).await {
            let observed: ProcessInfo = serde_json::from_value(
                client
                    .request(VesselCommand::Inspect {
                        session_id: process.session_id,
                    })
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert!(!matches!(
                observed.state,
                ProcessState::Live | ProcessState::Starting | ProcessState::CleanupUnconfirmed
            ));
        }
        self.child.kill().unwrap();
        self.child.wait().unwrap();
        self.finished = true;
    }
}
impl Drop for OwnedVessel {
    fn drop(&mut self) {
        if !self.finished {
            // Failure teardown only: exact owned executable/root/current PID identity,
            // never evidence of successful production cleanup or a replayed mutation.
            for (pid, start) in owned_pids(&self.directory, &self.voyage) {
                if pid_start(pid).as_ref() == Some(&start) {
                    let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

struct Provider {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    release: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
impl Provider {
    async fn new(failure: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let release = CancellationToken::new();
        let released = release.clone();
        let task = tokio::spawn(async move {
            let mut workers = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = workers.join_next(), if !workers.is_empty() => { result.unwrap().unwrap(); },
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        let requests = captured.clone(); let release = released.clone();
                        workers.spawn(async move {
                            let mut bytes = Vec::new();
                            let end = loop {
                                let mut buffer = [0u8; 4096]; let count = stream.read(&mut buffer).await.unwrap();
                                assert!(count > 0 && bytes.len() + count <= 262144); bytes.extend_from_slice(&buffer[..count]);
                                if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") { break index + 4; }
                            };
                            let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                            let authorization = headers.lines().filter_map(|line| { let (name, value) = line.split_once(':')?; name.eq_ignore_ascii_case("authorization").then(|| value.trim()) }).collect::<Vec<_>>();
                            assert_eq!(authorization, vec![format!("Bearer {SYNTHETIC_KEY}")], "only the fixed private-fixture credential may reach the owned loopback server");
                            let length = headers.lines().find_map(|line| { let (name, value) = line.split_once(':')?; name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap()) }).unwrap_or(0);
                            assert!(length <= 131072);
                            while bytes.len() < end + length { let mut buffer = [0u8; 4096]; let count = stream.read(&mut buffer).await.unwrap(); assert!(count > 0); bytes.extend_from_slice(&buffer[..count]); }
                            let (status, content_type, body) = if headers.starts_with("GET /v1/models") {
                                (200, "application/json", json!({"data":[{"id":"owned-model","input_modalities":["text"]},{"id":"owned-explicit-branch-model","input_modalities":["text"]}]}).to_string())
                            } else {
                                assert!(headers.starts_with("POST /v1/chat/completions"));
                                requests.lock().unwrap().push(serde_json::from_slice(&bytes[end..end + length]).unwrap());
                                release.cancelled().await;
                                if failure { (500, "application/json", json!({"error":{"message":"owned refusal","type":"fixture"}}).to_string()) }
                                else { (200, "text/event-stream", format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"owned canonical answer"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}))) }
                            };
                            let response = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                        });
                    }
                }
            }
        });
        Self {
            url,
            requests,
            release,
            task,
        }
    }
    fn config(&self) -> crate::Config {
        let registry = voyage_runtime::accounts::Registry::default_host().unwrap();
        let connection = registry
            .add_connection(
                "owned-loopback".into(),
                self.url.clone(),
                vec![voyage_protocol::accounts::Transport::OpenaiChat],
            )
            .unwrap();
        let account = registry
            .add_api(
                connection.id,
                "owned".into(),
                "Owned synthetic account".into(),
                voyage_runtime::accounts::ApiKeyInput::Stored(SYNTHETIC_KEY.into()),
            )
            .unwrap();
        crate::Config {
            provider: crate::config::ProviderKind::OpenaiChat,
            model: "owned-model".into(),
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
            access: Some(crate::config::AccessMode::ReadOnly),
            provider_retry_attempts: 1,
            provider_response_timeout_ms: 10000,
            provider_stream_idle_ms: 10000,
            ..Default::default()
        }
    }
    async fn wait(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while self.requests.lock().unwrap().len() < count {
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
async fn catalogue(client: &Client) -> Vec<ProcessInfo> {
    serde_json::from_value(client.request(VesselCommand::Catalogue).await.unwrap()).unwrap()
}
async fn inspect(client: &Client, session_id: Uuid) -> ProcessInfo {
    serde_json::from_value(
        client
            .request(VesselCommand::Inspect { session_id })
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn snapshot(client: &Client, process: &ProcessInfo) -> Value {
    client
        .voyage(
            process.session_id,
            process.incarnation,
            VoyageCommand::Snapshot,
        )
        .await
        .unwrap()
}
async fn terminal(client: &Client, process: &ProcessInfo) -> Value {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            let snapshot = snapshot(client, process).await;
            if matches!(
                snapshot["run"]["state"].as_str(),
                Some("completed" | "failed" | "cancelled")
            ) && snapshot["pending_cleanup_run"].is_null()
            {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
fn canonical(snapshot: &Value, prompt: &str) {
    let messages = snapshot["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["role"] == "user" && message["content"] == prompt)
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["role"] == "assistant"
                && message["content"] == "owned canonical answer")
            .count(),
        1
    );
    assert_eq!(snapshot["run"]["state"], "completed");
    assert!(snapshot["pending_cleanup_run"].is_null());
}

struct LostReply {
    client: Client,
    original: Arc<Mutex<Option<VoyageCommand>>>,
    task: tokio::task::JoinHandle<()>,
    _directory: tempfile::TempDir,
}
impl LostReply {
    async fn start(real: Client) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let path = directory.path().join("process-http.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"endpoint":endpoint,"token":"b".repeat(64)})).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let original = Arc::new(Mutex::new(None));
        let captured = original.clone();
        let vessel_id = serde_json::from_value(
            real.request(VesselCommand::Capabilities).await.unwrap()["vessel_id"].clone(),
        )
        .unwrap();
        let task = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            #[allow(clippy::result_large_err)]
            let handshake = |_: &tokio_tungstenite::tungstenite::handshake::server::Request, mut response: tokio_tungstenite::tungstenite::handshake::server::Response| { response.headers_mut().insert("sec-websocket-protocol", SUBPROTOCOL.parse().unwrap()); Ok(response) };
            let mut socket = tokio_tungstenite::accept_hdr_async(stream, handshake)
                .await
                .unwrap();
            socket
                .send(tokio_tungstenite::tungstenite::Message::Text(
                    serde_json::to_string(&ServerFrame::Hello {
                        protocol: VESSEL_API_VERSION,
                        socket_id: Uuid::new_v4(),
                        vessel_id,
                    })
                    .unwrap()
                    .into(),
                ))
                .await
                .unwrap();
            while let Some(Ok(message)) = socket.next().await {
                if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                    let ClientFrame::Command {
                        request_id,
                        request,
                    } = serde_json::from_str(&text).unwrap()
                    else {
                        panic!("only exact commands precede lost submit reply")
                    };
                    let lost = match &request.command {
                        VesselCommand::Voyage(request)
                            if matches!(request.command, VoyageCommand::Submit { .. }) =>
                        {
                            *captured.lock().unwrap() = Some(request.command.clone());
                            true
                        }
                        _ => false,
                    };
                    let result = real.request(request.command).await.unwrap();
                    if lost {
                        socket.close(None).await.unwrap();
                        break;
                    }
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            serde_json::to_string(&ServerFrame::Reply {
                                request_id,
                                response: VesselResponse {
                                    protocol: VESSEL_API_VERSION,
                                    result,
                                    error: None,
                                    outcome_unknown: false,
                                },
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await
                        .unwrap();
                } else if let tokio_tungstenite::tungstenite::Message::Ping(bytes) = message {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Pong(bytes))
                        .await
                        .unwrap();
                }
            }
        });
        Self {
            client: Client::local(directory.path().into()),
            original,
            task,
            _directory: directory,
        }
    }
}
impl Drop for LostReply {
    fn drop(&mut self) {
        self.client.disconnect();
        self.task.abort();
    }
}

async fn journey(mode: &str) {
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "ordinary evidence requires ordinary UID"
    );
    let directory = super::super::cli::default_directory();
    let mut vessel = OwnedVessel::start(directory.clone());
    let client = Client::local(directory);
    tokio::time::timeout(Duration::from_secs(15), async {
        while client.request(VesselCommand::Capabilities).await.is_err() {
            assert!(vessel.child.try_wait().unwrap().is_none());
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap();
    let provider = Provider::new(mode == "temporary-failure").await;
    let config = provider.config();
    let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
    if mode == "saved" || mode == "temporary" || mode == "temporary-failure" {
        let selected = config.clone();
        let root = workspace.clone();
        let temporary = mode != "saved";
        let mut running = tokio::task::spawn_local(async move {
            run(
                selected,
                Some(root),
                None,
                "owned first prompt".into(),
                temporary,
                false,
                false,
            )
            .await
        });
        tokio::select! {
            _ = provider.wait(1) => {},
            result = &mut running => panic!("frontend returned before the held provider request: {result:?}"),
        }
        let processes = catalogue(&client).await;
        assert_eq!(processes.len(), 1);
        let process = &processes[0];
        vessel.observe_runtime(process.session_id);
        let active = snapshot(&client, process).await;
        assert!(matches!(
            active["run"]["state"].as_str(),
            Some("accepted" | "running")
        ));
        assert_eq!(
            active["workspace"],
            serde_json::to_value(&workspace).unwrap()
        );
        assert_eq!(
            active["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|message| message["role"] == "user"
                    && message["content"] == "owned first prompt")
                .count(),
            1
        );
        provider.release.cancel();
        let result = tokio::time::timeout(Duration::from_secs(30), running)
            .await
            .unwrap()
            .unwrap();
        if mode == "temporary-failure" {
            assert!(
                result
                    .err()
                    .expect("owned failed run")
                    .to_string()
                    .contains("state failed")
            );
        } else {
            result.unwrap();
        }
        if temporary {
            let process = inspect(&client, process.session_id).await;
            assert_eq!(process.state, ProcessState::Stopped);
            let deletion = process.deletion.as_ref().unwrap();
            assert_eq!(deletion["deleted"], true);
            assert_eq!(deletion["status"], "applied");
            assert_eq!(deletion["cleanup"], "observed");
            assert!(Uuid::parse_str(deletion["command_id"].as_str().unwrap()).is_ok());
            let retained = snapshot(&client, &process).await;
            assert_eq!(retained["lifecycle"]["deleted"], true);
            assert!(retained["messages"].as_array().unwrap().is_empty());
            assert!(
                owned_pids(&vessel.directory, &vessel.voyage)
                    .into_iter()
                    .all(|(pid, _)| {
                        let arguments =
                            std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                        !matches!(
                            arguments.split(|byte| *byte == 0).nth(1),
                            Some(b"serve" | b"serve-bound")
                        )
                    }),
                "deleted snapshot observation revived an execution owner"
            );
        } else {
            let saved = terminal(&client, process).await;
            canonical(&saved, "owned first prompt");
            let (resumed, original) = open(
                &config,
                Some(workspace.clone()),
                Some(process.session_id.to_string()),
                false,
                false,
            )
            .await
            .unwrap();
            assert_eq!(original.session_id, process.session_id);
            assert_eq!(original.workspace, workspace);
            assert_eq!(
                snapshot(&resumed, &original).await["messages"],
                saved["messages"]
            );
            assert_eq!(catalogue(&client).await.len(), 1);
        }
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
    } else if mode == "branch" {
        let (_, parent) = open(&config, Some(workspace.clone()), None, false, false)
            .await
            .unwrap();
        let before = snapshot(&client, &parent).await;
        assert!(before["messages"].as_array().unwrap().is_empty());
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let ready = catalogue(&client)
                    .await
                    .into_iter()
                    .find(|info| info.session_id == parent.session_id);
                if let Some(info) = ready {
                    assert_eq!(info.incarnation, parent.incarnation);
                    assert_eq!(info.workspace, parent.workspace);
                    if matches!(info.state, ProcessState::Live | ProcessState::Suspended) {
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("actual parent catalogue readiness before the separate resume/branch flow");
        let mut selected = config.clone();
        selected.model = "owned-explicit-branch-model".into();
        let reference = parent.session_id.to_string();
        let mut running = tokio::task::spawn_local(async move {
            run(
                selected,
                None,
                Some(reference),
                "owned branch prompt".into(),
                true,
                true,
                true,
            )
            .await
        });
        tokio::select! {
            _ = provider.wait(1) => {},
            result = &mut running => panic!("frontend returned before the held provider request: {result:?}"),
        }
        let processes = catalogue(&client).await;
        assert_eq!(processes.len(), 2);
        let branch_id = processes
            .iter()
            .find(|process| process.session_id != parent.session_id)
            .unwrap()
            .session_id;
        vessel.observe_runtime(branch_id);
        provider.release.cancel();
        tokio::time::timeout(Duration::from_secs(30), running)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let processes = catalogue(&client).await;
        assert_eq!(processes.len(), 2);
        let branch = inspect(&client, branch_id).await;
        assert_eq!(branch.state, ProcessState::Stopped);
        assert_eq!(branch.deletion.as_ref().unwrap()["cleanup"], "observed");
        let after = snapshot(&client, &parent).await;
        assert_eq!(after["messages"], before["messages"]);
        assert_eq!(after["model"], before["model"]);
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["model"], "owned-explicit-branch-model");
    } else if mode == "lost-reply" {
        let (_, process) = open(&config, Some(workspace), None, false, false)
            .await
            .unwrap();
        let proxy = LostReply::start(client.clone()).await;
        let id = Uuid::new_v4();
        let result = super::super::plain::run(
            &proxy.client,
            process.session_id,
            Some(id),
            "owned exact uncertain prompt".into(),
        )
        .await;
        assert!(result.is_err());
        provider.wait(1).await;
        vessel.observe_runtime(process.session_id);
        let original = proxy.original.lock().unwrap().clone().unwrap();
        assert!(
            matches!(&original, VoyageCommand::Submit { command_id, prompt, .. } if *command_id == id && prompt == "owned exact uncertain prompt")
        );
        let observed = client
            .voyage(
                process.session_id,
                process.incarnation,
                VoyageCommand::Resolve {
                    command_id: id,
                    original: Some(Box::new(original.clone())),
                },
            )
            .await
            .unwrap();
        let run_id = observed
            .get("run_id")
            .or_else(|| {
                observed
                    .get("record")
                    .and_then(|record| record.get("run_id"))
            })
            .unwrap()
            .clone();
        let mut altered = original;
        if let VoyageCommand::Submit { prompt, .. } = &mut altered {
            *prompt = "replacement must not execute".into();
        }
        let changed = client
            .voyage(
                process.session_id,
                process.incarnation,
                VoyageCommand::Resolve {
                    command_id: id,
                    original: Some(Box::new(altered)),
                },
            )
            .await;
        assert!(match changed {
            Err(_) => true,
            Ok(reply) => reply["status"] == "rejected",
        });
        let current = snapshot(&client, &process).await;
        assert_eq!(current["run"]["run_id"], run_id);
        let mut overridden = config.clone();
        overridden.model = "must-not-reconfigure-active".into();
        assert!(
            open(
                &overridden,
                None,
                Some(process.session_id.to_string()),
                true,
                true
            )
            .await
            .is_err()
        );
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
        proxy.client.disconnect();
        assert_eq!(
            snapshot(&client, &process).await["run"]["run_id"],
            run_id,
            "Helm disconnect cancelled voyage"
        );
        provider.release.cancel();
        let saved = terminal(&client, &process).await;
        canonical(&saved, "owned exact uncertain prompt");
        assert_eq!(provider.requests.lock().unwrap().len(), 1);
        assert!(
            !saved["messages"]
                .to_string()
                .contains("replacement must not execute")
        );
    } else {
        panic!("unowned frontend fixture mode");
    }
    vessel.finish(&client).await;
    client.disconnect();
}

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}
#[test]
fn owned_ordinary_frontend_preserves_canonical_history_exact_delivery_and_cleanup() {
    if let Ok(mode) = std::env::var(CHILD) {
        let parent: libc::pid_t = std::env::var("HELM_OWNED_FRONTEND_PARENT_353")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::getppid() }, parent);
        assert_eq!(unsafe { libc::getsid(0) }, unsafe { libc::getpid() });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&runtime, journey(&mode));
        return;
    }
    for mode in [
        "saved",
        "temporary",
        "temporary-failure",
        "branch",
        "lost-reply",
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", TEST, "--nocapture"])
            .env_clear()
            .env(CHILD, mode)
            .env(
                "HELM_OWNED_FRONTEND_PARENT_353",
                std::process::id().to_string(),
            )
            .env("HOME", root.path())
            .env("PATH", "/usr/bin:/bin")
            .current_dir(root.path())
            .stdin(Stdio::null());
        for (key, name) in [
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_STATE_HOME", "state"),
            ("XDG_CACHE_HOME", "cache"),
            ("XDG_RUNTIME_DIR", "runtime"),
        ] {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            command.env(key, path);
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let output = tempfile::tempfile().unwrap();
        command
            .stdout(output.try_clone().unwrap())
            .stderr(output.try_clone().unwrap());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = OwnedChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(120);
        let exit = loop {
            if let Some(exit) = child.0.try_wait().unwrap() {
                break exit;
            }
            assert!(
                Instant::now() < deadline,
                "owned frontend child deadline exceeded in {mode}"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            output.metadata().unwrap().len() <= 1024 * 1024,
            "fixture output exceeded bound"
        );
        if !exit.success() {
            use std::io::{Read, Seek};
            let mut output = output;
            output.rewind().unwrap();
            let mut text = String::new();
            output.take(65536).read_to_string(&mut text).unwrap();
            let retained = root.keep();
            panic!(
                "owned frontend child failed in {mode}; private fixture retained at {}: {text}",
                retained.display()
            );
        }
        assert!(
            owned_pids(root.path(), &binary("voyage")).is_empty(),
            "owned voyage survived fixture cleanup"
        );
        assert!(
            owned_pids(root.path(), &binary("vessel")).is_empty(),
            "owned Vessel survived fixture cleanup"
        );
    }
}
