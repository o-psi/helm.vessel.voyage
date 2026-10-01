//! Ordinary child assembly and delegation. Native loopback responses are owned
//! fixtures; no credential environment or external inference service is used.
use super::*;
use crate::subagent::{AgentId, SpawnRequest};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

#[test]
fn nearest_ancestor_worktree_is_selected_only_after_the_complete_finite_ancestry_is_validated() {
    let root = tempfile::tempdir().unwrap();
    let a = AgentId::new();
    let b = AgentId::new();
    let c = AgentId::new();
    let ancestor = root.path().join("ancestor");
    let nearer = root.path().join("nearer");
    let records = vec![
        (a, None, Some(ancestor.clone())),
        (b, Some(a), Some(nearer.clone())),
        (c, Some(b), None),
    ];
    assert_eq!(
        inherited_child_workspace(records.clone(), c, root.path()).unwrap(),
        nearer
    );
    assert_eq!(
        inherited_child_workspace(records.clone(), a, root.path()).unwrap(),
        ancestor
    );
    assert_eq!(
        inherited_child_workspace(vec![(a, None, None), (b, Some(a), None)], b, root.path())
            .unwrap(),
        root.path()
    );
    // A nearer path does not excuse a missing ancestor or an ancestry cycle.
    assert!(
        inherited_child_workspace(vec![(b, Some(a), Some(nearer.clone()))], b, root.path())
            .is_err()
    );
    assert!(
        inherited_child_workspace(
            vec![
                (a, Some(b), Some(ancestor)),
                (b, Some(a), Some(nearer.clone()))
            ],
            b,
            root.path()
        )
        .is_err()
    );
    assert!(
        inherited_child_workspace(
            vec![(a, None, None), (a, None, Some(nearer))],
            a,
            root.path()
        )
        .is_err()
    );
    assert!(inherited_child_workspace(records, AgentId::new(), root.path()).is_err());
}

#[test]
fn child_workspace_requires_both_explicit_canonical_read_and_write_roots_before_implicit_workspace_roots()
 {
    let root = tempfile::tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let child = allowed.join("child");
    let outside = root.path().join("allowed-looking-sibling");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::create_dir(&outside).unwrap();
    assert!(
        check_requested_workspace(
            &child,
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed)
        )
        .is_ok()
    );
    for (read, write) in [
        (vec![], vec![allowed.clone()]),
        (vec![allowed.clone()], vec![]),
        (vec![outside.clone()], vec![allowed.clone()]),
        (vec![allowed.clone()], vec![outside.clone()]),
    ] {
        assert!(check_requested_workspace(&child, &read, &write).is_err());
    }
    assert!(
        check_requested_workspace(
            &outside,
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed)
        )
        .is_err()
    );
    assert!(
        check_requested_workspace(
            &root.path().join("missing"),
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed)
        )
        .is_err()
    );
    assert!(
        check_requested_workspace(
            &child,
            &[allowed.clone(), root.path().join("missing-root")],
            &[allowed]
        )
        .is_err()
    );
}
#[cfg(unix)]
#[test]
fn canonical_workspace_delegation_refuses_symlink_escape_without_widening_parent_roots() {
    let root = tempfile::tempdir().unwrap();
    let allowed = root.path().join("allowed");
    let outside = root.path().join("outside");
    std::fs::create_dir(&allowed).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let link = allowed.join("link");
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    assert!(
        check_requested_workspace(
            &link,
            std::slice::from_ref(&allowed),
            std::slice::from_ref(&allowed)
        )
        .is_err()
    );
    assert!(
        check_requested_workspace(
            &link,
            std::slice::from_ref(&outside),
            std::slice::from_ref(&outside)
        )
        .is_ok()
    );
}

fn child_request(root: &std::path::Path, task: &str) -> SpawnRequest {
    let budget = AgentBudget {
        max_tokens: 10,
        max_terminals: 0,
    };
    SpawnRequest {
        parent_id: None,
        name: "Owned child".into(),
        task: task.into(),
        policy: AgentPolicy {
            access: AccessMode::ReadOnly,
            readable_roots: vec![root.into()],
            writable_roots: vec![root.into()],
            allowed_tools: BTreeSet::from(["read_file".into()]),
            approval: ApprovalPolicy::Deny,
            budget: AgentBudget {
                max_tokens: 15,
                max_terminals: 0,
            },
        },
        budget,
        worktree: None,
        branch: None,
    }
}
fn executor(
    root: &std::path::Path,
    config: Config,
    parent_policy: Arc<Policy>,
) -> Arc<CliSubagentExecutor> {
    Arc::new(CliSubagentExecutor {
        approver: None,
        managed_resources: None,
        todos: TodoTool::new(Arc::new(TodoStore::new(
            root.join("todos.json"),
            TodoScope::workspace(root.into()),
        ))),
        config,
        parent_policy,
        workspace: root.into(),
        runtime: OnceLock::new(),
        worktrees: None,
        worktree_error: None,
        model: Arc::new(RwLock::new("reviewed-child-model".into())),
    })
}
fn connect(executor: &Arc<CliSubagentExecutor>) -> Arc<SubagentRuntime> {
    let runtime = Arc::new(
        SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 64,
            },
            None,
        )
        .unwrap(),
    );
    assert!(executor.runtime.set(Arc::downgrade(&runtime)).is_ok());
    runtime
}
fn local_config(root: &std::path::Path) -> Config {
    Config {
        provider: crate::config::ProviderKind::OpenaiChat,
        api_key_required: false,
        api_key_env: format!("VOYAGE_OWNED_UNUSED_{}", uuid::Uuid::new_v4().simple()),
        base_url: Some("http://127.0.0.1:9/v1".into()),
        model: "initial-root-model".into(),
        workspace: Some(root.into()),
        access: Some(AccessMode::ReadOnly),
        max_tokens: 20,
        chat_use_max_tokens: true,
        provider_retry_attempts: 1,
        provider_response_timeout_ms: 5000,
        provider_stream_idle_ms: 5000,
        ..Default::default()
    }
}
async fn wait(runtime: &SubagentRuntime, id: AgentId) -> Result<SubagentResult, String> {
    tokio::time::timeout(Duration::from_secs(10), runtime.wait(id))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn executor_refuses_unavailable_runtime_and_invalid_requested_workspace_before_provider_initialization()
 {
    for failure in ["runtime", "workspace"] {
        let root = tempfile::tempdir().unwrap();
        let mut peer = Peer::new().await;
        let mut config = local_config(root.path());
        config.base_url = Some(peer.url.clone());
        let parent = Arc::new(Policy::new(&config, root.path().into()).unwrap());
        let executor = executor(root.path(), config, parent);
        let runtime = if failure == "runtime" {
            Arc::new(SubagentRuntime::new(executor, RuntimeLimits::default(), None).unwrap())
        } else {
            connect(&executor)
        };
        let mut request = child_request(root.path(), "This must not reach any provider");
        if failure == "workspace" {
            request.policy.writable_roots.clear();
        }
        let id = runtime.spawn(request).await.unwrap();
        let error = wait(&runtime, id).await.unwrap_err();
        assert!(error.contains(if failure == "runtime" {
            "runtime unavailable"
        } else {
            "requested read/write delegation"
        }));
        assert!(runtime.get(id).await.unwrap().result.is_none());
        assert!(peer.requests.lock().await.is_empty());
        runtime.shutdown().await;
        peer.shutdown().await;
    }
}

#[derive(Debug)]
struct Revoked;
impl crate::policy::ExecutionAuthority for Revoked {
    fn check(&self) -> Result<()> {
        anyhow::bail!("owned child parent authority revoked")
    }
}
#[tokio::test]
async fn child_assembly_inherits_parent_authority_and_never_turns_revocation_into_unrestricted_execution()
 {
    let root = tempfile::tempdir().unwrap();
    let mut peer = Peer::new().await;
    let mut config = local_config(root.path());
    config.base_url = Some(peer.url.clone());
    let parent = Arc::new(
        Policy::new(&config, root.path().into())
            .unwrap()
            .with_execution_authority(Arc::new(Revoked)),
    );
    let executor = executor(root.path(), config, parent);
    let runtime = connect(&executor);
    let id = runtime
        .spawn(child_request(
            root.path(),
            "Bounded refusal before provider",
        ))
        .await
        .unwrap();
    let error = wait(&runtime, id).await.unwrap_err();
    assert!(error.contains("authority revoked"));
    assert!(runtime.get(id).await.unwrap().result.is_none());
    assert!(peer.requests.lock().await.is_empty());
    runtime.shutdown().await;
    peer.shutdown().await;
}

struct Peer {
    url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    task: Option<tokio::task::JoinHandle<()>>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
impl Peer {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let received = requests.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let end = loop {
                    let mut block = [0; 4096];
                    let n = socket.read(&mut block).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&block[..n]);
                    assert!(bytes.len() < 1024 * 1024);
                    if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break end + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let models = headers.starts_with("GET /v1/models ");
                if !models {
                    assert!(headers.starts_with("POST /v1/chat/completions "));
                }
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                assert!(length < 1024 * 1024);
                while bytes.len() < end + length {
                    let mut block = [0; 4096];
                    let n = socket.read(&mut block).await.unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&block[..n]);
                }
                let body = if models {
                    json!({"data":[{"id":"reviewed-child-model"}]}).to_string()
                } else {
                    received
                        .lock()
                        .await
                        .push(serde_json::from_slice(&bytes[end..end + length]).unwrap());
                    format!(
                        "data: {}\n\ndata: [DONE]\n\n",
                        json!({"choices":[{"index":0,"delta":{"role":"assistant","content":"Owned loopback child result"},"finish_reason":"stop"}]})
                    )
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    if models {
                        "application/json"
                    } else {
                        "text/event-stream"
                    },
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
                if !models {
                    return;
                }
            }
        });
        Self {
            url,
            requests,
            task: Some(task),
        }
    }
    async fn shutdown(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let result = tokio::time::timeout(Duration::from_secs(3), task)
                .await
                .unwrap();
            assert!(result.is_ok() || result.unwrap_err().is_cancelled());
        }
    }
    async fn retired(&mut self) {
        tokio::time::timeout(Duration::from_secs(5), self.task.take().unwrap())
            .await
            .unwrap()
            .unwrap();
    }
}

#[tokio::test]
async fn actual_child_agent_uses_captured_model_budget_tools_and_selected_input_on_owned_loopback_only()
 {
    let root = tempfile::tempdir().unwrap();
    let mut peer = Peer::new().await;
    let mut config = local_config(root.path());
    config.base_url = Some(peer.url.clone());
    config.system_prompt = "Owned child runtime instructions".into();
    let parent = Arc::new(Policy::new(&config, root.path().into()).unwrap());
    let executor = executor(root.path(), config, parent);
    let runtime = connect(&executor);
    let id = runtime
        .spawn(child_request(
            root.path(),
            "Only this explicitly selected child task",
        ))
        .await
        .unwrap();
    assert_eq!(
        wait(&runtime, id).await.unwrap().summary,
        "Owned loopback child result"
    );
    peer.retired().await;
    let requests = peer.requests.lock().await;
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request["model"], "reviewed-child-model");
    assert_eq!(request["max_tokens"], 10);
    assert_eq!(request["tools"].as_array().unwrap().len(), 1);
    assert_eq!(request["tools"][0]["function"]["name"], "read_file");
    assert!(
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "user"
                && message["content"] == "Only this explicitly selected child task")
    );
    assert!(!request.to_string().contains("initial-root-model"));
    drop(requests);
    runtime.shutdown().await;
}

#[tokio::test]
async fn managed_child_build_isolated_resource_scope_produces_current_persistent_owner_and_drains_it()
 {
    const CHILD: &str = "VOYAGE_SUBAGENT_BUILD_FAMILY_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let root = tempfile::tempdir().unwrap();
        let resources = root.path().join("resources");
        crate::build::set_resource_root(resources.clone()).unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let mut peer = Peer::new().await;
        let mut config = local_config(&workspace);
        config.base_url = Some(peer.url.clone());
        config.subagent_max_concurrency = 1;
        let policy = Arc::new(Policy::new(&config, workspace.clone()).unwrap());
        let bundle = build_subagents_managed(&config, &workspace, policy, None, None)
            .await
            .unwrap();
        assert!(
            bundle
                .runtime
                .store()
                .unwrap()
                .coordinator()
                .unwrap()
                .same(&bundle.coordinator)
        );
        assert_eq!(*bundle.model.read().unwrap(), "initial-root-model");
        *bundle.model.write().unwrap() = "reviewed-child-model".into();
        let id = bundle
            .runtime
            .spawn(child_request(&workspace, "Owned managed child task"))
            .await
            .unwrap();
        assert_eq!(
            wait(&bundle.runtime, id).await.unwrap().summary,
            "Owned loopback child result"
        );
        peer.retired().await;
        assert_eq!(peer.requests.lock().await.len(), 1);
        assert!(bundle.runtime.is_archived(id).await.unwrap());
        assert!(resources.join("subagents").exists());
        assert!(
            bundle
                .todos
                .store()
                .snapshot()
                .await
                .unwrap()
                .items
                .is_empty()
        );
        bundle.runtime.shutdown().await;
        return;
    }
    let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
    command.args(["--exact","build::subagents::subagents_family_tests::managed_child_build_isolated_resource_scope_produces_current_persistent_owner_and_drains_it","--nocapture"]).env(CHILD,"1").kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(30), command.output())
        .await
        .unwrap()
        .unwrap();
    assert!(
        output.status.success(),
        "managed child fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}
