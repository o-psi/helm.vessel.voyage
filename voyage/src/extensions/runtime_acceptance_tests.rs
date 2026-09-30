//! Offline integration of the actual ExtensionTool with a scripted SDK peer.
//! The adapter owns only a Tokio task and a private fixture ledger reservation;
//! it never spawns executable code and proves no native isolation behavior.
use super::*;
use std::sync::atomic::AtomicUsize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const ROOT: &str = "VOYAGE_EXTENSION_ACCEPTANCE_ROOT";
const BINDING: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[derive(Default)]
struct Counts {
    launches: AtomicUsize,
    cleanup: AtomicUsize,
}
struct Script {
    value: Value,
    progress: Vec<String>,
    observe: bool,
    wrong_identity: bool,
    action: String,
    counts: Arc<Counts>,
}
struct FixtureLease {
    task: Option<tokio::task::JoinHandle<Result<()>>>,
    reservation: crate::host_resources::extensions::ExtensionReservation,
    observe: bool,
    counts: Arc<Counts>,
}
#[async_trait]
impl sdk::Lease for FixtureLease {
    async fn terminate_and_observe(&mut self) -> sdk::Cleanup {
        self.counts.cleanup.fetch_add(1, Ordering::SeqCst);
        let drained = if let Some(task) = self.task.take() {
            matches!(
                tokio::time::timeout(Duration::from_secs(2), task).await,
                Ok(Ok(Ok(())))
            )
        } else {
            false
        };
        // This adapter has positively never launched an OS process. Only actual
        // task completion permits release; EOF/handshake alone does not do so.
        if drained && self.observe && self.reservation.release_observed().is_ok() {
            sdk::Cleanup::Observed
        } else {
            sdk::Cleanup::Pending
        }
    }
}
async fn read<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    let mut line = String::new();
    ensure!(
        reader.read_line(&mut line).await? > 0 && line.len() < 65536,
        "fixture peer frame unavailable"
    );
    Ok(serde_json::from_str(&line)?)
}
async fn write<W: tokio::io::AsyncWrite + Unpin>(writer: &mut W, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}
#[async_trait]
impl sdk::LaunchAdapter for Script {
    async fn launch(&self, identity: &sdk::Identity, _: Instant) -> Result<sdk::Launched> {
        self.counts.launches.fetch_add(1, Ordering::SeqCst);
        let reservation = crate::host_resources::extensions::ExtensionReservation::acquire(
            BINDING,
            &identity.digest,
            identity.invocation,
            identity.run,
            &self.action,
        )?;
        let (host, peer) = tokio::io::duplex(65536);
        let value = self.value.clone();
        let progress = self.progress.clone();
        let wrong = self.wrong_identity;
        let task = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(peer);
            let mut reader = BufReader::new(reader);
            let mut initialize = read(&mut reader).await?;
            initialize["type"] = json!("initialized");
            write(&mut writer, &initialize).await?;
            let call = read(&mut reader).await?;
            for text in progress {
                write(
                    &mut writer,
                    &json!({"type":"progress","invocation":call["invocation"],"text":text}),
                )
                .await?;
            }
            let invocation = if wrong {
                json!(Uuid::new_v4())
            } else {
                call["invocation"].clone()
            };
            write(
                &mut writer,
                &json!({"type":"result","invocation":invocation,"value":value}),
            )
            .await?;
            loop {
                let control = read(&mut reader).await?;
                if control["type"] == "shutdown" {
                    write(&mut writer, &json!({"type":"shutdown_ack"})).await?;
                    writer.shutdown().await?;
                    return Ok(());
                }
                ensure!(
                    control["type"] == "cancel",
                    "unexpected fixture peer command"
                );
            }
        });
        let (reader, writer) = tokio::io::split(host);
        Ok(sdk::Launched {
            reader: Box::new(reader),
            writer: Box::new(writer),
            lease: Box::new(FixtureLease {
                task: Some(task),
                reservation,
                observe: self.observe,
                counts: self.counts.clone(),
            }),
        })
    }
}
fn tool(
    kind: sdk::Kind,
    value: Value,
    progress: Vec<String>,
    observe: bool,
    wrong_identity: bool,
    output_schema: Value,
) -> (ExtensionTool, Arc<Counts>) {
    let counts = Arc::new(Counts::default());
    let (field, remote, prefix, action) = match kind {
        sdk::Kind::Tool => ("tools", "fixture", "ext", "tool"),
        sdk::Kind::Command => ("commands", "fixture", "extcmd", "command"),
        sdk::Kind::Lifecycle => ("lifecycle", "run_start", "extlife", "lifecycle"),
    };
    let input_schema = json!({"type":"object"});
    let mut definitions = json!({"tools":[],"commands":[],"lifecycle":[]});
    definitions[field] = json!([{"name":remote,"description":"owned offline peer","input_schema":input_schema,"output_schema":output_schema}]);
    let capabilities = vec!["execute".into()];
    let executor = sdk::Executor::new(
        definitions.clone(),
        capabilities.clone(),
        Arc::new(Script {
            value,
            progress,
            observe,
            wrong_identity,
            action: format!("{action}.{remote}"),
            counts: counts.clone(),
        }),
    )
    .unwrap();
    let snapshot = Arc::new(ExecutableSnapshot {
        scope: super::super::catalog::Scope::Project,
        digest: "a".repeat(64),
        archive: super::super::executable::Archive {
            manifest: super::super::executable::Manifest {
                format: 2,
                id: "fixture".into(),
                version: "1.0.0".into(),
                voyage: env!("CARGO_PKG_VERSION").rsplit_once('.').unwrap().0.into(),
                protocol: 1,
                platform: "linux-x86_64".into(),
                runtime: "static-elf".into(),
                entrypoint: "fixture".into(),
                contents: vec![],
                capabilities,
                definitions,
            },
            files: BTreeMap::new(),
        },
    });
    (
        ExtensionTool {
            manager: Arc::new(Manager::default()),
            definition: crate::model::ToolDefinition {
                name: format!("{prefix}_fixture_{remote}"),
                description: "offline fixture".into(),
                input_schema,
                output_schema: Some(output_schema),
                annotations: None,
            },
            remote: remote.into(),
            kind,
            snapshot,
            executor,
            contexts: Arc::new(Mutex::new(BTreeMap::new())),
        },
        counts,
    )
}
fn context(workspace: &std::path::Path, access: crate::config::AccessMode) -> ToolContext {
    let mut context = crate::tools::reliability_tests::context(workspace);
    context.policy = Arc::new(
        crate::policy::Policy::new(
            &crate::Config {
                access: Some(access),
                ..Default::default()
            },
            workspace.to_owned(),
        )
        .unwrap(),
    );
    context.redactor = Arc::new(crate::tools::Redactor::new(["private-token".into()]));
    context
}
fn latest() -> Value {
    crate::host_resources::extensions::status_at(BINDING, &crate::config::default_data_dir())
        .unwrap()["latest"][0]
        .clone()
}

#[test]
fn output_acceptance_child() {
    let Some(root) = std::env::var_os(ROOT) else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(crate::config::default_data_dir().starts_with(root.join("data")));
    let workspace = root.join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    crate::host_resources::set_process_scope(Uuid::new_v4(), Uuid::new_v4()).unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        for kind in [sdk::Kind::Tool, sdk::Kind::Command, sdk::Kind::Lifecycle] {
            let context = context(&workspace, crate::config::AccessMode::Unrestricted);
            let (tool, counts) = tool(
                kind,
                json!({"answer":"safe 🌊"}),
                vec!["progress\u{1b}\u{7}\n".into()],
                true,
                false,
                json!({"type":"object"}),
            );
            let output = tool.execute_output(json!({}), &context).await.unwrap();
            assert_eq!(output.structured_content, Some(json!({"answer":"safe 🌊"})));
            assert!(output.text_fallback().contains("progress\n"));
            assert!(!output.text_fallback().contains('\u{1b}'));
            assert!(tool.contexts.lock().unwrap().is_empty());
            assert_eq!(counts.launches.load(Ordering::SeqCst), 1);
            assert_eq!(counts.cleanup.load(Ordering::SeqCst), 1);
            let receipt = latest();
            assert_eq!(receipt["run"], context.execution_id.to_string());
            assert_eq!(receipt["cleanup"], "observed");
            assert_eq!(receipt["outcome"], "succeeded");
            assert_eq!(
                receipt["action"],
                match kind {
                    sdk::Kind::Tool => "tool.fixture",
                    sdk::Kind::Command => "command.fixture",
                    sdk::Kind::Lifecycle => "lifecycle.run_start",
                }
            );
            assert_eq!(tool.executor.shutdown().await, sdk::Cleanup::Observed);
        }
        // Policy/secret refusals occur before the SDK adapter and host ledger.
        for (access, arguments) in [
            (crate::config::AccessMode::ReadOnly, json!({})),
            (crate::config::AccessMode::Approval, json!({})),
            (
                crate::config::AccessMode::Unrestricted,
                json!({"value":"private-token"}),
            ),
        ] {
            let context = context(&workspace, access);
            let (tool, counts) = tool(
                sdk::Kind::Tool,
                json!({"answer":"safe"}),
                vec![],
                true,
                false,
                json!(true),
            );
            assert!(tool.execute_output(arguments, &context).await.is_err());
            assert_eq!(counts.launches.load(Ordering::SeqCst), 0);
            assert!(tool.contexts.lock().unwrap().is_empty());
            assert_eq!(tool.executor.shutdown().await, sdk::Cleanup::Observed);
        }
        // Protocol, typed-output and confidentiality failures all quarantine this
        // exact package; a second call cannot launch another effect.
        for (value, progress, observe, wrong, schema, budget) in [
            (
                json!({"answer":"safe"}),
                vec![],
                true,
                true,
                json!(true),
                4096,
            ),
            (
                json!("schema mismatch"),
                vec![],
                true,
                false,
                json!({"type":"object"}),
                4096,
            ),
            (
                json!({"answer":"token"}),
                vec!["private-".into()],
                true,
                false,
                json!(true),
                4096,
            ),
            (
                json!({"private-token":"key must be withheld"}),
                vec![],
                true,
                false,
                json!(true),
                4096,
            ),
            (
                json!({"answer":"safe"}),
                vec![],
                false,
                false,
                json!(true),
                4096,
            ),
            (
                json!({"answer":"x".repeat(512)}),
                vec![],
                true,
                false,
                json!(true),
                128,
            ),
        ] {
            let mut context = context(&workspace, crate::config::AccessMode::Unrestricted);
            context.max_output_bytes = budget;
            let (tool, counts) = tool(sdk::Kind::Tool, value, progress, observe, wrong, schema);
            let error = tool
                .execute_output(json!({}), &context)
                .await
                .unwrap_err()
                .to_string();
            assert!(!error.contains("private-token"));
            assert_eq!(counts.launches.load(Ordering::SeqCst), 1);
            assert_eq!(counts.cleanup.load(Ordering::SeqCst), 1);
            assert!(tool.contexts.lock().unwrap().is_empty());
            assert!(tool.execute_output(json!({}), &context).await.is_err());
            assert_eq!(counts.launches.load(Ordering::SeqCst), 1);
            assert_eq!(latest()["outcome"], "failed");
            assert_eq!(
                latest()["cleanup"],
                if observe { "observed" } else { "pending" }
            );
            assert_eq!(
                tool.executor.shutdown().await,
                if observe {
                    sdk::Cleanup::Observed
                } else {
                    sdk::Cleanup::Pending
                }
            );
        }
        std::fs::write(root.join("completed"), b"accepted-and-fenced").unwrap();
    });
}

#[test]
fn isolated_extension_tool_qualifies_policy_output_and_no_replay_glue() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "extensions::runtime::acceptance_tests::output_acceptance_child",
        ])
        .env(ROOT, &root)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "isolated extension acceptance fixture failed"
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("isolated extension acceptance fixture exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(root.join("completed")).unwrap(),
        b"accepted-and-fenced"
    );
}
