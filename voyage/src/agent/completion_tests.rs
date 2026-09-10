//! Offline scripted-provider checks. No live requests or synthetic user turns.
use super::*;
use crate::{
    completion::runtime::{Coordinator, RunHandle},
    model::{ModelResponse, Role, ToolCall},
    subagent::{AgentTreeStore, RuntimeLimits, SubagentExecutor, SubagentResult, SubagentRuntime},
    todo::{NewTodo, Priority, TodoScope, TodoStatus, TodoStore},
    tools::{InteractionMode, TodoTool, UnattendedApprover},
};
use std::sync::Mutex;

const NOTICE: &str = "Voyage runtime completion notice (not a user message)";
const USER: &str = "Finish the requested work";

enum Step {
    Answer(&'static str),
    Finish(crate::todo::TodoId),
    Fail,
}
struct Scripted {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    steps: Mutex<VecDeque<Step>>,
}
#[async_trait]
impl Provider for Scripted {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let mut message = Message::new(Role::Assistant, "");
        match self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected inference")
        {
            Step::Answer(text) => message.content = text.into(),
            Step::Finish(id) => message.tool_calls.push(ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: "todo".into(),
                arguments: serde_json::json!({"action":"status","id":id,"status":"completed"}),
            }),
            Step::Fail => return Err(ProviderError::Request("synthetic failure".into())),
        }
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
struct UnusedExecutor;
#[async_trait]
impl SubagentExecutor for UnusedExecutor {
    async fn execute(
        &self,
        _: crate::subagent::ExecutionContext,
    ) -> Result<SubagentResult, String> {
        panic!("no child was requested")
    }
}
#[derive(Default)]
struct Sink {
    notices: Mutex<usize>,
    cancel_on_notice: Option<CancellationToken>,
    steer_on_notice: Option<SteeringSender>,
    release_child: Option<(
        Arc<tokio::sync::Notify>,
        Arc<SubagentRuntime>,
        crate::subagent::AgentId,
    )>,
}
#[async_trait]
impl EventSink for Sink {
    async fn emit(&self, event: AgentEvent) {
        if matches!(
            event,
            AgentEvent::CompletionState {
                phase: CompletionPhase::Reconciling,
                ..
            }
        ) {
            *self.notices.lock().unwrap() += 1;
            if let Some((release, runtime, id)) = &self.release_child {
                release.notify_one();
                runtime
                    .wait(*id)
                    .await
                    .unwrap()
                    .expect("useful child was cancelled before continuation");
            }
            if let Some(cancel) = &self.cancel_on_notice {
                cancel.cancel();
            }
            if let Some(sender) = &self.steer_on_notice {
                sender.try_send("Pause this work now".into()).unwrap();
            }
        }
    }
}
struct Checkpoint {
    run: uuid::Uuid,
    fail: bool,
    saved: Mutex<Vec<Message>>,
    accepted: Mutex<usize>,
}
#[async_trait]
impl RunCheckpoint for Checkpoint {
    fn run_id(&self) -> uuid::Uuid {
        self.run
    }
    async fn canonical(&self, messages: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        if self.fail {
            return Err(CheckpointError);
        }
        *self.saved.lock().unwrap() = messages.to_vec();
        Ok(())
    }
    async fn partial(&self, _: &str) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn accepted(
        &self,
        _: &[Message],
        _: &Usage,
        _: &StopReason,
    ) -> Result<(), CheckpointError> {
        *self.accepted.lock().unwrap() += 1;
        Ok(())
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    context: ToolContext,
    coordinator: Coordinator,
    scope: RunHandle,
    todos: Arc<TodoStore>,
    runtime: Arc<SubagentRuntime>,
    checkpoint: Checkpoint,
}
impl Fixture {
    async fn new() -> Self {
        Self::with_executor(Arc::new(UnusedExecutor)).await
    }
    async fn with_executor(executor: Arc<dyn SubagentExecutor>) -> Self {
        let root = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(root.path().join("coord"), root.path()).unwrap();
        let session = uuid::Uuid::new_v4();
        let scope = RunHandle::create(coordinator.clone(), session, uuid::Uuid::new_v4())
            .await
            .unwrap();
        let todos = Arc::new(
            TodoStore::new(
                root.path().join("todos.json"),
                TodoScope::session(root.path().into(), session),
            )
            .with_coordinator(coordinator.clone()),
        );
        let store = AgentTreeStore::new(root.path().join("agents.json"))
            .with_coordinator(coordinator.clone());
        let runtime = Arc::new(
            SubagentRuntime::new(executor, RuntimeLimits::default(), Some(store)).unwrap(),
        );
        let config = crate::Config {
            access: Some(AccessMode::Unrestricted),
            ..Default::default()
        };
        let context = ToolContext {
            artifact_scope: None,
            github: None,
            completion: None,
            policy: Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap()),
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: Duration::from_secs(3),
            max_output_bytes: 4096,
            environment: Default::default(),
            cancellation: CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        };
        let checkpoint = Checkpoint {
            run: scope.run_id(),
            fail: false,
            saved: Mutex::new(vec![]),
            accepted: Mutex::new(0),
        };
        Self {
            _root: root,
            context,
            coordinator,
            scope,
            todos,
            runtime,
            checkpoint,
        }
    }
    async fn todo(&self) -> crate::todo::TodoId {
        self.todos
            .create_registered(
                NewTodo {
                    title: "Finish the fixture".into(),
                    description: String::new(),
                    priority: Priority::Normal,
                    order: None,
                    assignees: Default::default(),
                },
                Some(&self.scope),
            )
            .await
            .unwrap()
            .id
    }
    fn agent(&self, steps: Vec<Step>, sink: Arc<Sink>) -> (Agent, Arc<Mutex<Vec<ModelRequest>>>) {
        let requests = Arc::new(Mutex::new(vec![]));
        let provider = Scripted {
            requests: requests.clone(),
            steps: Mutex::new(steps.into()),
        };
        let mut tools = ToolRegistry::default();
        tools.register(TodoTool::new(self.todos.clone()));
        let agent = Agent::new(
            Box::new(provider),
            tools,
            self.context.clone(),
            sink,
            "fixture".into(),
            "Runtime instructions".into(),
            1024,
            None,
        )
        .with_completion_coordinator(self.coordinator.clone())
        .with_completion_gate(
            self.todos.clone(),
            self.runtime.store().unwrap(),
            self.runtime.clone(),
        );
        (agent, requests)
    }
    async fn run(
        &self,
        agent: &Agent,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
    ) -> Result<AgentOutcome, AgentError> {
        tokio::time::timeout(
            Duration::from_secs(5),
            agent.run_checkpointed_scoped(
                vec![],
                USER.into(),
                cancel,
                input,
                &self.checkpoint,
                "fixture".into(),
                Some(self.scope.clone()),
            ),
        )
        .await
        .expect("continuation hung while holding a writer lease")
    }
}
fn assert_authored_history(messages: &[Message]) {
    assert!(
        messages
            .iter()
            .all(|m| m.role != Role::System && !m.content.contains(NOTICE))
    );
    let users: Vec<_> = messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(users, [USER]);
}

#[tokio::test]
async fn clean_completion_does_not_request_continuation() {
    let f = Fixture::new().await;
    let sink = Arc::new(Sink::default());
    let (agent, requests) = f.agent(vec![Step::Answer("Done")], sink.clone());
    let outcome = f.run(&agent, CancellationToken::new(), None).await.unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(*sink.notices.lock().unwrap(), 0);
    assert_authored_history(&outcome.messages);
}

#[tokio::test]
async fn unfinished_work_continues_via_system_instructions_and_tools() {
    let f = Fixture::new().await;
    let id = f.todo().await;
    let sink = Arc::new(Sink::default());
    let (agent, requests) = f.agent(
        vec![
            Step::Answer("Premature final"),
            Step::Finish(id),
            Step::Answer("Actually done"),
        ],
        sink.clone(),
    );
    let outcome = f.run(&agent, CancellationToken::new(), None).await.unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(outcome.answer, "Actually done");
    assert_eq!(*sink.notices.lock().unwrap(), 1);
    assert_eq!(*f.checkpoint.accepted.lock().unwrap(), 1);
    assert_eq!(
        f.todos.snapshot().await.unwrap().items[&id].status,
        TodoStatus::Completed
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(!requests[0].messages[0].content.contains(NOTICE));
    for request in &requests[1..] {
        assert_eq!(request.messages[0].role, Role::System);
        assert!(request.messages[0].content.contains(NOTICE));
        assert!(request.messages[0].content.contains(&id.0.to_string()));
        assert_eq!(
            request
                .messages
                .iter()
                .filter(|m| m.role == Role::User)
                .count(),
            1
        );
    }
    assert!(
        outcome
            .messages
            .iter()
            .any(|m| m.content == "Premature final")
    );
    assert!(
        requests[1..]
            .iter()
            .all(|r| r.messages.iter().all(|m| m.content != "Premature final"))
    );
    assert_ne!(requests[1].messages.last().unwrap().role, Role::Assistant);
    assert_authored_history(&outcome.messages);
    assert_authored_history(&f.checkpoint.saved.lock().unwrap());
}

#[tokio::test]
async fn no_progress_gets_one_reminder_and_stays_incomplete() {
    let f = Fixture::new().await;
    let id = f.todo().await;
    let sink = Arc::new(Sink::default());
    let (agent, requests) = f.agent(
        vec![Step::Answer("Done"), Step::Answer("Still blocked")],
        sink.clone(),
    );
    let outcome = f.run(&agent, CancellationToken::new(), None).await.unwrap();
    assert!(matches!(outcome.stop_reason, StopReason::Incomplete { .. }));
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(*sink.notices.lock().unwrap(), 1);
    assert_authored_history(&outcome.messages);
    assert_eq!(
        f.todos.snapshot().await.unwrap().items[&id].status,
        TodoStatus::Pending
    );
}

#[tokio::test]
async fn cancellation_at_notice_prevents_another_provider_request() {
    let f = Fixture::new().await;
    f.todo().await;
    let cancel = CancellationToken::new();
    let sink = Arc::new(Sink {
        cancel_on_notice: Some(cancel.clone()),
        ..Default::default()
    });
    let (agent, requests) = f.agent(vec![Step::Answer("Done")], sink);
    assert!(f.run(&agent, cancel, None).await.is_err());
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(*f.checkpoint.accepted.lock().unwrap(), 0);
}

#[tokio::test]
async fn real_steering_is_preserved_and_notice_is_not_user_input() {
    let f = Fixture::new().await;
    f.todo().await;
    let (sender, receiver) = steering_channel(2);
    let sink = Arc::new(Sink {
        steer_on_notice: Some(sender),
        ..Default::default()
    });
    let (agent, requests) = f.agent(
        vec![Step::Answer("Done"), Step::Answer("Paused as requested")],
        sink,
    );
    let outcome = f
        .run(&agent, CancellationToken::new(), Some(receiver))
        .await
        .unwrap();
    assert!(matches!(outcome.stop_reason, StopReason::Incomplete { .. }));
    let requests = requests.lock().unwrap();
    let users: Vec<_> = requests[1]
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .collect();
    assert_eq!(users, [USER, "Pause this work now"]);
    assert!(
        requests[1].messages[0]
            .content
            .contains("Respect the user's latest instructions")
    );
    assert!(outcome.messages.iter().all(|m| !m.content.contains(NOTICE)));
}

#[tokio::test]
async fn provider_failure_does_not_trigger_completion_retry() {
    let f = Fixture::new().await;
    f.todo().await;
    let sink = Arc::new(Sink::default());
    let (agent, requests) = f.agent(vec![Step::Fail], sink.clone());
    assert!(f.run(&agent, CancellationToken::new(), None).await.is_err());
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(*sink.notices.lock().unwrap(), 0);
}

#[tokio::test]
async fn failed_checkpoint_prevents_inference() {
    let mut f = Fixture::new().await;
    f.todo().await;
    f.checkpoint.fail = true;
    let (agent, requests) = f.agent(vec![], Arc::new(Sink::default()));
    assert!(f.run(&agent, CancellationToken::new(), None).await.is_err());
    assert!(requests.lock().unwrap().is_empty());
}

struct HeldChild(Arc<tokio::sync::Notify>);
#[async_trait]
impl SubagentExecutor for HeldChild {
    async fn execute(
        &self,
        context: crate::subagent::ExecutionContext,
    ) -> Result<SubagentResult, String> {
        tokio::select! {
            _ = context.cancellation.cancelled() => Err("cancelled too early".into()),
            _ = self.0.notified() => Ok(SubagentResult { summary: "useful work finished".into() }),
        }
    }
}
#[tokio::test]
async fn useful_child_is_not_cancelled_before_runtime_continuation() {
    use crate::subagent::{AgentBudget, AgentPolicy, ApprovalPolicy, SpawnRequest};
    let release = Arc::new(tokio::sync::Notify::new());
    let f = Fixture::with_executor(Arc::new(HeldChild(release.clone()))).await;
    let budget = AgentBudget {
        max_tokens: 0,
        max_terminals: 1,
    };
    let id = f
        .runtime
        .spawn_for_run(
            SpawnRequest {
                parent_id: None,
                name: "useful-child".into(),
                task: "finish useful work".into(),
                policy: AgentPolicy {
                    access: AccessMode::Unrestricted,
                    readable_roots: vec![f._root.path().into()],
                    writable_roots: vec![f._root.path().into()],
                    allowed_tools: Default::default(),
                    approval: ApprovalPolicy::Deny,
                    budget: budget.clone(),
                },
                budget,
                worktree: None,
                branch: None,
            },
            Some(f.scope.clone()),
        )
        .await
        .unwrap();
    let sink = Arc::new(Sink {
        release_child: Some((release, f.runtime.clone(), id)),
        ..Default::default()
    });
    let (agent, requests) = f.agent(
        vec![
            Step::Answer("Premature final"),
            Step::Answer("Child finished"),
        ],
        sink.clone(),
    );
    let outcome = f.run(&agent, CancellationToken::new(), None).await.unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(*sink.notices.lock().unwrap(), 1);
}

fn request_copy(request: &ModelRequest) -> ModelRequest {
    ModelRequest {
        model: request.model.clone(),
        messages: request.messages.clone(),
        tools: request.tools.clone(),
        temperature: request.temperature,
        reasoning_effort: request.reasoning_effort.clone(),
        service_tier: request.service_tier.clone(),
        max_tokens: request.max_tokens,
    }
}
fn capture_http(
    response: serde_json::Value,
) -> (String, std::thread::JoinHandle<serde_json::Value>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "provider never connected"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let (header_end, length) = loop {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
            assert!(bytes.len() < 1024 * 1024);
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                assert!(length < 1024 * 1024);
                break (end + 4, length);
            }
        };
        while bytes.len() < header_end + length {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&chunk[..n]);
        }
        let body = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
        let response = response.to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        body
    });
    (endpoint, worker)
}
#[tokio::test]
async fn native_wire_encodes_notice_as_instructions_not_user_content() {
    use crate::provider::{AnthropicProvider, OpenAiProvider, OpenAiResponsesProvider};
    use serde_json::json;
    let f = Fixture::new().await;
    f.todo().await;
    let (agent, requests) = f.agent(
        vec![Step::Answer("Premature final"), Step::Answer("Blocked")],
        Arc::new(Sink::default()),
    );
    f.run(&agent, CancellationToken::new(), None).await.unwrap();
    let request = request_copy(&requests.lock().unwrap()[1]);
    for kind in ["responses", "chat", "anthropic"] {
        let response = match kind {
            "responses" => {
                json!({"id":"fixture","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}],"usage":{"input_tokens":1,"output_tokens":1}})
            }
            "chat" => {
                json!({"choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}})
            }
            _ => {
                json!({"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}})
            }
        };
        let (url, capture) = capture_http(response);
        let provider: Box<dyn Provider> = match kind {
            "responses" => Box::new(OpenAiResponsesProvider::new("synthetic".into(), Some(url))),
            "chat" => Box::new(OpenAiProvider::new("synthetic".into(), Some(url))),
            _ => Box::new(AnthropicProvider::new("synthetic".into(), Some(url))),
        };
        provider.complete(request_copy(&request)).await.unwrap();
        let body = capture.join().unwrap();
        let instructions = match kind {
            "responses" => body["instructions"].to_string(),
            "anthropic" => body["system"].to_string(),
            _ => body["messages"][0]["content"].to_string(),
        };
        assert!(instructions.contains(NOTICE), "{kind}");
        let messages = if kind == "responses" {
            &body["input"]
        } else {
            &body["messages"]
        };
        let users: Vec<_> = messages
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m["role"] == "user")
            .collect();
        assert_eq!(users.len(), 1, "{kind}");
        assert!(!users[0].to_string().contains(NOTICE), "{kind}");
        assert!(!messages.to_string().contains("Premature final"), "{kind}");
        assert_eq!(
            messages.as_array().unwrap().last().unwrap()["role"],
            "user",
            "{kind}"
        );
    }
}

#[tokio::test]
async fn failure_during_continuation_does_not_start_another_reminder() {
    let f = Fixture::new().await;
    f.todo().await;
    let sink = Arc::new(Sink::default());
    let (agent, requests) = f.agent(
        vec![Step::Answer("Premature final"), Step::Fail],
        sink.clone(),
    );
    assert!(f.run(&agent, CancellationToken::new(), None).await.is_err());
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(*sink.notices.lock().unwrap(), 1);
    assert_eq!(*f.checkpoint.accepted.lock().unwrap(), 0);
}

#[tokio::test]
async fn later_run_does_not_inherit_runtime_notice() {
    let f = Fixture::new().await;
    let id = f.todo().await;
    let (agent, requests) = f.agent(
        vec![
            Step::Answer("Premature final"),
            Step::Finish(id),
            Step::Answer("Done"),
            Step::Answer("Next request done"),
        ],
        Arc::new(Sink::default()),
    );
    let prior = f.run(&agent, CancellationToken::new(), None).await.unwrap();
    let next = RunHandle::create(
        f.coordinator.clone(),
        f.scope.reference().session_id,
        uuid::Uuid::new_v4(),
    )
    .await
    .unwrap();
    let checkpoint = Checkpoint {
        run: next.run_id(),
        fail: false,
        saved: Mutex::new(vec![]),
        accepted: Mutex::new(0),
    };
    let outcome = agent
        .run_checkpointed_scoped(
            prior.messages,
            "A new request".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
            Some(next),
        )
        .await
        .unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(!requests[3].messages[0].content.contains(NOTICE));
    assert_eq!(
        requests[3]
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .count(),
        2
    );
}
