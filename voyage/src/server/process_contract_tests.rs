//! Real ordinary serve entry points run in one isolated child per lifetime.
//! The child inherits only fixture paths and the caller's coverage filename.
//! Forced fixture teardown is never reported as runtime cleanup evidence.
use super::family_fixture::{Provider, Reply, expiry, socket_call};
use super::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::Duration,
};
use voyage_protocol::process::{ProcessState, RuntimeCommand};

const DRIVER: &str = "VOYAGE_ORDINARY_SERVER_FIXTURE";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Driver {
    mode: String,
    directory: PathBuf,
    session: Uuid,
    incarnation: Uuid,
    workspace: PathBuf,
    config: Option<PathBuf>,
    result: PathBuf,
}
fn private(path: &Path, value: &impl Serialize) {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    serde_json::to_writer(&mut file, value).unwrap();
    file.flush().unwrap();
    file.sync_all().unwrap();
}

#[tokio::test]
async fn ordinary_runtime_child() {
    let Some(path) = std::env::var_os(DRIVER) else {
        return;
    };
    // The production ordinary path is the selected scope, not a root simulation.
    assert_ne!(unsafe { libc::geteuid() }, 0);
    let request: Driver =
        serde_json::from_slice(&bootstrap::read_private_artifact(Path::new(&path)).unwrap())
            .unwrap();
    if request.mode == "models" || request.mode == "observe" {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        use tokio::io::AsyncWriteExt;
        let saved = unsafe { libc::fcntl(1, libc::F_DUPFD_CLOEXEC, 3) };
        assert!(saved >= 0);
        let saved = unsafe { OwnedFd::from_raw_fd(saved) };
        let output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&request.result)
            .unwrap();
        assert_eq!(unsafe { libc::dup2(output.as_raw_fd(), 1) }, 1);
        let result = if request.mode == "models" {
            models::run().await
        } else {
            suspended::run(suspended::Args {
                directory: request.directory,
            })
            .await
        };
        tokio::io::stdout().flush().await.unwrap();
        assert_eq!(unsafe { libc::dup2(saved.as_raw_fd(), 1) }, 1);
        result.unwrap();
        return;
    }
    assert_eq!(request.mode, "serve");
    let result = serve(ServeArgs {
        directory: request.directory,
        session: request.session,
        incarnation: request.incarnation,
        workspace: request.workspace,
        config: request.config,
    })
    .await;
    private(
        &request.result,
        &json!({"success":result.is_ok(),"error":result.err().map(|e|e.to_string())}),
    );
}

struct Process {
    child: Child,
    result: PathBuf,
    log: PathBuf,
}
impl Drop for Process {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            self.child.kill().unwrap();
            self.child.wait().unwrap();
        }
    }
}
impl Process {
    fn diagnostics(&self) -> String {
        // Only the cleared-environment fixture child writes these files.
        let bytes = std::fs::read(&self.log).unwrap_or_default();
        let mut text = String::from_utf8_lossy(&bytes[..bytes.len().min(16_384)]).into_owned();
        if let Ok(result) = std::fs::read(&self.result)
            && let Ok(value) = serde_json::from_slice::<Value>(&result)
        {
            text.push_str(&format!("\nfixture result: {value}"));
        }
        text
    }
    async fn waited(&mut self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    assert!(
                        status.success(),
                        "ordinary child failed: {}",
                        self.diagnostics()
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn exited(&mut self) -> Value {
        self.waited().await;
        serde_json::from_slice(&std::fs::read(&self.result).unwrap()).unwrap()
    }
    async fn frame(&mut self) -> Value {
        self.waited().await;
        let bytes = std::fs::read(&self.result).unwrap();
        let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
        assert_eq!(
            bytes.len(),
            length + 4,
            "helper emitted exactly one bounded frame"
        );
        serde_json::from_slice(&bytes[4..]).unwrap()
    }
    async fn ready(&mut self, directory: &Path, registration: &ProcessRegistration) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                assert!(
                    self.child.try_wait().unwrap().is_none(),
                    "server exited before readiness: {}",
                    self.diagnostics()
                );
                if directory.join("runtime.sock").exists() {
                    let response =
                        socket_call(directory, registration, RuntimeCommand::Health).await;
                    assert!(response.error.is_none());
                    assert!(!response.outcome_unknown);
                    assert_eq!(response.incarnation, registration.incarnation);
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
}

struct Fixture {
    root: tempfile::TempDir,
    directory: PathBuf,
    workspace: PathBuf,
    config: PathBuf,
    registration: ProcessRegistration,
}
impl Fixture {
    fn new(provider: &Provider) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace =
            crate::attachment::journal::prepare_directory(root.path().join("workspace")).unwrap();
        let directory =
            crate::attachment::journal::prepare_directory(root.path().join("runtime")).unwrap();
        let data = crate::attachment::journal::prepare_directory(root.path().join("data")).unwrap();
        crate::attachment::journal::prepare_directory(data.join("helm")).unwrap();
        let registry = crate::accounts::Registry::new(data.join("helm/accounts"));
        let connection = registry
            .add_connection(
                "owned-loopback".into(),
                provider.url.clone(),
                vec![voyage_protocol::accounts::Transport::OpenaiChat],
            )
            .unwrap();
        let account = registry
            .add_api(
                connection.id,
                "owned".into(),
                "Owned synthetic account".into(),
                crate::accounts::ApiKeyInput::Stored("synthetic-local-only-key".into()),
            )
            .unwrap();
        let config = provider.config();
        let binding = voyage_protocol::accounts::AccountBinding {
            account_id: account.id,
            connection_id: connection.id,
            identity_generation: account.identity_generation,
            connection_revision: connection.revision,
            transport: voyage_protocol::accounts::Transport::OpenaiChat,
        };
        let config_path = root.path().join("launch.json");
        // The registry belongs to the child XDG namespace. Capture the safe
        // account-free policy DTO here, then insert its exact child binding.
        // The real child resolves and validates it in that namespace at launch;
        // the parent never changes its process-global environment or registry.
        let mut launch = serde_json::to_value(
            crate::launch_config::LaunchConfig::capture(&config, &workspace).unwrap(),
        )
        .unwrap();
        launch["config"]["account"] = serde_json::to_value(binding).unwrap();
        private(&config_path, &launch);
        let registration = ProcessRegistration {
            protocol: 1,
            session_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: None,
            initialize: None,
            config_path: Some(config_path.clone()),
            token: "f".repeat(64),
            peer_uids: None,
            workspace: workspace.clone(),
            state: ProcessState::Starting,
            name: None,
            executable: None,
        };
        private(&directory.join("registration.json"), &registration);
        Self {
            root,
            directory,
            workspace,
            config: config_path,
            registration,
        }
    }
    fn spawn(
        &self,
        registration: &ProcessRegistration,
        workspace: &Path,
        config: Option<PathBuf>,
    ) -> Process {
        private(&self.directory.join("registration.json"), registration);
        self.spawn_preserving(workspace, config)
    }
    fn spawn_preserving(&self, workspace: &Path, config: Option<PathBuf>) -> Process {
        self.spawn_mode(workspace, config, "serve", None)
    }
    fn spawn_mode(
        &self,
        workspace: &Path,
        config: Option<PathBuf>,
        mode: &str,
        input: Option<Vec<u8>>,
    ) -> Process {
        let id = Uuid::new_v4();
        let result = self.root.path().join(format!("result-{id}.json"));
        let request = self.root.path().join(format!("driver-{id}.json"));
        private(
            &request,
            &Driver {
                mode: mode.into(),
                directory: self.directory.clone(),
                session: self.registration.session_id,
                incarnation: self.registration.incarnation,
                workspace: workspace.into(),
                config,
                result: result.clone(),
            },
        );
        let log_path = self.root.path().join(format!("child-{id}.log"));
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&log_path)
            .unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        let stdin = if let Some(bytes) = input {
            let path = self.root.path().join(format!("input-{id}.bin"));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .unwrap();
            file.write_all(&bytes).unwrap();
            drop(file);
            Stdio::from(std::fs::File::open(path).unwrap())
        } else {
            Stdio::null()
        };
        command
            .args([
                "--exact",
                "server::process_contract_tests::ordinary_runtime_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.root.path().join("home"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env(DRIVER, &request)
            .stdin(stdin)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        Process {
            child: command.spawn().unwrap(),
            result,
            log: log_path,
        }
    }
    async fn call(&self, command: RuntimeCommand) -> voyage_protocol::process::RuntimeResponse {
        socket_call(&self.directory, &self.registration, command).await
    }
    fn evidence(&self) -> Value {
        serde_json::from_slice(&std::fs::read(self.directory.join("stopped.json")).unwrap())
            .unwrap()
    }
}

#[tokio::test]
async fn fresh_serve_authenticates_metadata_and_stop_publishes_exact_cleanup() {
    let provider = Provider::new(vec![]).await;
    let f = Fixture::new(&provider);
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    process.ready(&f.directory, &f.registration).await;
    let id = Uuid::new_v4();
    let rename = RuntimeCommand::Rename {
        command_id: id,
        expected_revision: 0,
        expires_at_ms: expiry(),
        name: "Actual ordinary server".into(),
    };
    let first = f.call(rename.clone()).await;
    assert!(first.error.is_none());
    assert!(!first.outcome_unknown);
    assert_eq!(f.call(rename).await.result, first.result);
    let snapshot = f.call(RuntimeCommand::Snapshot).await;
    assert_eq!(snapshot.result["name"], "Actual ordinary server");
    assert_eq!(snapshot.result["revision"], 1);
    let stop = f.call(RuntimeCommand::Stop).await;
    assert_eq!(stop.result["cleanup"], "pending");
    assert_eq!(process.exited().await["success"], true);
    let evidence = f.evidence();
    assert_eq!(
        evidence["session_id"],
        f.registration.session_id.to_string()
    );
    assert_eq!(
        evidence["incarnation"],
        f.registration.incarnation.to_string()
    );
    assert_eq!(evidence["cleanup_observed"], true);
    assert_eq!(evidence["suspended"], false);
    assert!(!f.directory.join("runtime.sock").exists());
    let journal = Journal::open(f.directory.join("journal")).unwrap();
    assert_eq!(
        journal
            .load_session(f.registration.session_id)
            .unwrap()
            .session
            .name
            .as_deref(),
        Some("Actual ordinary server")
    );
    assert!(provider.requests.lock().await.is_empty());
}

#[tokio::test]
async fn real_completed_run_suspends_and_fresh_process_retains_canonical_configuration() {
    let provider = Provider::new(vec![Reply::held("Actual retained ordinary answer")]).await;
    let mut f = Fixture::new(&provider);
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    process.ready(&f.directory, &f.registration).await;
    let submit = family_fixture::submit(0, "Synthetic fresh process submission");
    let id = submit.mutation_id().unwrap();
    let admitted = f.call(submit.clone()).await;
    assert!(admitted.error.is_none());
    assert_eq!(admitted.result["status"], "accepted");
    provider.wait_requests(1).await;
    assert_eq!(
        f.call(submit).await.result["run_id"],
        admitted.result["run_id"]
    );
    assert_eq!(provider.requests.lock().await.len(), 1);
    provider.release.cancel();
    assert_eq!(process.exited().await["success"], true);
    assert_eq!(f.evidence()["suspended"], true);
    assert_eq!(f.evidence()["cleanup_observed"], true);
    let previous = f.registration.incarnation;
    f.registration.incarnation = Uuid::new_v4();
    f.registration.command_id = Uuid::new_v4();
    f.registration.restart_from = Some(previous);
    // The retained initial configuration is canonical. A changed launch draft
    // cannot silently replace it when this ordinary process resumes.
    private(
        &f.config,
        &json!({"deliberately":"invalid replacement draft"}),
    );
    let mut resumed = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    resumed.ready(&f.directory, &f.registration).await;
    assert!(!f.directory.join("stopped.json").exists());
    let snapshot = f.call(RuntimeCommand::Snapshot).await;
    assert_eq!(snapshot.result["model"], "fixture");
    let history = f
        .call(RuntimeCommand::History {
            offset: 0,
            limit: 20,
            expected_revision: None,
        })
        .await;
    assert!(
        history.result["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["content"] == "Actual retained ordinary answer")
    );
    let receipt = f.call(RuntimeCommand::Receipt { command_id: id }).await;
    assert!(receipt.error.is_none());
    assert_ne!(receipt.result["status"], "unknown");
    f.call(RuntimeCommand::Stop).await;
    assert_eq!(resumed.exited().await["success"], true);
    assert_eq!(provider.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn startup_refuses_wrong_registration_workspace_config_and_bound_projection() {
    let provider = Provider::new(vec![]).await;
    for case in 0..5 {
        let f = Fixture::new(&provider);
        let mut registration = f.registration.clone();
        let mut workspace = f.workspace.clone();
        let mut config = Some(f.config.clone());
        match case {
            0 => registration.session_id = Uuid::new_v4(),
            1 => registration.incarnation = Uuid::new_v4(),
            2 => workspace = f.root.path().into(),
            3 => config = None,
            _ => {
                registration.peer_uids = Some(voyage_protocol::process::ProcessPeerUids {
                    supervisor: 0,
                    runtime: unsafe { libc::geteuid() },
                })
            }
        }
        let mut child = f.spawn(&registration, &workspace, config);
        let result = child.exited().await;
        assert_eq!(result["success"], false, "case {case}");
        assert!(!f.directory.join("runtime.sock").exists());
        assert!(!f.directory.join("journal").exists());
        assert!(provider.requests.lock().await.is_empty());
    }
}

#[tokio::test]
async fn failed_bootstrap_retains_definite_startup_failure_without_an_executor() {
    let provider = Provider::new(vec![]).await;
    let f = Fixture::new(&provider);
    private(
        &f.config,
        &json!({"version":9,"workspace":f.workspace,"config":{}}),
    );
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    let result = process.exited().await;
    assert_eq!(result["success"], false);
    let evidence = f.evidence();
    assert_eq!(evidence["startup_failed"], true);
    assert_eq!(evidence["cleanup_observed"], true);
    assert_eq!(
        evidence["session_id"],
        f.registration.session_id.to_string()
    );
    assert!(!f.directory.join("runtime.sock").exists());
    assert!(
        Journal::open(f.directory.join("journal"))
            .unwrap()
            .load_session(f.registration.session_id)
            .is_err()
    );
    assert!(provider.requests.lock().await.is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn registration_change_while_waiting_for_startup_fence_is_not_admitted() {
    let provider = Provider::new(vec![]).await;
    let f = Fixture::new(&provider);
    let lock =
        crate::attachment::journal::open_private_file(&f.directory.join("startup.lock")).unwrap();
    lock.try_lock().unwrap();
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    let descriptor = std::path::PathBuf::from(format!("/proc/{}/fd", process.child.id()));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(process.child.try_wait().unwrap().is_none());
            if std::fs::read_dir(&descriptor)
                .unwrap()
                .flatten()
                .any(|entry| {
                    std::fs::read_link(entry.path())
                        .is_ok_and(|path| path == f.directory.join("startup.lock"))
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut changed = f.registration.clone();
    changed.token = "a".repeat(64);
    private(&f.directory.join("registration.json"), &changed);
    drop(lock);
    let result = process.exited().await;
    assert_eq!(result["success"], false);
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("changed during startup")
    );
    assert!(!f.directory.join("journal").exists());
    assert!(!f.directory.join("runtime.sock").exists());
}

#[tokio::test]
async fn malformed_private_registration_and_overlong_socket_never_initialize_history() {
    let provider = Provider::new(vec![]).await;
    for case in 0..3 {
        let mut f = Fixture::new(&provider);
        match case {
            0 => private(
                &f.directory.join("registration.json"),
                &json!({"invalid":true}),
            ),
            1 => {
                let mut bad = f.registration.clone();
                bad.token = "short".into();
                private(&f.directory.join("registration.json"), &bad);
            }
            _ => {
                f.directory = crate::attachment::journal::prepare_directory(
                    f.root.path().join("r".repeat(90)),
                )
                .unwrap();
            }
        }
        let mut process = f.spawn_preserving(&f.workspace, Some(f.config.clone()));
        assert_eq!(process.exited().await["success"], false);
        assert!(!f.directory.join("journal").exists());
    }
}

fn frame(value: &impl Serialize) -> Vec<u8> {
    let bytes = serde_json::to_vec(value).unwrap();
    let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
    frame.extend(bytes);
    frame
}

#[tokio::test]
async fn sessionless_model_helper_reports_safe_metadata_and_typed_refusals_without_a_journal() {
    use voyage_protocol::{model_discovery::Failure, vessel::VesselCommand};
    let provider = Provider::new(vec![]).await;
    let f = Fixture::new(&provider);
    let mut config = provider.config();
    config.model = "selected-but-not-listed".into();
    let launch = serde_json::to_value(
        crate::launch_config::LaunchConfig::capture(&config, &f.workspace).unwrap(),
    )
    .unwrap();
    let request = VesselCommand::DiscoverModels {
        workspace: f.workspace.clone(),
        configuration: launch.clone(),
    };
    let mut process = f.spawn_mode(&f.workspace, None, "models", Some(frame(&request)));
    let models = process.frame().await;
    assert!(
        models
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "fixture")
    );
    assert!(
        models
            .as_array()
            .unwrap()
            .iter()
            .any(|model| model["id"] == "selected-but-not-listed")
    );
    for (request, expected) in [
        (VesselCommand::Capabilities, Failure::Configuration),
        (
            VesselCommand::DiscoverModels {
                workspace: "relative".into(),
                configuration: launch.clone(),
            },
            Failure::Configuration,
        ),
        (
            VesselCommand::DiscoverModels {
                workspace: f.root.path().join("missing"),
                configuration: launch.clone(),
            },
            Failure::Workspace,
        ),
        (
            VesselCommand::DiscoverModels {
                workspace: f.workspace.clone(),
                configuration: json!({"invalid":true}),
            },
            Failure::Configuration,
        ),
        (
            VesselCommand::DiscoverModels {
                workspace: f.workspace.clone(),
                configuration: json!({"oversized":"x".repeat(1024*1024+1)}),
            },
            Failure::Configuration,
        ),
    ] {
        let mut process = f.spawn_mode(&f.workspace, None, "models", Some(frame(&request)));
        let result = process.frame().await;
        assert_eq!(
            result["model_catalog_error"],
            serde_json::to_value(expected).unwrap()
        );
    }
    let mut process = f.spawn_mode(
        &f.workspace,
        None,
        "models",
        Some(vec![0, 0, 0, 3, b'?', b'?', b'?']),
    );
    assert_eq!(
        process.frame().await["model_catalog_error"],
        serde_json::to_value(Failure::Configuration).unwrap()
    );
    assert!(!f.directory.join("journal").exists());
    assert!(!f.directory.join("runtime.sock").exists());
    assert!(provider.requests.lock().await.is_empty());
}

#[tokio::test]
async fn model_discovery_auth_rate_invalid_display_and_timeout_errors_never_expose_provider_body() {
    use voyage_protocol::{model_discovery::Failure, vessel::VesselCommand};
    for (reply, expected) in [
        (Reply::error(401), Failure::Authentication),
        (Reply::error(429), Failure::RateLimit),
        (
            Reply {
                status: 200,
                body: "not model JSON".into(),
                held: false,
            },
            Failure::InvalidResponse,
        ),
        (
            Reply {
                status: 200,
                body: json!({"data":[{"id":"untrusted\u{1b}[31m"}]}).to_string(),
                held: false,
            },
            Failure::InvalidResponse,
        ),
    ] {
        let provider = Provider::with_models(vec![], reply).await;
        let f = Fixture::new(&provider);
        let request = VesselCommand::DiscoverModels {
            workspace: f.workspace.clone(),
            configuration: serde_json::to_value(
                crate::launch_config::LaunchConfig::capture(&provider.config(), &f.workspace)
                    .unwrap(),
            )
            .unwrap(),
        };
        let mut process = f.spawn_mode(&f.workspace, None, "models", Some(frame(&request)));
        let result = process.frame().await;
        assert_eq!(
            result["model_catalog_error"],
            serde_json::to_value(expected).unwrap()
        );
        assert!(!result.to_string().contains("owned synthetic refusal"));
        assert!(provider.requests.lock().await.is_empty());
    }
    let provider = Provider::new(vec![]).await;
    let f = Fixture::new(&provider);
    let mut unsafe_display = provider.config();
    unsafe_display.model = "requested\u{202e}model".into();
    let request = VesselCommand::DiscoverModels {
        workspace: f.workspace.clone(),
        configuration: serde_json::to_value(
            crate::launch_config::LaunchConfig::capture(&unsafe_display, &f.workspace).unwrap(),
        )
        .unwrap(),
    };
    let mut process = f.spawn_mode(&f.workspace, None, "models", Some(frame(&request)));
    assert_eq!(
        process.frame().await["model_catalog_error"],
        serde_json::to_value(Failure::DisplayValidation).unwrap()
    );
    // A zero-duration timeout may poll a ready future to completion. Hold the
    // owned model response instead, so this case requires actual cancellation
    // of an outstanding catalogue request rather than a scheduling assumption.
    let held = Provider::with_models(
        vec![],
        Reply {
            status: 200,
            body: json!({"data":[{"id":"fixture"}]}).to_string(),
            held: true,
        },
    )
    .await;
    let mut config = held.config();
    config.command_timeout_secs = 2;
    let result = super::models::discover(&config, &f.workspace)
        .await
        .unwrap_err();
    assert!(result.to_string().contains("model_catalog:timeout"));
    held.wait_disconnected().await;
    assert!(held.requests.lock().await.is_empty());
    assert!(provider.requests.lock().await.is_empty());
}

#[tokio::test]
async fn saved_observer_resolves_without_executor_and_delayed_delivery_remains_fenced_on_resume() {
    use voyage_protocol::process::RuntimeRequest;
    let provider = Provider::new(vec![]).await;
    let mut f = Fixture::new(&provider);
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    process.ready(&f.directory, &f.registration).await;
    f.call(RuntimeCommand::Stop).await;
    assert_eq!(process.exited().await["success"], true);
    let missing = Uuid::new_v4();
    let original = RuntimeCommand::Rename {
        command_id: missing,
        expected_revision: 0,
        expires_at_ms: expiry(),
        name: "must remain unadmitted".into(),
    };
    let request = |command| RuntimeRequest {
        protocol: 1,
        session_id: f.registration.session_id,
        incarnation: f.registration.incarnation,
        token: f.registration.token.clone(),
        authorization: None,
        scope_authority: None,
        command,
    };
    let mut observer = f.spawn_mode(
        &f.workspace,
        None,
        "observe",
        Some(frame(&request(RuntimeCommand::Resolve {
            command_id: missing,
            original: Some(Box::new(original.clone())),
        }))),
    );
    let result = observer.frame().await;
    assert!(result["error"].is_null());
    assert_eq!(result["outcome_unknown"], false);
    assert_eq!(result["result"]["status"], "not_admitted");
    assert!(!f.directory.join("runtime.sock").exists());
    let mut invalid = request(RuntimeCommand::Snapshot);
    invalid.token = "wrong-owner-token".into();
    let mut observer = f.spawn_mode(&f.workspace, None, "observe", Some(frame(&invalid)));
    let refusal = observer.frame().await;
    assert!(refusal["error"].is_string());
    assert_eq!(refusal["outcome_unknown"], false);
    let mut observer = f.spawn_mode(
        &f.workspace,
        None,
        "observe",
        Some(frame(&request(RuntimeCommand::Snapshot))),
    );
    let saved = observer.frame().await;
    assert_eq!(saved["result"]["suspended"], false);
    assert_eq!(saved["result"]["revision"], 0);
    f.registration.incarnation = Uuid::new_v4();
    f.registration.command_id = Uuid::new_v4();
    let mut resumed = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    resumed.ready(&f.directory, &f.registration).await;
    let rejected = f.call(original).await;
    assert!(rejected.error.is_some());
    assert!(!rejected.outcome_unknown);
    assert_eq!(rejected.result["status"], "not_admitted");
    f.call(RuntimeCommand::Stop).await;
    assert_eq!(resumed.exited().await["success"], true);
    assert!(provider.requests.lock().await.is_empty());
}

#[tokio::test]
async fn ordinary_json_and_managed_imports_preserve_identity_and_finalize_without_source_replay() {
    use sha2::{Digest, Sha256};
    use voyage_protocol::process::RuntimeInitialization;
    let provider = Provider::new(vec![]).await;
    for managed in [false, true] {
        let mut f = Fixture::new(&provider);
        let source =
            crate::attachment::journal::prepare_directory(f.root.path().join("legacy")).unwrap();
        let mut session = Session::new(f.workspace.clone(), "legacy-fixture".into());
        session.id = f.registration.session_id;
        session.name = Some("Retained legacy identity".into());
        let transfer = Uuid::new_v4();
        if managed {
            Journal::open(source.join("journal"))
                .unwrap()
                .create_session(&session)
                .unwrap();
            LocalActorStore::open(&source).unwrap().identity().unwrap();
            f.registration.initialize = Some(RuntimeInitialization::ManagedImport {
                transfer_id: transfer,
                source_directory: source.clone(),
                expected_revision: 0,
            });
        } else {
            private(&source.join(format!("{}.json", session.id)), &session);
            let bytes = std::fs::read(source.join(format!("{}.json", session.id))).unwrap();
            f.registration.initialize = Some(RuntimeInitialization::Import {
                transfer_id: transfer,
                source_directory: source.clone(),
                expected_revision: 0,
                source_sha256: hex::encode(Sha256::digest(bytes)),
            });
        }
        let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
        process.ready(&f.directory, &f.registration).await;
        assert_eq!(
            f.call(RuntimeCommand::Snapshot).await.result["name"],
            "Retained legacy identity"
        );
        f.call(RuntimeCommand::Stop).await;
        assert_eq!(process.exited().await["success"], true);
        assert!(
            Journal::open(f.directory.join("journal"))
                .unwrap()
                .json_import_finalized(session.id, transfer)
                .unwrap()
        );
        f.registration.incarnation = Uuid::new_v4();
        f.registration.command_id = Uuid::new_v4();
        let mut resumed = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
        resumed.ready(&f.directory, &f.registration).await;
        assert_eq!(
            f.call(RuntimeCommand::Snapshot).await.result["session_id"],
            session.id.to_string()
        );
        f.call(RuntimeCommand::Stop).await;
        assert_eq!(resumed.exited().await["success"], true);
        assert!(provider.requests.lock().await.is_empty());
    }
}

#[tokio::test]
async fn actual_active_serve_stages_settings_exposes_controls_and_observes_cancelled_provider_cleanup()
 {
    let provider = Provider::new(vec![Reply::held("must be cancelled")]).await;
    let f = Fixture::new(&provider);
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    process.ready(&f.directory, &f.registration).await;
    let admitted = f
        .call(family_fixture::submit(0, "Held ordinary server activity"))
        .await;
    assert!(admitted.error.is_none());
    let run: Uuid = admitted.result["run_id"].as_str().unwrap().parse().unwrap();
    provider.wait_requests(1).await;
    for section in [
        "tools",
        "policy",
        "models",
        "todos",
        "subagents",
        "subagents_archive",
        "terminals",
        "workflows",
        "host_resources",
    ] {
        let response = f
            .call(RuntimeCommand::Controls {
                run_id: Some(run),
                section: section.into(),
            })
            .await;
        assert!(
            response.error.is_none(),
            "owned control section {section}: {response:?}"
        );
        assert!(!response.outcome_unknown);
    }
    let current = f.call(RuntimeCommand::Snapshot).await;
    assert_eq!(current.result["inference_current"]["model"], "fixture");
    assert_eq!(current.result["inference_next_turn"], true);
    let revision = current.result["revision"].as_u64().unwrap();
    let inference = RuntimeCommand::SetInference {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        model: "fixture-next".into(),
        reasoning_effort: None,
        service_tier: None,
    };
    let updated = f.call(inference).await;
    assert!(updated.error.is_none());
    assert!(!updated.outcome_unknown);
    let current = f.call(RuntimeCommand::Snapshot).await;
    assert_eq!(current.result["inference_current"]["model"], "fixture");
    assert_eq!(current.result["inference"]["model"], "fixture-next");
    let launch: Value = serde_json::from_slice(&std::fs::read(&f.config).unwrap()).unwrap();
    let account = serde_json::from_value(launch["config"]["account"].clone()).unwrap();
    let selected = f
        .call(RuntimeCommand::SetAccountInference {
            command_id: Uuid::new_v4(),
            expected_revision: current.result["revision"].as_u64().unwrap(),
            expires_at_ms: expiry(),
            account,
            model: "fixture".into(),
            reasoning_effort: None,
            service_tier: None,
        })
        .await;
    assert!(selected.error.is_none());
    assert!(!selected.outcome_unknown);
    let current = f.call(RuntimeCommand::Snapshot).await;
    let cancelled = f
        .call(RuntimeCommand::Cancel {
            command_id: Uuid::new_v4(),
            expected_revision: current.result["revision"].as_u64().unwrap(),
            expires_at_ms: expiry(),
            run_id: run,
        })
        .await;
    assert!(cancelled.error.is_none());
    assert!(!cancelled.outcome_unknown);
    provider.wait_disconnected().await;
    assert_eq!(process.exited().await["success"], true);
    assert_eq!(f.evidence()["cleanup_observed"], true);
    let saved = Journal::open(f.directory.join("journal"))
        .unwrap()
        .load_session(f.registration.session_id)
        .unwrap();
    assert!(
        !saved
            .session
            .messages
            .iter()
            .any(|m| m.content == "must be cancelled")
    );
    assert_eq!(provider.requests.lock().await.len(), 1);
}

#[tokio::test]
async fn live_owner_exclusion_does_not_publish_false_cleanup_for_a_second_serve() {
    let provider = Provider::new(vec![Reply::held("held owner")]).await;
    let f = Fixture::new(&provider);
    let mut launch: Value = serde_json::from_slice(&std::fs::read(&f.config).unwrap()).unwrap();
    launch["config"]["provider_response_timeout_ms"] = json!(30_000);
    private(&f.config, &launch);
    let mut first = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    first.ready(&f.directory, &f.registration).await;
    let admitted = f
        .call(family_fixture::submit(
            0,
            "Keep the original exclusive execution owner",
        ))
        .await;
    assert!(admitted.error.is_none());
    provider.wait_requests(1).await;
    let mut second = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    let rejected = second.exited().await;
    assert_eq!(rejected["success"], false);
    assert!(f.directory.join("runtime.sock").exists());
    assert!(!f.directory.join("stopped.json").exists());
    assert!(f.call(RuntimeCommand::Health).await.error.is_none());
    let marker: Value = serde_json::from_slice(
        &std::fs::read(
            f.directory
                .join(format!("startup-{}.json", f.registration.incarnation)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(marker["stage"], "execution_owner");
    f.call(RuntimeCommand::Stop).await;
    provider.wait_disconnected().await;
    assert_eq!(first.exited().await["success"], true);
    assert_eq!(f.evidence()["cleanup_observed"], true);
}

#[tokio::test]
async fn missing_default_account_refuses_creation_and_required_sandbox_never_starts_a_browser_fallback()
 {
    let provider = Provider::new(vec![]).await;
    let mut f = Fixture::new(&provider);
    f.registration.config_path = None;
    let mut process = f.spawn(&f.registration, &f.workspace, None);
    let result = process.exited().await;
    assert_eq!(result["success"], false);
    assert!(
        result["error"]
            .as_str()
            .unwrap()
            .contains("default_account_required")
    );
    assert_eq!(f.evidence()["startup_failed"], true);
    let f = Fixture::new(&provider);
    let mut launch: Value = serde_json::from_slice(&std::fs::read(&f.config).unwrap()).unwrap();
    launch["config"]["sandbox"]["mode"] =
        serde_json::to_value(crate::sandbox::Mode::Required).unwrap();
    // Reach the required-sandbox missing-worker path, not the earlier
    // read-only policy refusal. No browser executor is admitted by this setting.
    launch["config"]["access"] = json!("unrestricted");
    private(&f.config, &launch);
    let mut process = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
    process.ready(&f.directory, &f.registration).await;
    let socket = voyage_protocol::host_browser::HostBrowserSocket {
        socket_id: Uuid::new_v4(),
    };
    let status = || RuntimeCommand::HostBrowser {
        operation: voyage_protocol::host_browser::HostBrowserOperation::Status {},
        socket,
    };
    let before = f.call(status()).await;
    assert!(before.error.is_none());
    assert!(!before.outcome_unknown);
    assert_eq!(before.result["status"]["available"], false);
    assert_eq!(before.result["status"]["running"], false);
    let canonical_before = f.call(RuntimeCommand::Snapshot).await;
    assert!(canonical_before.error.is_none());
    assert_eq!(canonical_before.result["access"], "unrestricted");
    let command_id = Uuid::new_v4();
    let response = f
        .call(RuntimeCommand::HostBrowser {
            operation: voyage_protocol::host_browser::HostBrowserOperation::Start {
                command_id,
                expected_revision: 0,
                incarnation: f.registration.incarnation,
            },
            socket,
        })
        .await;
    assert!(
        response
            .error
            .as_deref()
            .unwrap()
            .contains("host browser unavailable")
    );
    // Browser mutations retain their own conservative receipt. Transport does
    // not turn this returned error into a positively fenced non-admission.
    assert!(response.outcome_unknown);
    let receipt = f
        .call(RuntimeCommand::HostBrowser {
            operation: voyage_protocol::host_browser::HostBrowserOperation::Receipt { command_id },
            socket,
        })
        .await;
    assert!(receipt.error.is_none());
    assert!(!receipt.outcome_unknown);
    assert_eq!(
        receipt.result["receipt"]["command_id"],
        command_id.to_string()
    );
    assert_eq!(receipt.result["receipt"]["state"], "unknown");
    let after = f.call(status()).await;
    assert!(after.error.is_none());
    assert_eq!(after.result["status"]["available"], false);
    assert_eq!(after.result["status"]["running"], false);
    let canonical_after = f.call(RuntimeCommand::Snapshot).await;
    assert!(canonical_after.error.is_none());
    assert_eq!(
        canonical_after.result["revision"],
        canonical_before.result["revision"]
    );
    assert_eq!(
        canonical_after.result["messages"],
        canonical_before.result["messages"]
    );
    assert!(canonical_after.result["run"].is_null());
    assert_eq!(canonical_after.result["session_resources"], json!([]));
    assert!(
        !f.directory
            .join("journal/host-browser/worker.lock")
            .exists()
    );
    assert!(
        !f.root
            .path()
            .join("data/helm/host-browser-capacity")
            .exists()
    );
    f.call(RuntimeCommand::Stop).await;
    assert_eq!(process.exited().await["success"], true);
    assert_eq!(f.evidence()["cleanup_observed"], true);
    assert!(provider.requests.lock().await.is_empty());
}

#[tokio::test]
async fn branch_and_portable_transfer_start_from_exact_local_initialization_and_preserve_source_fences()
 {
    use voyage_protocol::process::RuntimeInitialization;
    let provider = Provider::new(vec![]).await;
    for transfer in [false, true] {
        let (_source_root, source_state) = super::tests::fixture().await;
        *source_state.config.write().await = provider.config();
        let mut f = Fixture::new(&provider);
        if transfer {
            f.registration.session_id = source_state.registration.session_id;
            let command = RuntimeCommand::Relinquish {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: expiry(),
                transfer_id: Uuid::new_v4(),
                destination_vessel_id: Uuid::new_v4(),
                prepare_digest: "b".repeat(64),
            };
            let receipt = family_fixture::call(&source_state, command).await.unwrap();
            let id: Uuid = receipt["transfer_id"].as_str().unwrap().parse().unwrap();
            let artifact = source_state
                .directory
                .join("transfers")
                .join(format!("{id}.json"));
            f.registration.initialize = Some(RuntimeInitialization::Transfer {
                transfer_id: id,
                artifact_path: artifact,
                sha256: receipt["artifact_sha256"].as_str().unwrap().into(),
                prepare_digest: "b".repeat(64),
                generation: receipt["generation"].as_u64().unwrap(),
            });
            assert!(
                !source_state.owner.process_snapshot().await.unwrap()["lifecycle"]["transfer_id"]
                    .is_null()
            );
        } else {
            f.workspace = source_state.registration.workspace.clone();
            f.registration.workspace = f.workspace.clone();
            let command = RuntimeCommand::Branch {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: expiry(),
                branch_id: f.registration.session_id,
                name: Some("Initialized ordinary branch".into()),
                through_message: None,
            };
            let id = command.mutation_id().unwrap();
            family_fixture::call(&source_state, command).await.unwrap();
            f.registration.initialize = Some(RuntimeInitialization::Branch {
                source_directory: source_state.directory.clone(),
                source_session_id: source_state.registration.session_id,
                source_command_id: id,
                branch_id: f.registration.session_id,
            });
            private(
                &f.config,
                &json!({"invalid":"frozen branch settings must win"}),
            );
        }
        let mut child = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
        child.ready(&f.directory, &f.registration).await;
        let snapshot = f.call(RuntimeCommand::Snapshot).await;
        assert!(snapshot.error.is_none());
        assert_eq!(
            snapshot.result["session_id"],
            f.registration.session_id.to_string()
        );
        if !transfer {
            assert_eq!(snapshot.result["name"], "Initialized ordinary branch");
        }
        f.call(RuntimeCommand::Stop).await;
        assert_eq!(child.exited().await["success"], true);
        assert!(provider.requests.lock().await.is_empty());
    }
}

#[tokio::test]
async fn portable_participant_initialization_intersects_parent_limits_without_broadening_host_policy()
 {
    use voyage_protocol::process::{ParticipantPolicy, RuntimeInitialization};
    let provider = Provider::new(vec![]).await;
    for invalid_parent in [false, true] {
        let mut f = Fixture::new(&provider);
        let parent = if invalid_parent {
            f.registration.session_id
        } else {
            Uuid::new_v4()
        };
        f.registration.initialize = Some(RuntimeInitialization::Participant {
            assignment_id: Uuid::new_v4(),
            parent_vessel_id: Uuid::new_v4(),
            parent_session_id: parent,
            parent_run_id: Uuid::new_v4(),
            policy: ParticipantPolicy {
                access: "read-only".into(),
                legacy_deny_commands: vec![],
                inherit_env: vec!["PATH".into()],
                github_enabled: false,
                timeout_secs: 2,
                max_output_bytes: 2048,
                max_subagents: 1,
            },
        });
        let mut child = f.spawn(&f.registration, &f.workspace, Some(f.config.clone()));
        if invalid_parent {
            assert_eq!(child.exited().await["success"], false);
            assert_eq!(f.evidence()["startup_failed"], true);
        } else {
            child.ready(&f.directory, &f.registration).await;
            let snapshot = f.call(RuntimeCommand::Snapshot).await;
            assert_eq!(snapshot.result["access"], "read-only");
            let policy = f
                .call(RuntimeCommand::Controls {
                    run_id: None,
                    section: "policy".into(),
                })
                .await;
            assert!(policy.error.is_none());
            f.call(RuntimeCommand::Stop).await;
            assert_eq!(child.exited().await["success"], true);
            assert_eq!(
                Journal::open(f.directory.join("journal"))
                    .unwrap()
                    .load_session(f.registration.session_id)
                    .unwrap()
                    .session
                    .parent_id,
                Some(parent)
            );
        }
        assert!(provider.requests.lock().await.is_empty());
    }
}
