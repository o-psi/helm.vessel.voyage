use super::*;
use crate::{
    completion::{
        DispositionKind, FinalOutcome, Obligation,
        runtime::{Coordinator, Review, RunHandle},
    },
    model::{ModelResponse, Role, ToolCall},
    subagent::{
        AgentBudget, AgentPolicy, AgentTreeStore, ApprovalPolicy, ExecutionContext, RuntimeLimits,
        SpawnRequest, SubagentExecutor, SubagentResult, SubagentRuntime,
    },
    todo::{EntryKind, NewTodo, Priority, TodoId, TodoScope, TodoStatus, TodoStore},
};
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Executor;
#[async_trait]
impl SubagentExecutor for Executor {
    async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
        if context.task == "message" {
            context.recv().await;
        }
        if context.task == "fail" {
            return Err("fixture child failure".into());
        }
        if context.task == "hold" {
            context.cancellation.cancelled().await;
        }
        Ok(SubagentResult {
            summary: "fixture child result".into(),
        })
    }
}

struct Fixture {
    directory: tempfile::TempDir,
    coordinator: Coordinator,
    todos: Arc<TodoStore>,
    agents: AgentTreeStore,
    runtime: Arc<SubagentRuntime>,
}
impl Fixture {
    fn new() -> Self {
        Self::with_limits(RuntimeLimits::default())
    }
    fn with_limits(limits: RuntimeLimits) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let coordinator =
            Coordinator::open(directory.path().join("completion"), directory.path()).unwrap();
        let todos = Arc::new(
            TodoStore::new(
                directory.path().join("todos.json"),
                TodoScope::workspace(directory.path().canonicalize().unwrap()),
            )
            .with_coordinator(coordinator.clone()),
        );
        let agents = AgentTreeStore::new(directory.path().join("agents/tree.json"))
            .with_coordinator(coordinator.clone());
        let runtime = Arc::new(
            SubagentRuntime::new(Arc::new(Executor), limits, Some(agents.clone())).unwrap(),
        );
        Self {
            directory,
            coordinator,
            todos,
            agents,
            runtime,
        }
    }
    async fn scope(&self) -> RunHandle {
        RunHandle::create(
            self.coordinator.clone(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        )
        .await
        .unwrap()
    }
    fn agent(&self, provider: Arc<Script>) -> Agent {
        let mut agent = super::tests::agent(Box::new(provider), &self.directory)
            .with_completion_coordinator(self.coordinator.clone())
            .with_completion_gate(
                self.todos.clone(),
                self.agents.clone(),
                self.runtime.clone(),
            )
            .with_completion_deadlines(Duration::from_secs(1), Duration::from_millis(200))
            .with_context_window(1_000_000)
            .with_retry_policy(RetryPolicy {
                max_attempts: 1,
                ..RetryPolicy::default()
            });
        agent
            .tools
            .register(crate::completion::tool::CompletionTool::new(
                self.todos.clone(),
                self.agents.clone(),
            ));
        agent
    }
    async fn todo(&self, scope: &RunHandle, complete: bool) -> TodoId {
        let item = self
            .todos
            .create_registered(
                NewTodo {
                    title: "owned work".into(),
                    description: String::new(),
                    priority: Priority::Normal,
                    order: None,
                    assignees: BTreeSet::new(),
                },
                Some(scope),
            )
            .await
            .unwrap();
        if complete {
            self.todos
                .append_note(
                    item.id,
                    EntryKind::Evidence,
                    "verified fixture evidence".into(),
                    None,
                )
                .await
                .unwrap();
            self.todos
                .set_status(item.id, TodoStatus::Completed)
                .await
                .unwrap();
        }
        item.id
    }
    async fn account(
        &self,
        scope: &RunHandle,
        obligation: Obligation,
        disposition: DispositionKind,
    ) {
        let readiness = scope.snapshot(&self.todos, &self.agents, 64).await.unwrap();
        scope
            .account(
                &self.todos,
                &self.agents,
                obligation,
                Review {
                    revision: readiness.revision,
                    fingerprint: readiness.fingerprint,
                    disposition,
                    reason: "verified fixture disposition".into(),
                },
            )
            .await
            .unwrap();
    }
}

#[derive(Clone)]
enum Step {
    Answer(&'static str),
    Account(TodoId, &'static str),
    Fail,
    Hang,
    ToolLoop,
}
struct Script {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<ModelRequest>>,
    entered: tokio::sync::Notify,
    supports: bool,
}
impl Script {
    fn new(steps: impl IntoIterator<Item = Step>) -> Arc<Self> {
        Arc::new(Self {
            steps: Mutex::new(steps.into_iter().collect()),
            requests: Mutex::new(vec![]),
            entered: tokio::sync::Notify::new(),
            supports: true,
        })
    }
    fn calls(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
#[async_trait]
impl Provider for Arc<Script> {
    fn supports_steering(&self) -> bool {
        self.supports
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(Step::Answer("repeated final"));
        self.requests.lock().unwrap().push(request.clone());
        self.entered.notify_one();
        let mut message = Message::new(Role::Assistant, "");
        match step {
            Step::Answer(text) => message.content = text.into(),
            Step::Fail => return Err(ProviderError::Unavailable("fixture outage".into())),
            Step::Hang => return std::future::pending().await,
            Step::ToolLoop => {
                self.steps.lock().unwrap().push_front(Step::ToolLoop);
                message.tool_calls.push(ToolCall {
                    id: format!("loop-{}", self.calls()),
                    name: "missing".into(),
                    arguments: serde_json::json!({}),
                });
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Step::Account(id, disposition) => {
                let update = request
                    .messages
                    .iter()
                    .find(|m| {
                        m.role == Role::System
                            && m.content.contains("Helm has withheld final acceptance")
                    })
                    .expect("targeted update");
                let readiness: serde_json::Value =
                    serde_json::from_str(update.content.lines().last().unwrap()).unwrap();
                message.tool_calls.push(ToolCall { id: "account-call".into(), name: "completion".into(), arguments: serde_json::json!({"action":"account","kind":"todo","id":id.0,"revision":readiness["revision"],"fingerprint":readiness["fingerprint"],"disposition":disposition,"reason":"Reviewed fixture evidence and incorporated it"}) });
            }
        }
        Ok(ModelResponse {
            message,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
        })
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<AgentEvent>>);
#[async_trait]
impl EventSink for Events {
    async fn emit(&self, event: AgentEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn root_gate_clean_run_seals_without_extra_request_or_adopting_unrelated_work() {
    let fixture = Fixture::new();
    let unrelated = fixture.scope().await;
    fixture.todo(&unrelated, false).await;
    let scope = fixture.scope().await;
    let provider = Script::new([Step::Answer("accepted")]);
    let mut agent = fixture.agent(provider.clone());
    let events = Arc::new(Events::default());
    agent.sink = events.clone();
    let outcome = agent
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            Some(scope.clone()),
        )
        .await
        .unwrap();
    assert_eq!(provider.calls(), 1);
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(
        scope.decision().await.unwrap().unwrap().outcome,
        FinalOutcome::Completed
    );
    assert!(unrelated.decision().await.unwrap().is_none());
    let phases: Vec<_> = events
        .0
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| {
            if let AgentEvent::CompletionState { phase, .. } = e {
                Some(phase.clone())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        phases,
        [CompletionPhase::Provisional, CompletionPhase::Completed]
    );
}

#[tokio::test]
async fn root_gate_reconciliation_tools_produce_one_accepted_final_with_canonical_proposal() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let id = fixture.todo(&scope, true).await;
    let provider = Script::new([
        Step::Answer("premature final"),
        Step::Account(id, "completed_with_evidence"),
        Step::Answer("verified final"),
    ]);
    let agent = fixture.agent(provider.clone());
    let outcome = agent
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            Some(scope.clone()),
        )
        .await
        .unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(provider.calls(), 3);
    assert_eq!(outcome.answer, "verified final");
    assert_eq!(
        outcome
            .messages
            .iter()
            .filter(|m| m.role == Role::User)
            .count(),
        1
    );
    assert_eq!(
        outcome
            .messages
            .iter()
            .filter(|m| m.content == "premature final")
            .count(),
        1
    );
    assert!(outcome.messages.iter().all(
        |m| m.role != Role::System && !m.content.contains("Helm has withheld final acceptance")
    ));
    assert!(
        outcome
            .messages
            .iter()
            .any(|m| m.role == Role::Tool && m.tool_success == Some(true))
    );
    {
        let requests = provider.requests.lock().unwrap();
        assert!(
            requests[0]
                .messages
                .iter()
                .all(|m| !m.content.contains("one bounded reconciliation pass"))
        );
        for request in &requests[1..] {
            assert_eq!(
                request.tools.iter().map(|t| &t.name).collect::<Vec<_>>(),
                requests[0]
                    .tools
                    .iter()
                    .map(|t| &t.name)
                    .collect::<Vec<_>>()
            );
            assert_eq!(request.messages[0].content, requests[0].messages[0].content);
            assert!(
                request.messages.iter().any(|m| m.role == Role::System
                    && m.content.contains("Preserve blocked or deferred"))
            );
        }
    }
    assert_eq!(
        scope.decision().await.unwrap().unwrap().outcome,
        FinalOutcome::Completed
    );
}

#[tokio::test]
async fn root_gate_ignored_update_stops_after_second_proposal_with_unresolved_ids() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let id = fixture.todo(&scope, false).await;
    let provider = Script::new([
        Step::Answer("done"),
        Step::Answer("still done"),
        Step::Answer("must not run"),
    ]);
    let outcome = fixture
        .agent(provider.clone())
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            Some(scope.clone()),
        )
        .await
        .unwrap();
    let StopReason::Incomplete { reason, readiness } = outcome.stop_reason else {
        panic!("unresolved work accepted")
    };
    assert!(reason.contains(&id.0.to_string()));
    assert_eq!(readiness.unwrap().accounted, 0);
    assert_eq!(provider.calls(), 2);
    assert_eq!(
        fixture.todos.snapshot().await.unwrap().items[&id].status,
        TodoStatus::Pending
    );
    assert_eq!(
        scope.decision().await.unwrap().unwrap().outcome,
        FinalOutcome::Incomplete
    );
}

#[tokio::test]
async fn root_gate_accounted_deferral_and_failed_child_are_incomplete() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let id = fixture.todo(&scope, false).await;
    fixture
        .account(
            &scope,
            Obligation::Todo(id),
            DispositionKind::DeferredWithImpact,
        )
        .await;
    let child = fixture
        .runtime
        .spawn_for_run(spawn("fail"), Some(scope.clone()))
        .await
        .unwrap();
    assert!(fixture.runtime.wait(child).await.unwrap().is_err());
    fixture
        .account(
            &scope,
            Obligation::Agent(child),
            DispositionKind::FailureWithImpact,
        )
        .await;
    let provider = Script::new([Step::Answer("blocked"), Step::Answer("still blocked")]);
    let outcome = fixture
        .agent(provider.clone())
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            Some(scope.clone()),
        )
        .await
        .unwrap();
    let StopReason::Incomplete {
        readiness: Some(readiness),
        reason,
    } = outcome.stop_reason
    else {
        panic!("failed work accepted")
    };
    assert!(readiness.ready());
    assert_eq!(readiness.incomplete, 2);
    assert!(reason.contains(&id.0.to_string()) && reason.contains(&child.0.to_string()));
    assert_eq!(provider.calls(), 1);
}

fn spawn(task: &str) -> SpawnRequest {
    let budget = AgentBudget {
        max_tokens: 100,
        max_terminals: 1,
    };
    SpawnRequest {
        parent_id: None,
        name: "child".into(),
        task: task.into(),
        policy: AgentPolicy {
            readable_roots: vec![],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: ApprovalPolicy::Deny,
            budget: budget.clone(),
        },
        budget,
        worktree: None,
        branch: None,
    }
}

#[tokio::test]
async fn root_gate_cancellation_preserves_history_and_stops_only_owned_children() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let unrelated = fixture.scope().await;
    let owned = fixture
        .runtime
        .spawn_for_run(spawn("hold"), Some(scope.clone()))
        .await
        .unwrap();
    let other = fixture
        .runtime
        .spawn_for_run(spawn("hold"), Some(unrelated.clone()))
        .await
        .unwrap();
    let provider = Script::new([Step::Hang]);
    let agent = fixture.agent(provider.clone());
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let scope_copy = scope.clone();
    let run = tokio::spawn(async move {
        agent
            .run_scoped(
                vec![],
                "accepted user prompt".into(),
                token,
                None,
                Some(scope_copy),
            )
            .await
    });
    provider.entered.notified().await;
    cancel.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), run)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    let AgentError::Finalization(failure) = &error else {
        panic!("missing scoped recovery")
    };
    assert!(matches!(failure.source.as_ref(), AgentError::Cancelled));
    assert_eq!(
        error.recovery().unwrap().messages[0].content,
        "accepted user prompt"
    );
    assert!(failure.shutdown.observation_complete && failure.shutdown.remaining.is_empty());
    assert!(
        fixture
            .runtime
            .get(owned)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
    assert!(
        !fixture
            .runtime
            .get(other)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
    assert!(unrelated.decision().await.unwrap().is_none());
    assert_eq!(provider.calls(), 1);
    fixture.runtime.cancel(other).await.unwrap();
    let _ = fixture.runtime.wait(other).await;
}

#[tokio::test]
async fn root_gate_deadline_covers_provider_wait_and_does_not_reset_through_tools() {
    for step in [Step::Hang, Step::ToolLoop] {
        let fixture = Fixture::new();
        let scope = fixture.scope().await;
        fixture.todo(&scope, false).await;
        let provider = Script::new([Step::Answer("proposal"), step]);
        let agent = fixture
            .agent(provider.clone())
            .with_completion_deadlines(Duration::from_millis(40), Duration::from_millis(100));
        let error = tokio::time::timeout(
            Duration::from_secs(2),
            agent.run_scoped(
                vec![],
                "task".into(),
                CancellationToken::new(),
                None,
                Some(scope),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        let AgentError::Finalization(failure) = &error else {
            panic!("missing failure")
        };
        assert!(matches!(
            failure.source.as_ref(),
            AgentError::ReconciliationExpired
        ));
        assert!(
            error
                .recovery()
                .unwrap()
                .messages
                .iter()
                .any(|m| m.content == "proposal")
        );
        assert!(provider.calls() >= 2);
    }
}

#[tokio::test]
async fn root_gate_provider_failure_is_not_retried_or_accepted() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    fixture.todo(&scope, false).await;
    let provider = Script::new([Step::Answer("proposal"), Step::Fail]);
    let error = fixture
        .agent(provider.clone())
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            Some(scope.clone()),
        )
        .await
        .unwrap_err();
    assert_eq!(provider.calls(), 2);
    assert!(
        error
            .recovery()
            .unwrap()
            .messages
            .iter()
            .any(|m| m.content == "proposal")
    );
    assert_eq!(
        scope.decision().await.unwrap().unwrap().outcome,
        FinalOutcome::Interrupted
    );
}

#[tokio::test]
async fn root_gate_inherited_child_scope_does_not_gate_the_root_ledger() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    fixture.todo(&scope, false).await;
    let provider = Script::new([Step::Answer("child result")]);
    let mut agent = fixture.agent(provider.clone());
    agent.context.completion = Some(scope.clone());
    let outcome = agent.run(vec![], "child task".into()).await.unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(provider.calls(), 1);
    assert!(scope.decision().await.unwrap().is_none());
}

#[tokio::test]
async fn root_gate_reconciliation_is_distinct_from_mid_run_steering_support() {
    for unresolved in [false, true] {
        let fixture = Fixture::new();
        let scope = fixture.scope().await;
        if unresolved {
            fixture.todo(&scope, false).await;
        }
        let provider = Arc::new(Script {
            steps: Mutex::new(VecDeque::from([Step::Answer("proposal")])),
            requests: Mutex::new(vec![]),
            entered: tokio::sync::Notify::new(),
            supports: false,
        });
        let result = fixture
            .agent(provider.clone())
            .run_scoped(
                vec![],
                "task".into(),
                CancellationToken::new(),
                None,
                Some(scope),
            )
            .await;
        assert_eq!(provider.calls(), if unresolved { 2 } else { 1 });
        if unresolved {
            assert!(matches!(
                result.unwrap().stop_reason,
                StopReason::Incomplete { .. }
            ));
        } else {
            assert_eq!(result.unwrap().stop_reason, StopReason::Completed);
        }
    }
}

pub(super) fn attach_gate(agent: Agent, directory: &tempfile::TempDir) -> Agent {
    let coordinator = agent.completion_coordinator.clone().unwrap();
    let todos = Arc::new(
        TodoStore::new(
            directory.path().join("gate-todos.json"),
            TodoScope::workspace(directory.path().canonicalize().unwrap()),
        )
        .with_coordinator(coordinator.clone()),
    );
    let agents = AgentTreeStore::new(directory.path().join("gate-agents/tree.json"))
        .with_coordinator(coordinator);
    let runtime = Arc::new(
        SubagentRuntime::new(
            Arc::new(Executor),
            RuntimeLimits::default(),
            Some(agents.clone()),
        )
        .unwrap(),
    );
    agent.with_completion_gate(todos, agents, runtime)
}

#[derive(Default)]
struct AcceptanceCheckpoint {
    run_id: uuid::Uuid,
    canonical_calls: AtomicUsize,
    accepted_calls: AtomicUsize,
    fail_acceptance: bool,
    cancel_at_acceptance: Option<CancellationToken>,
}
#[async_trait]
impl RunCheckpoint for AcceptanceCheckpoint {
    fn run_id(&self) -> uuid::Uuid {
        self.run_id
    }
    async fn canonical(&self, _: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        self.canonical_calls.fetch_add(1, Ordering::SeqCst);
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
        self.accepted_calls.fetch_add(1, Ordering::SeqCst);
        if let Some(cancel) = &self.cancel_at_acceptance {
            cancel.cancel();
        }
        if self.fail_acceptance {
            Err(CheckpointError)
        } else {
            Ok(())
        }
    }
}

#[tokio::test]
async fn root_gate_post_seal_failure_never_emits_stale_success_and_keeps_recovery() {
    for cancellation in [false, true] {
        let fixture = Fixture::new();
        let scope = fixture.scope().await;
        let provider = Script::new([Step::Answer("canonical proposal")]);
        let mut agent = fixture.agent(provider);
        let events = Arc::new(Events::default());
        agent.sink = events.clone();
        let cancel = CancellationToken::new();
        let checkpoint = AcceptanceCheckpoint {
            run_id: scope.run_id(),
            fail_acceptance: !cancellation,
            cancel_at_acceptance: cancellation.then(|| cancel.clone()),
            ..AcceptanceCheckpoint::default()
        };
        let error = agent
            .run_checkpointed_scoped(
                vec![],
                "task".into(),
                cancel,
                None,
                &checkpoint,
                "test".into(),
                Some(scope.clone()),
            )
            .await
            .unwrap_err();
        assert!(
            error
                .recovery()
                .unwrap()
                .messages
                .iter()
                .any(|message| message.content == "canonical proposal")
        );
        assert_eq!(
            scope.decision().await.unwrap().unwrap().outcome,
            FinalOutcome::Completed
        );
        assert_eq!(checkpoint.accepted_calls.load(Ordering::SeqCst), 1);
        assert!(events.0.lock().unwrap().iter().all(|event| !matches!(
            event,
            AgentEvent::CompletionState {
                phase: CompletionPhase::Completed,
                ..
            }
        )));
        assert!(events.0.lock().unwrap().iter().any(|event| matches!(
            event,
            AgentEvent::CompletionState {
                phase: CompletionPhase::Interrupted,
                ..
            }
        )));
    }
}

#[tokio::test]
async fn root_gate_and_legacy_checkpoint_have_explicit_acceptance_hooks() {
    let directory = tempfile::tempdir().unwrap();
    let provider = Script::new([Step::Answer("legacy final")]);
    let agent = super::tests::agent(Box::new(provider), &directory);
    let checkpoint = AcceptanceCheckpoint {
        run_id: uuid::Uuid::new_v4(),
        ..AcceptanceCheckpoint::default()
    };
    let outcome = agent
        .run_checkpointed(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "test".into(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(checkpoint.accepted_calls.load(Ordering::SeqCst), 1);
    assert_eq!(checkpoint.canonical_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn root_gate_missing_store_fails_closed_without_extra_request() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let id = fixture.todo(&scope, false).await;
    std::fs::write(fixture.directory.path().join("todos.json"), "malformed").unwrap();
    let provider = Script::new([Step::Answer("false success")]);
    let error = fixture
        .agent(provider.clone())
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            None,
            Some(scope),
        )
        .await
        .unwrap_err();
    assert_eq!(provider.calls(), 1);
    assert!(
        error
            .recovery()
            .unwrap()
            .messages
            .iter()
            .any(|message| message.content == "false success")
    );
    assert!(error.to_string().contains("completion ownership failed"));
    assert!(!error.to_string().contains(&id.0.to_string()));
}

#[cfg(unix)]
#[tokio::test]
async fn root_gate_compatibility_bridge_rebuilds_thread_for_request_only_reconciliation() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    fixture.todo(&scope, false).await;
    let script = fixture.directory.path().join("fixture-codex");
    std::fs::write(&script, r#"#!/bin/sh
read init
echo '{"id":1,"result":{"userAgent":"fixture","platformFamily":"unix","platformOs":"linux","codexHome":"/tmp"}}'
read initialized
read thread
echo '{"id":2,"result":{"thread":{"id":"th1"}}}'
read turn
echo '{"id":3,"result":{"turn":{"id":"tu1"}}}'
echo '{"method":"item/agentMessage/delta","params":{"threadId":"th1","turnId":"tu1","delta":"first proposal"}}'
echo '{"method":"turn/completed","params":{"threadId":"th1","turn":{"id":"tu1","status":"completed"}}}'
read thread
case "$thread" in *thread/start*"Helm has withheld final acceptance"*) ;; *) exit 2;; esac
echo '{"id":4,"result":{"thread":{"id":"th2"}}}'
read turn
case "$turn" in *original-task*"first proposal"*) ;; *) exit 3;; esac
echo '{"id":5,"result":{"turn":{"id":"tu2"}}}'
echo '{"method":"item/agentMessage/delta","params":{"threadId":"th2","turnId":"tu2","delta":"truthfully incomplete"}}'
echo '{"method":"turn/completed","params":{"threadId":"th2","turn":{"id":"tu2","status":"completed"}}}'
"#).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut agent = fixture.agent(Script::new([]));
    agent.provider = Box::new(crate::provider::CodexSubscriptionProvider::new(
        script.to_string_lossy().into_owned(),
        fixture.directory.path().into(),
    ));
    let outcome = agent
        .run_scoped(
            vec![],
            "original-task".into(),
            CancellationToken::new(),
            None,
            Some(scope),
        )
        .await
        .unwrap();
    assert_eq!(outcome.turns, 2);
    assert!(matches!(outcome.stop_reason, StopReason::Incomplete { .. }));
    assert_eq!(outcome.answer, "truthfully incomplete");
    assert!(
        outcome
            .messages
            .iter()
            .all(|message| message.role != Role::System)
    );
}

struct ReconciliationSteering {
    sender: SteeringSender,
    updates: AtomicUsize,
}
#[async_trait]
impl EventSink for ReconciliationSteering {
    async fn emit(&self, event: AgentEvent) {
        if matches!(
            event,
            AgentEvent::CompletionState {
                phase: CompletionPhase::Reconciling,
                ..
            }
        ) {
            self.updates.fetch_add(1, Ordering::SeqCst);
            self.sender
                .try_send("supervisor correction".into())
                .unwrap();
        }
    }
}

#[tokio::test]
async fn root_gate_keeps_steering_open_during_reconciliation_without_duplicate_proposals() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    fixture.todo(&scope, false).await;
    let provider = Script::new([
        Step::Answer("first proposal"),
        Step::Answer("corrected proposal"),
    ]);
    let (sender, receiver) = steering_channel(4);
    let sink = Arc::new(ReconciliationSteering {
        sender: sender.clone(),
        updates: AtomicUsize::new(0),
    });
    let mut agent = fixture.agent(provider.clone());
    agent.sink = sink.clone();
    let outcome = agent
        .run_scoped(
            vec![],
            "task".into(),
            CancellationToken::new(),
            Some(receiver),
            Some(scope),
        )
        .await
        .unwrap();
    assert!(matches!(outcome.stop_reason, StopReason::Incomplete { .. }));
    assert_eq!(sink.updates.load(Ordering::SeqCst), 1);
    assert_eq!(provider.calls(), 2);
    let steering: Vec<_> = outcome
        .messages
        .iter()
        .filter(|message| message.steering.is_some())
        .collect();
    assert_eq!(steering.len(), 1);
    assert_eq!(steering[0].content, "supervisor correction");
    assert_eq!(
        steering[0].steering.as_ref().unwrap().status,
        crate::model::SteeringStatus::Applied
    );
    assert_eq!(
        outcome
            .messages
            .iter()
            .filter(|message| message.content == "first proposal")
            .count(),
        1
    );
    assert!(matches!(
        sender.try_send("too late".into()),
        Err(SteeringError::Closed(_))
    ));
    assert!(
        provider.requests.lock().unwrap()[1]
            .messages
            .iter()
            .any(|message| message.content == "supervisor correction")
    );
}

#[test]
fn finalization_debug_omits_provider_error_and_canonical_content() {
    let mut message = Message::new(Role::Assistant, "CANONICAL_SENTINEL");
    message.provider_state = Some(serde_json::json!({"private": "CONTINUATION_SENTINEL"}));
    let error = AgentError::Finalization(Box::new(FinalizationFailure {
        source: Box::new(AgentError::Provider(ProviderError::Request(
            "PROVIDER_BODY_SENTINEL".into(),
        ))),
        recovery: CanonicalRecovery {
            messages: vec![message],
            usage: Usage::default(),
        },
        readiness: None,
        shutdown: OwnedShutdown::default(),
    }));
    let debug = format!("{error:?}");
    assert!(!debug.contains("SENTINEL"));
    assert!(debug.contains("provider") && debug.contains("message_count"));
}

struct PartialFailure;
#[async_trait]
impl Provider for PartialFailure {
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        unreachable!("stream fixture")
    }
    async fn stream(
        &self,
        _: ModelRequest,
    ) -> Result<crate::provider::ProviderStream, ProviderError> {
        use crate::provider::{ProviderDelta, ProviderStreamEvent};
        Ok(Box::pin(futures_util::stream::iter([
            Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
                "partial canonical output".into(),
            ))),
            Err(ProviderError::Unavailable("stream stopped".into())),
        ])))
    }
}

#[tokio::test]
async fn root_gate_preserves_partial_stream_on_provider_failure() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let mut agent = fixture.agent(Script::new([]));
    agent.provider = Box::new(PartialFailure);
    let error = agent
        .run_scoped(
            vec![],
            "original user input".into(),
            CancellationToken::new(),
            None,
            Some(scope.clone()),
        )
        .await
        .unwrap_err();
    let history = &error.recovery().unwrap().messages;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "original user input");
    assert_eq!(history[1].role, Role::Assistant);
    assert_eq!(history[1].content, "partial canonical output");
    assert_eq!(
        scope.decision().await.unwrap().unwrap().outcome,
        FinalOutcome::Interrupted
    );
}

#[tokio::test]
async fn owned_shutdown_includes_descendants_of_terminal_parents() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let parent = fixture
        .runtime
        .spawn_for_run(spawn("message"), Some(scope.clone()))
        .await
        .unwrap();
    let mut child_request = spawn("hold");
    child_request.parent_id = Some(parent);
    let child = fixture.runtime.spawn(child_request).await.unwrap();
    fixture
        .runtime
        .send_message(parent, "finish parent only")
        .await
        .unwrap();
    fixture.runtime.wait(parent).await.unwrap().unwrap();
    assert!(
        !fixture
            .runtime
            .get(child)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
    let agent = fixture.agent(Script::new([]));
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&scope)
        .await;
    assert!(report.observation_complete && report.remaining.is_empty());
    assert!(
        fixture
            .runtime
            .get(child)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
    assert!(
        fixture
            .runtime
            .pending_owned_shutdown(&scope.reference())
            .await
            .unwrap()
            .is_empty()
    );
}

async fn running_owned_child(fixture: &Fixture, scope: &RunHandle) -> crate::subagent::AgentId {
    let id = fixture
        .runtime
        .spawn_for_run(spawn("hold"), Some(scope.clone()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if fixture
                .agents
                .get(id)
                .await
                .unwrap()
                .is_some_and(|r| r.status == crate::subagent::AgentStatus::Running)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn owned_shutdown_does_not_claim_blocked_terminal_persistence_finished() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let child = running_owned_child(&fixture, &scope).await;
    let guard = fixture.coordinator.lock().await.unwrap();
    let agent = fixture.agent(Script::new([]));
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&scope)
        .await;
    assert!(!report.observation_complete);
    assert!(report.remaining.contains(&child));
    assert!(
        fixture
            .runtime
            .get(child)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
    assert_eq!(
        fixture.agents.get(child).await.unwrap().unwrap().status,
        crate::subagent::AgentStatus::Running
    );
    drop(guard);
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&scope)
        .await;
    assert!(report.observation_complete && report.remaining.is_empty());
}

#[tokio::test]
async fn owned_shutdown_failed_persistence_is_inconclusive() {
    let fixture = Fixture::new();
    let scope = fixture.scope().await;
    let child = running_owned_child(&fixture, &scope).await;
    let path = fixture.directory.path().join("agents/tree.json");
    let backup = fixture.directory.path().join("agents/tree.backup");
    std::fs::rename(&path, &backup).unwrap();
    std::fs::create_dir(&path).unwrap();
    let agent = fixture.agent(Script::new([]));
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&scope)
        .await;
    assert!(!report.observation_complete);
    assert!(report.remaining.contains(&child));
    fixture.runtime.wait(child).await.unwrap().unwrap_err();
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(&backup, &path).unwrap();
    // The worker ended, but its durable record never recorded that transition.
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&scope)
        .await;
    assert!(!report.observation_complete);
    assert!(report.remaining.contains(&child));
}

#[tokio::test]
async fn owned_shutdown_does_not_cancel_a_new_runs_adopted_followup() {
    // Keep an ancestor running while a separate child finishes. This fixture
    // requires two slots independent of the host's CPU-based production default.
    let fixture = Fixture::with_limits(RuntimeLimits {
        max_concurrency: 2,
        ..RuntimeLimits::default()
    });
    let original = fixture.scope().await;
    let adopter = fixture.scope().await;
    let ancestor = fixture
        .runtime
        .spawn_for_run(spawn("hold"), Some(original.clone()))
        .await
        .unwrap();
    let mut parent_request = spawn("message");
    parent_request.parent_id = Some(ancestor);
    parent_request.worktree = Some(fixture.directory.path().to_owned());
    let parent = fixture.runtime.spawn(parent_request).await.unwrap();
    fixture
        .runtime
        .send_message(parent, "finish parent")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), fixture.runtime.wait(parent))
        .await
        .expect("adoptable parent did not finish")
        .unwrap()
        .unwrap();
    adopter
        .adopt_existing(
            &fixture.todos,
            &fixture.agents,
            Obligation::Agent(parent),
            0,
        )
        .await
        .unwrap();
    let followup = fixture
        .runtime
        .follow_up_in_run(None, parent, "hold", Some(adopter.clone()))
        .await
        .unwrap();
    let agent = fixture.agent(Script::new([]));
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&original)
        .await;
    assert!(report.observation_complete);
    assert!(
        !fixture
            .runtime
            .get(followup)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
    let report = agent
        .completion_gate
        .as_ref()
        .unwrap()
        .shutdown_owned(&adopter)
        .await;
    assert!(report.observation_complete);
    assert!(
        fixture
            .runtime
            .get(followup)
            .await
            .unwrap()
            .status
            .is_terminal()
    );
}
