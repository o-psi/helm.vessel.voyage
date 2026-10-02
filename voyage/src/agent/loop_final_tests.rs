//! Offline loop boundary regressions; all provider and tool effects are scripted.
use super::*;
use crate::{
    model::{ModelResponse, Role, ToolCall},
    provider::{ProviderDelta, ProviderStream, ProviderStreamEvent},
    tools::{InteractionMode, Tool, ToolError, UnattendedApprover},
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
struct Observed {
    requests: Mutex<Vec<ModelRequest>>,
    events: Mutex<Vec<AgentEvent>>,
    discoveries: AtomicUsize,
    executions: AtomicUsize,
}
#[async_trait]
impl EventSink for Observed {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}
enum Step {
    Answer(Box<Message>),
    Events(Vec<Result<ProviderStreamEvent, ProviderError>>),
    Fail,
    Pending,
    Steer(SteeringSender),
}
struct Script {
    observed: Arc<Observed>,
    steps: Mutex<VecDeque<Step>>,
    steering: bool,
}
fn response(message: Message) -> ModelResponse {
    ModelResponse {
        message,
        usage: Usage {
            input_tokens: 7,
            output_tokens: 3,
        },
        service_tier: None,
    }
}
fn answer(text: &str) -> Step {
    Step::Answer(Box::new(Message::new(Role::Assistant, text)))
}
fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}
fn calls(calls: Vec<ToolCall>) -> Step {
    let mut message = Message::new(Role::Assistant, "dispatch");
    message.tool_calls = calls;
    Step::Answer(Box::new(message))
}
#[async_trait]
impl Provider for Script {
    fn supports_steering(&self) -> bool {
        self.steering
    }
    fn context_window(&self, _: &str) -> Option<usize> {
        Some(4096)
    }
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        self.observed.discoveries.fetch_add(1, Ordering::SeqCst);
        Ok(vec![
            ModelInfo::minimal(crate::titles::title_model().unwrap()),
            ModelInfo::minimal("z-model"),
        ])
    }
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        panic!("loop must use streaming interface")
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        self.observed.requests.lock().unwrap().push(request);
        let step = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected request");
        let events = match step {
            Step::Answer(message) => vec![Ok(ProviderStreamEvent::Completed(Box::new(response(
                *message,
            ))))],
            Step::Events(events) => events,
            Step::Fail => return Err(ProviderError::Request("scripted refusal".into())),
            Step::Pending => return Ok(Box::pin(futures_util::stream::pending())),
            Step::Steer(sender) => {
                sender
                    .try_send("Use the corrected requirement".into())
                    .unwrap();
                vec![Ok(ProviderStreamEvent::Completed(Box::new(response(
                    Message::new(Role::Assistant, "superseded"),
                ))))]
            }
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}
struct Probe(Arc<Observed>);
#[async_trait]
impl Tool for Probe {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "loop_probe".into(),
            description: "Deterministic loop probe".into(),
            input_schema: serde_json::json!({"type":"object","properties":{"fail":{"type":"boolean"}},"additionalProperties":false}),
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(
        &self,
        arguments: serde_json::Value,
        _: &ToolContext,
    ) -> Result<String, ToolError> {
        self.0.executions.fetch_add(1, Ordering::SeqCst);
        if arguments["fail"] == true {
            Err(ToolError::Failed("probe failed".into()))
        } else {
            Ok("probe succeeded".into())
        }
    }
}
fn fixture(root: &std::path::Path, steps: Vec<Step>, steering: bool) -> (Agent, Arc<Observed>) {
    let observed = Arc::new(Observed::default());
    let config = crate::Config {
        access: Some(AccessMode::Unrestricted),
        ..Default::default()
    };
    let context = ToolContext {
        tool_call_id: None,
        artifact_scope: None,
        github: None,
        completion: None,
        policy: Arc::new(crate::policy::Policy::new(&config, root.into()).unwrap()),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(2),
        max_output_bytes: 200_000,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(crate::tools::Redactor::new(["SECRET_LOOP_VALUE".into()])),
    };
    let mut registry = ToolRegistry::default();
    registry.register(Probe(observed.clone()));
    let agent = Agent::new(
        Box::new(Script {
            observed: observed.clone(),
            steps: Mutex::new(steps.into()),
            steering,
        }),
        registry,
        context,
        observed.clone(),
        "loop-model".into(),
        "loop system".into(),
        128,
        None,
    )
    .with_retry_policy(RetryPolicy {
        max_attempts: 1,
        ..Default::default()
    });
    (agent, observed)
}

#[derive(Default)]
struct Journal {
    id: uuid::Uuid,
    snapshots: Mutex<Vec<Vec<Message>>>,
    partials: Mutex<Vec<String>>,
    unstreamed: Mutex<Vec<String>>,
    accepted: AtomicUsize,
    fail: Option<&'static str>,
}
impl Journal {
    fn new(fail: Option<&'static str>) -> Self {
        Self {
            id: uuid::Uuid::new_v4(),
            fail,
            ..Default::default()
        }
    }
}
#[async_trait]
impl RunCheckpoint for Journal {
    fn run_id(&self) -> uuid::Uuid {
        self.id
    }
    async fn canonical(&self, messages: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        if self.fail == Some("canonical") {
            return Err(CheckpointError);
        }
        self.snapshots.lock().unwrap().push(messages.to_vec());
        Ok(())
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        if self.fail == Some("partial") {
            return Err(CheckpointError);
        }
        self.partials.lock().unwrap().push(text.into());
        Ok(())
    }
    async fn unstreamed(&self, text: &str) -> Result<(), CheckpointError> {
        if self.fail == Some("unstreamed") {
            return Err(CheckpointError);
        }
        self.unstreamed.lock().unwrap().push(text.into());
        Ok(())
    }
    async fn accepted(
        &self,
        _: &[Message],
        _: &Usage,
        _: &StopReason,
    ) -> Result<(), CheckpointError> {
        if self.fail == Some("accepted") {
            return Err(CheckpointError);
        }
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
async fn checkpointed(agent: &Agent, journal: &Journal) -> Result<AgentOutcome, AgentError> {
    agent
        .run_checkpointed(
            vec![],
            "Do the work".into(),
            CancellationToken::new(),
            None,
            journal,
            "selected-model".into(),
        )
        .await
}

#[tokio::test]
async fn tool_batch_records_failures_and_continues_without_replaying_duplicates() {
    let root = tempfile::tempdir().unwrap();
    let first = call("one", "loop_probe", serde_json::json!({}));
    let (agent, observed) = fixture(
        root.path(),
        vec![
            calls(vec![
                first.clone(),
                first,
                call("two", "loop_probe", serde_json::json!({"fail":true})),
                call("three", "absent_tool", serde_json::json!({})),
                call("four", "loop_probe", serde_json::json!({"extra":1})),
            ]),
            answer("Finished after examining failures"),
        ],
        true,
    );
    let journal = Journal::new(None);
    let outcome = checkpointed(&agent, &journal).await.unwrap();
    assert_eq!(outcome.turns, 2);
    assert_eq!(outcome.usage.input_tokens, 14);
    assert_eq!(outcome.usage.output_tokens, 6);
    assert_eq!(observed.executions.load(Ordering::SeqCst), 2);
    let results: Vec<_> = outcome
        .messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .collect();
    assert_eq!(results.len(), 4);
    assert!(results[0].tool_outcome.as_ref().unwrap().success());
    assert!(
        results[1..]
            .iter()
            .all(|m| !m.tool_outcome.as_ref().unwrap().success())
    );
    assert!(results.iter().all(|m| m.tool_output.is_some()));
    assert_eq!(journal.accepted.load(Ordering::SeqCst), 1);
    assert_eq!(journal.unstreamed.lock().unwrap().len(), 2);
    let requests = observed.requests.lock().unwrap();
    assert_eq!(requests[1].model, "selected-model");
    assert_eq!(
        requests[1]
            .messages
            .iter()
            .filter(|m| m.role == Role::Tool)
            .count(),
        4
    );
    let events = observed.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolStarted { .. }))
            .count(),
        4
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, AgentEvent::ToolFinished { success: false, .. }))
            .count(),
        3
    );
}

#[tokio::test]
async fn invalid_call_batches_never_admit_even_the_valid_first_call() {
    let root = tempfile::tempdir().unwrap();
    for invalid in [
        call("", "loop_probe", serde_json::json!({})),
        call("\n", "loop_probe", serde_json::json!({})),
        call(&"x".repeat(1025), "loop_probe", serde_json::json!({})),
        call("second", " ", serde_json::json!({})),
        call("second", "bad\nname", serde_json::json!({})),
        call("valid", "loop_probe", serde_json::json!({"fail":true})),
    ] {
        let (agent, observed) = fixture(
            root.path(),
            vec![calls(vec![
                call("valid", "loop_probe", serde_json::json!({})),
                invalid,
            ])],
            true,
        );
        let journal = Journal::new(None);
        assert!(matches!(
            checkpointed(&agent, &journal).await,
            Err(AgentError::Provider(ProviderError::InvalidResponse(_)))
        ));
        assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
        assert!(
            journal
                .snapshots
                .lock()
                .unwrap()
                .iter()
                .all(|messages| messages.iter().all(|m| m.tool_calls.is_empty()))
        );
        assert_eq!(journal.accepted.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn historical_missing_results_are_projected_unknown_not_executed_or_persisted() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![answer("Inspected only")], true);
    let mut historical = Message::new(Role::Assistant, "prior dispatch");
    historical
        .tool_calls
        .push(call("old", "loop_probe", serde_json::json!({})));
    let outcome = agent
        .run(
            vec![Message::new(Role::System, "obsolete system"), historical],
            "Inspect, do not replay".into(),
        )
        .await
        .unwrap();
    assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
    assert!(
        outcome
            .messages
            .iter()
            .all(|m| m.role != Role::System && m.role != Role::Tool)
    );
    let requests = observed.requests.lock().unwrap();
    let projected = requests[0]
        .messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert_eq!(projected.tool_call_id.as_deref(), Some("old"));
    assert!(projected.content.contains("unknown"));
    assert!(
        !requests[0]
            .messages
            .iter()
            .any(|m| m.content == "obsolete system")
    );
}

#[tokio::test]
async fn pending_steering_receipts_are_canonical_before_request_and_channel_closes() {
    let root = tempfile::tempdir().unwrap();
    let (sender, receiver) = steering_channel(2);
    sender.try_send("Correct the objective".into()).unwrap();
    let (agent, observed) = fixture(root.path(), vec![answer("Corrected")], true);
    let outcome = agent
        .run_with_cancel_and_input(
            vec![],
            "Original".into(),
            CancellationToken::new(),
            Some(receiver),
        )
        .await
        .unwrap();
    let steering = outcome
        .messages
        .iter()
        .find(|m| m.steering.is_some())
        .unwrap();
    assert_eq!(
        steering.steering.as_ref().unwrap().status,
        crate::model::SteeringStatus::Applied
    );
    assert_eq!(
        observed.requests.lock().unwrap()[0]
            .messages
            .iter()
            .filter(|m| m.content == "Correct the objective")
            .count(),
        1
    );
    assert!(
        observed
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, AgentEvent::SteeringApplied { .. }))
    );
    assert!(matches!(
        sender.try_send("late".into()),
        Err(SteeringError::Closed(_))
    ));
}

#[tokio::test]
async fn steering_arriving_during_inference_requires_a_fresh_answer() {
    let root = tempfile::tempdir().unwrap();
    let (sender, receiver) = steering_channel(1);
    let (agent, observed) = fixture(
        root.path(),
        vec![Step::Steer(sender.clone()), answer("Corrected final")],
        true,
    );
    let outcome = agent
        .run_with_cancel_and_input(
            vec![],
            "Original".into(),
            CancellationToken::new(),
            Some(receiver),
        )
        .await
        .unwrap();
    assert_eq!(outcome.turns, 2);
    assert_eq!(outcome.answer, "Corrected final");
    assert!(
        observed.requests.lock().unwrap()[1]
            .messages
            .iter()
            .any(|m| m.content == "Use the corrected requirement")
    );
    assert!(matches!(
        sender.try_send("late".into()),
        Err(SteeringError::Closed(_))
    ));
}

#[tokio::test]
async fn compatibility_provider_refuses_steering_before_or_after_inference() {
    let root = tempfile::tempdir().unwrap();
    for during in [false, true] {
        let (sender, receiver) = steering_channel(1);
        let steps = if during {
            vec![Step::Steer(sender)]
        } else {
            sender.try_send("correction".into()).unwrap();
            vec![]
        };
        let (agent, observed) = fixture(root.path(), steps, false);
        let error = agent
            .run_with_cancel_and_input(
                vec![],
                "Original".into(),
                CancellationToken::new(),
                Some(receiver),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("active steering is unavailable"));
        assert_eq!(observed.requests.lock().unwrap().len(), usize::from(during));
    }
}

#[tokio::test]
async fn steering_backpressure_wakes_on_drain_and_receiver_drop() {
    let (sender, mut receiver) = steering_channel(0);
    sender.try_send("first".into()).unwrap();
    assert!(matches!(
        sender.try_send("full".into()),
        Err(SteeringError::Full(_))
    ));
    let waiting = sender.send("second".into());
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), &mut waiting)
            .await
            .is_err()
    );
    assert_eq!(receiver.drain()[0].content, "first");
    waiting.await.unwrap();
    assert_eq!(receiver.drain_or_close()[0].content, "second");
    assert!(receiver.drain_or_close().is_empty());
    assert!(sender.send("closed".into()).await.is_err());
    let (sender, receiver) = steering_channel(1);
    sender.try_send("occupied".into()).unwrap();
    let waiting = sender.send("blocked".into());
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(Duration::from_millis(5), &mut waiting)
            .await
            .is_err()
    );
    drop(receiver);
    assert_eq!(waiting.await.unwrap_err().0, "blocked");
    let (sender, _receiver) = steering_channel(1);
    assert!(matches!(
        sender.try_send("x".repeat(MAX_STEERING_BYTES + 1)),
        Err(SteeringError::TooLarge(_))
    ));
    assert!(
        sender
            .send("x".repeat(MAX_STEERING_BYTES + 1))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn checkpoint_failures_stop_at_the_correct_publication_boundary() {
    let root = tempfile::tempdir().unwrap();
    for phase in ["canonical", "unstreamed", "accepted"] {
        let (agent, observed) = fixture(root.path(), vec![answer("done")], true);
        let journal = Journal::new(Some(phase));
        assert!(matches!(
            checkpointed(&agent, &journal).await,
            Err(AgentError::Checkpoint(_))
        ));
        assert_eq!(journal.accepted.load(Ordering::SeqCst), 0);
        assert_eq!(
            observed.requests.lock().unwrap().len(),
            usize::from(phase != "canonical")
        );
        if phase == "unstreamed" {
            assert!(
                !observed
                    .events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| matches!(e, AgentEvent::AssistantText(_)))
            );
        }
    }
    let (agent, observed) = fixture(root.path(), vec![], true);
    let journal = Journal::default(); // nil IDs cannot claim a durable run.
    assert!(matches!(
        checkpointed(&agent, &journal).await,
        Err(AgentError::Checkpoint(_))
    ));
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn streamed_text_is_not_projected_again_as_unstreamed() {
    let root = tempfile::tempdir().unwrap();
    let (agent, _) = fixture(
        root.path(),
        vec![Step::Events(vec![
            Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
                "streamed".into(),
            ))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(
                Message::new(Role::Assistant, "streamed"),
            )))),
        ])],
        true,
    );
    let journal = Journal::new(None);
    assert_eq!(
        checkpointed(&agent, &journal).await.unwrap().answer,
        "streamed"
    );
    assert_eq!(journal.partials.lock().unwrap().concat(), "streamed");
    assert!(journal.unstreamed.lock().unwrap().is_empty());
    assert_eq!(journal.accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn direct_partial_projection_skips_empty_and_stops_before_event_on_failure() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], true);
    let mut partial = String::new();
    let journal = Journal::new(Some("partial"));
    agent
        .project_provider_text(String::new(), Some(&journal), &mut partial)
        .await
        .unwrap();
    assert!(matches!(
        agent
            .project_provider_text("partial".into(), Some(&journal), &mut partial)
            .await,
        Err(AgentError::Checkpoint(_))
    ));
    assert_eq!(partial, "partial");
    assert!(observed.events.lock().unwrap().is_empty());
    agent
        .project_provider_text(" remainder".into(), None, &mut partial)
        .await
        .unwrap();
    assert_eq!(partial, "partial remainder");
    assert!(
        matches!(&observed.events.lock().unwrap()[0], AgentEvent::AssistantTextDelta(text) if text == " remainder")
    );
}

#[tokio::test]
async fn cancellation_prevents_dispatch_and_interrupts_a_pending_stream() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], true);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        agent.run_with_cancel(vec![], "work".into(), cancel).await,
        Err(AgentError::Cancelled)
    ));
    assert!(observed.requests.lock().unwrap().is_empty());
    assert!(
        observed
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, AgentEvent::Cancelled))
    );
    let (agent, observed) = fixture(root.path(), vec![Step::Pending], true);
    let cancel = CancellationToken::new();
    let run = agent.run_with_cancel(vec![], "work".into(), cancel.clone());
    let interrupt = async {
        tokio::time::timeout(Duration::from_secs(2), async {
            while observed.requests.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(run, interrupt);
    assert!(matches!(result, Err(AgentError::Cancelled)));
}

#[tokio::test]
async fn provider_failure_does_not_accept_or_execute_tools() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![Step::Fail], true);
    let journal = Journal::new(None);
    assert!(matches!(
        checkpointed(&agent, &journal).await,
        Err(AgentError::Provider(ProviderError::Request(_)))
    ));
    assert_eq!(journal.accepted.load(Ordering::SeqCst), 0);
    assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
    assert_eq!(observed.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn model_selection_cache_and_context_ceiling_remain_separate() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], true);
    let mirror = Arc::new(RwLock::new(String::new()));
    let agent = agent.with_model_mirror(mirror.clone());
    assert_eq!(agent.context_limit("loop-model"), 0);
    let agent = agent.with_context_window(8192);
    assert_eq!(agent.context_limit("loop-model"), 4096);
    assert!(agent.set_model("  ").is_err());
    assert_eq!(agent.set_model("  new-model  ").unwrap(), "new-model");
    assert_eq!(*mirror.read().unwrap(), "new-model");
    let models = agent.models(false).await.unwrap();
    assert!(models.iter().any(|m| m.id == "new-model"));
    agent.models(false).await.unwrap();
    assert_eq!(observed.discoveries.load(Ordering::SeqCst), 1);
    agent.models(true).await.unwrap();
    assert_eq!(observed.discoveries.load(Ordering::SeqCst), 2);
    assert!(
        !agent
            .redact_diagnostic("SECRET_LOOP_VALUE")
            .contains("SECRET_LOOP_VALUE")
    );
    assert_eq!(agent.workspace(), root.path());
}

#[tokio::test]
async fn title_request_is_isolated_and_accounts_completed_response() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![answer("Repair runtime loop tests")], true);
    let title = agent
        .generate_title(
            &[
                Message::new(Role::Assistant, "do not include this response"),
                Message::new(Role::User, "Repair SECRET_LOOP_VALUE runtime tests"),
            ],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(title.title.as_deref(), Some("Repair runtime loop tests"));
    assert_eq!(title.usage.input_tokens, 7);
    let requests = observed.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].model, crate::titles::title_model().unwrap());
    assert_eq!(requests[0].max_tokens, Some(128));
    assert!(requests[0].tools.is_empty());
    assert!(
        !requests[0]
            .messages
            .iter()
            .any(|m| m.content.contains("SECRET_LOOP_VALUE")
                || m.content.contains("do not include this response"))
    );
    assert!(observed.events.lock().unwrap().is_empty());
}

#[tokio::test]
async fn title_rejects_failed_incomplete_oversized_and_tool_streams() {
    let root = tempfile::tempdir().unwrap();
    for step in [
        Step::Fail,
        Step::Events(vec![]),
        Step::Events(vec![Err(ProviderError::Request("failed title".into()))]),
        Step::Events(vec![Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
            "x".repeat(4097),
        )))]),
        Step::Events(vec![Ok(ProviderStreamEvent::Delta(
            ProviderDelta::ToolCall {
                index: 0,
                id: Some("x".into()),
                name: Some("loop_probe".into()),
                arguments: "{}".into(),
            },
        ))]),
    ] {
        let (agent, observed) = fixture(root.path(), vec![step], true);
        assert!(
            agent
                .generate_title(
                    &[Message::new(Role::User, "name this work")],
                    CancellationToken::new()
                )
                .await
                .is_none()
        );
        assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
    }
    let (agent, observed) = fixture(root.path(), vec![], true);
    assert!(
        agent
            .generate_title(&[], CancellationToken::new())
            .await
            .is_none()
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        agent
            .generate_title(&[Message::new(Role::User, "work")], cancel)
            .await
            .is_none()
    );
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn title_ignores_nontext_activity_and_preserves_invalid_title_usage() {
    let root = tempfile::tempdir().unwrap();
    let (agent, _) = fixture(
        root.path(),
        vec![Step::Events(vec![
            Ok(ProviderStreamEvent::Activity),
            Ok(ProviderStreamEvent::ResponseMetadata {
                status: 200,
                request_id: Some("title-request".into()),
            }),
            Ok(ProviderStreamEvent::UsageReported(
                crate::provider::ReportedUsage {
                    input_tokens: Some(7),
                    output_tokens: Some(3),
                },
            )),
            Ok(ProviderStreamEvent::Delta(ProviderDelta::Text(
                "partial".into(),
            ))),
            Ok(ProviderStreamEvent::Completed(Box::new(response(
                Message::new(Role::Assistant, ""),
            )))),
        ])],
        true,
    );
    let session = crate::session::Session::new(root.path().into(), "loop-model".into());
    let mut session = session;
    session
        .messages
        .push(Message::new(Role::User, "Name this work"));
    let title = agent
        .generate_title_for_session(&session, CancellationToken::new())
        .await
        .unwrap();
    assert!(title.title.is_none());
    assert_eq!(title.usage.output_tokens, 3);
}

#[tokio::test]
async fn manual_operator_controls_reuse_registry_and_refuse_private_input() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], true);
    let session = uuid::Uuid::new_v4();
    let run = uuid::Uuid::new_v4();
    let output = agent
        .operator_tool(
            session,
            run,
            CancellationToken::new(),
            "loop_probe",
            serde_json::json!({}),
        )
        .await
        .unwrap();
    assert!(output.contains("probe succeeded"));
    assert_eq!(observed.executions.load(Ordering::SeqCst), 1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        agent
            .operator_tool(session, run, cancel, "loop_probe", serde_json::json!({}))
            .await
            .unwrap_err()
            .to_string()
            .contains("closed")
    );
    assert!(
        agent
            .operator_tool(
                session,
                run,
                CancellationToken::new(),
                "process",
                serde_json::json!({"action":"write","data":"private"})
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("private terminal")
    );
    assert!(
        agent
            .operator_tool(
                session,
                run,
                CancellationToken::new(),
                "absent",
                serde_json::json!({})
            )
            .await
            .is_err()
    );
    assert_eq!(observed.executions.load(Ordering::SeqCst), 1);
    assert!(
        agent
            .operator_arguments(&serde_json::json!({"nested":["SECRET_LOOP_VALUE"]}))
            .is_err()
    );
    agent
        .operator_arguments(&serde_json::json!({"safe":true}))
        .unwrap();
    assert!(
        agent.operator_policy().unwrap()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "loop_probe")
    );
    assert!(
        agent
            .operator_todos()
            .await
            .unwrap_err()
            .to_string()
            .contains("task store unavailable")
    );
    assert!(agent.operator_lease(session, run).await.is_err());
    assert!(agent.operator_terminals().is_none());
    assert!(agent.terminal_metadata().is_empty());
    agent
        .operator_run_lifecycle(run, CancellationToken::new(), "run_finish")
        .await;
    agent.github_operator_authority(false).unwrap();
    agent.github_operator_authority(true).unwrap();
    let github_cancel = CancellationToken::new();
    github_cancel.cancel();
    assert!(
        agent
            .github_command(session, vec!["status".into()], github_cancel)
            .await
            .is_err()
    );
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn default_checkpoint_interfaces_are_explicitly_nonpersistent() {
    let journal = Journal::new(None);
    let context = journal.working_context().await.unwrap();
    assert!(journal.save_working_context(&context).await.is_err());
    journal.tool_previews(&[]).await.unwrap();
    journal.reasoning_previews(&[]).await.unwrap();
    let messages = vec![Message::new(Role::User, "canonical")];
    let reconciled = journal
        .reconciled(&messages, &Usage::default())
        .await
        .unwrap();
    assert_eq!(reconciled[0].content, "canonical");
    assert_eq!(journal.snapshots.lock().unwrap().len(), 1);
    let root = tempfile::tempdir().unwrap();
    let (agent, _) = fixture(root.path(), vec![], true);
    let session = crate::session::Session::new(root.path().into(), "loop-model".into());
    assert!(agent.prepare_run(&session).await.unwrap().is_none());
}

#[tokio::test]
async fn usage_overflow_rejects_response_before_publication() {
    let root = tempfile::tempdir().unwrap();
    for input_overflows in [false, true] {
        let mut first = Message::new(Role::Assistant, "first");
        first
            .tool_calls
            .push(call("one", "loop_probe", serde_json::json!({})));
        let usage = if input_overflows {
            Usage {
                input_tokens: u64::MAX,
                output_tokens: 0,
            }
        } else {
            Usage {
                input_tokens: 0,
                output_tokens: u64::MAX,
            }
        };
        let (agent, observed) = fixture(
            root.path(),
            vec![
                Step::Events(vec![Ok(ProviderStreamEvent::Completed(Box::new(
                    ModelResponse {
                        message: first,
                        usage,
                        service_tier: None,
                    },
                )))]),
                answer("must not be published"),
            ],
            true,
        );
        let journal = Journal::new(None);
        assert!(matches!(
            checkpointed(&agent, &journal).await,
            Err(AgentError::UsageOverflow)
        ));
        assert_eq!(observed.executions.load(Ordering::SeqCst), 1);
        assert_eq!(journal.accepted.load(Ordering::SeqCst), 0);
        assert!(
            journal
                .snapshots
                .lock()
                .unwrap()
                .iter()
                .flatten()
                .all(|m| m.content != "must not be published")
        );
    }
}

#[derive(Debug)]
struct Revoked;
impl crate::policy::ExecutionAuthority for Revoked {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::bail!("loop authority revoked")
    }
}

#[tokio::test]
async fn revoked_authority_blocks_public_operator_and_inference_entrypoints() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, observed) = fixture(root.path(), vec![], true);
    agent.context.policy = Arc::new(
        (*agent.context.policy)
            .clone()
            .with_execution_authority(Arc::new(Revoked)),
    );
    assert!(matches!(
        agent.run(vec![], "work".into()).await,
        Err(AgentError::Policy(_))
    ));
    assert!(matches!(
        agent.models(false).await,
        Err(AgentError::Policy(_))
    ));
    assert!(agent.operator_policy().is_err());
    assert!(agent.operator_todos().await.is_err());
    assert!(agent.github_operator_authority(false).is_err());
    assert!(
        agent
            .operator_tool(
                uuid::Uuid::new_v4(),
                uuid::Uuid::new_v4(),
                CancellationToken::new(),
                "loop_probe",
                serde_json::json!({})
            )
            .await
            .is_err()
    );
    assert!(
        agent
            .generate_title(
                &[Message::new(Role::User, "work")],
                CancellationToken::new()
            )
            .await
            .is_none()
    );
    let session = crate::session::Session::new(root.path().into(), "loop-model".into());
    assert!(agent.prepare_run(&session).await.is_err());
    assert_eq!(observed.discoveries.load(Ordering::SeqCst), 0);
    assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn github_operator_help_is_available_without_provider_or_github_authority() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], false);
    let result = agent
        .github_command(
            uuid::Uuid::new_v4(),
            vec!["--help".into()],
            CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(result.display.contains("Usage:"));
    assert!(result.reference.is_none() && result.feedback.is_none());
    assert!(observed.requests.lock().unwrap().is_empty());
    assert_eq!(observed.discoveries.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn operator_cloned_context_keeps_explicit_approval_and_refuses_disabled_github() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], false);
    let error = agent
        .github_command_with_approver(
            uuid::Uuid::new_v4(),
            vec!["auth".into()],
            CancellationToken::new(),
            Arc::new(UnattendedApprover { allow: true }),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("capability is disabled"));
    assert!(observed.requests.lock().unwrap().is_empty());
    assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn read_only_operator_permission_distinguishes_reference_inspection_from_edits() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, observed) = fixture(root.path(), vec![], false);
    let config = crate::Config {
        access: Some(AccessMode::ReadOnly),
        ..Default::default()
    };
    agent.context.policy =
        Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap());
    agent.github_operator_authority(false).unwrap();
    assert!(
        agent
            .github_operator_authority(true)
            .unwrap_err()
            .to_string()
            .contains("read-only")
    );
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn inventory_presentation_redacts_controls_without_changing_executable_definitions() {
    struct Metadata;
    #[async_trait]
    impl Tool for Metadata {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "metadata_probe".into(),
                description: "Visible\nSECRET_LOOP_VALUE\u{202e}suffix".into(),
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
            }
        }
        async fn execute(
            &self,
            _: serde_json::Value,
            _: &ToolContext,
        ) -> Result<String, ToolError> {
            panic!("presentation must never execute a tool")
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (mut agent, observed) = fixture(root.path(), vec![], false);
    agent.tools.register(Metadata);
    let definitions = agent.tool_inventory();
    let original = definitions
        .iter()
        .find(|definition| definition.name == "metadata_probe")
        .unwrap();
    assert!(original.description.contains("SECRET_LOOP_VALUE"));
    let display = agent.tool_inventory_display();
    let display = display
        .iter()
        .find(|display| display.starts_with("metadata_probe"))
        .unwrap();
    assert!(display.contains("[REDACTED]"));
    assert!(display.contains("\\n"));
    assert!(display.contains("\\u{202e}"));
    assert!(!display.chars().any(char::is_control));
    assert!(!display.contains("SECRET_LOOP_VALUE"));
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn inventory_escape_cannot_form_a_configured_secret_on_the_public_surface() {
    struct Metadata;
    #[async_trait]
    impl Tool for Metadata {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "escape_probe".into(),
                description: "first\nsecond".into(),
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: None,
                annotations: None,
            }
        }
        async fn execute(
            &self,
            _: serde_json::Value,
            _: &ToolContext,
        ) -> Result<String, ToolError> {
            panic!("metadata only")
        }
    }
    let root = tempfile::tempdir().unwrap();
    let (mut agent, _) = fixture(root.path(), vec![], false);
    agent.tools.register(Metadata);
    agent.context.redactor = Arc::new(crate::tools::Redactor::new(["first\\nsecond".into()]));
    assert!(agent.tool_inventory_display().iter().any(String::is_empty));
    assert!(
        agent
            .tool_inventory()
            .iter()
            .any(|tool| tool.name == "escape_probe" && tool.description == "first\nsecond")
    );
}

#[tokio::test]
async fn plain_terminal_fallback_is_empty_and_observed_shutdown_does_not_invent_resources() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], false);
    let (terminals, policy) = agent.plain_terminals().unwrap();
    assert!(terminals.list().await.unwrap().is_empty());
    assert_eq!(policy.workspace(), root.path());
    let shutdown = agent.shutdown_plain_terminals().await;
    assert!(shutdown.observation_complete);
    assert!(shutdown.remaining.is_empty() && shutdown.failures.is_empty());
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn plain_terminal_metadata_reflects_owned_live_and_exited_processes_then_retires_them() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, observed) = fixture(root.path(), vec![], false);
    agent.tools = ToolRegistry::standard_with_terminal_limits(2, 65536);
    let session = uuid::Uuid::new_v4();
    let run = uuid::Uuid::new_v4();
    let command = "exec sleep 30";
    for name in [Some("owned-operator-terminal"), None] {
        let launch = if name.is_some() {
            "while [ ! -e release-terminal ]; do sleep 0.02; done"
        } else {
            command
        };
        agent
            .operator_tool(
                session,
                run,
                CancellationToken::new(),
                "process",
                serde_json::json!({"action":"start","command":launch,"name":name}),
            )
            .await
            .unwrap();
    }
    let metadata = agent.terminal_metadata();
    assert_eq!(metadata.len(), 2);
    assert!(
        metadata
            .iter()
            .all(|terminal| terminal.state == crate::terminal::TerminalState::Running)
    );
    assert!(
        metadata
            .iter()
            .any(|terminal| terminal.title == "owned-operator-terminal")
    );
    assert!(metadata.iter().any(|terminal| terminal.title == command));
    let (terminals, policy) = agent.plain_terminals().unwrap();
    assert_eq!(policy.workspace(), root.path());
    assert_eq!(terminals.list().await.unwrap().len(), 2);
    let named = metadata
        .iter()
        .find(|terminal| terminal.title == "owned-operator-terminal")
        .unwrap();
    std::fs::write(
        root.path().join("release-terminal"),
        b"owned fixture release",
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if agent.terminal_metadata().iter().any(|terminal| {
                terminal.id == named.id
                    && matches!(
                        terminal.state,
                        crate::terminal::TerminalState::Exited { .. }
                    )
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        agent
            .terminal_metadata()
            .iter()
            .any(|terminal| terminal.title == command
                && terminal.state == crate::terminal::TerminalState::Running)
    );
    let shutdown = agent.shutdown_plain_terminals().await;
    assert!(shutdown.observation_complete, "{shutdown:?}");
    assert!(shutdown.remaining.is_empty() && shutdown.failures.is_empty());
    assert!(agent.terminal_metadata().is_empty());
    assert!(terminals.list().await.unwrap().is_empty());
    assert!(observed.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn scoped_public_run_wrapper_preserves_history_and_canonical_usage_without_coordinator() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![answer("Observed wrapper answer")], false);
    let outcome = agent
        .run_scoped(
            vec![Message::new(Role::User, "Earlier context")],
            "Current prompt".into(),
            CancellationToken::new(),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(outcome.answer, "Observed wrapper answer");
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(outcome.usage.input_tokens, 7);
    assert_eq!(outcome.usage.output_tokens, 3);
    assert!(
        outcome
            .messages
            .iter()
            .any(|message| message.content == "Earlier context")
    );
    assert!(
        outcome
            .messages
            .iter()
            .any(|message| message.content == "Current prompt")
    );
    assert!(
        outcome
            .messages
            .iter()
            .all(|message| message.role != Role::System)
    );
    assert_eq!(observed.requests.lock().unwrap().len(), 1);
}

struct CanonicalOnly {
    id: uuid::Uuid,
    snapshots: Mutex<Vec<Vec<Message>>>,
}
#[async_trait]
impl RunCheckpoint for CanonicalOnly {
    fn run_id(&self) -> uuid::Uuid {
        self.id
    }
    async fn canonical(&self, messages: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        self.snapshots.lock().unwrap().push(messages.to_vec());
        Ok(())
    }
    async fn partial(&self, _: &str) -> Result<(), CheckpointError> {
        Ok(())
    }
}

#[tokio::test]
async fn default_optional_checkpoint_hooks_allow_canonical_only_embedders_to_complete() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![answer("Canonical-only answer")], false);
    let checkpoint = CanonicalOnly {
        id: uuid::Uuid::new_v4(),
        snapshots: Mutex::new(Vec::new()),
    };
    let outcome = agent
        .run_checkpointed(
            vec![],
            "Current prompt".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "loop-model".into(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.stop_reason, StopReason::Completed);
    assert_eq!(outcome.answer, "Canonical-only answer");
    assert!(
        checkpoint
            .snapshots
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .iter()
            .any(|message| message.role == Role::Assistant
                && message.content == "Canonical-only answer")
    );
    assert_eq!(observed.requests.lock().unwrap().len(), 1);
    assert_eq!(outcome.usage.input_tokens, 7);
    assert_eq!(outcome.usage.output_tokens, 3);
    assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn nil_checkpoint_identity_refuses_before_canonical_writes_or_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let (agent, observed) = fixture(root.path(), vec![], false);
    let checkpoint = CanonicalOnly {
        id: uuid::Uuid::nil(),
        snapshots: Mutex::new(Vec::new()),
    };
    let result = agent
        .run_checkpointed(
            vec![],
            "Current prompt".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "loop-model".into(),
        )
        .await;
    assert!(matches!(result, Err(AgentError::Checkpoint(_))));
    assert!(checkpoint.snapshots.lock().unwrap().is_empty());
    assert!(observed.requests.lock().unwrap().is_empty());
    assert_eq!(observed.discoveries.load(Ordering::SeqCst), 0);
    assert_eq!(observed.executions.load(Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[test]
fn recovery_debug_reports_accounting_without_disclosing_canonical_history() {
    let recovery = CanonicalRecovery {
        messages: vec![Message::new(Role::User, "PRIVATE_CANONICAL_HISTORY")],
        usage: Usage {
            input_tokens: 11,
            output_tokens: 5,
        },
    };
    let diagnostic = format!("{recovery:?}");
    assert!(diagnostic.contains("message_count: 1"));
    assert!(diagnostic.contains("input_tokens: 11"));
    assert!(!diagnostic.contains("PRIVATE_CANONICAL_HISTORY"));
}

#[test]
fn context_failure_recovery_retains_full_history_without_public_secret_diagnostics() {
    let messages = vec![Message::new(Role::User, "PRIVATE_CANONICAL_HISTORY")];
    let usage = Usage {
        input_tokens: 11,
        output_tokens: 5,
    };
    for error in [
        AgentError::from(crate::context::ContextError {
            estimated: 1024,
            limit: 32,
        }),
        AgentError::ContextExhausted(None),
    ] {
        let error = error.with_recovery(&messages, &usage);
        let retained = error.recovery().unwrap();
        assert_eq!(retained.messages[0].content, "PRIVATE_CANONICAL_HISTORY");
        assert_eq!(retained.usage.input_tokens, 11);
        assert!(
            !error
                .public_failure_reason()
                .contains("PRIVATE_CANONICAL_HISTORY")
        );
        assert!(!format!("{retained:?}").contains("PRIVATE_CANONICAL_HISTORY"));
    }
}

#[test]
fn public_failure_categories_never_echo_runtime_private_diagnostics() {
    for error in [
        AgentError::Completion("PRIVATE_DIAGNOSTIC".into()),
        AgentError::Policy("PRIVATE_DIAGNOSTIC".into()),
        AgentError::WorkspaceInstructions("PRIVATE_DIAGNOSTIC".into()),
        AgentError::Provider(ProviderError::Request("PRIVATE_DIAGNOSTIC".into())),
        AgentError::Cancelled,
        AgentError::Checkpoint(CheckpointError),
        AgentError::UsageOverflow,
    ] {
        assert!(!error.public_failure_reason().contains("PRIVATE_DIAGNOSTIC"));
        assert!(error.recovery().is_none());
        assert!(!error.is_incomplete());
    }
}

#[test]
fn wrapped_incomplete_failure_preserves_recovery_and_original_public_category() {
    let error = AgentError::Finalization(Box::new(FinalizationFailure {
        source: Box::new(AgentError::Provider(ProviderError::Incomplete)),
        recovery: CanonicalRecovery {
            messages: vec![Message::new(Role::User, "PRIVATE_CANONICAL_HISTORY")],
            usage: Usage::default(),
        },
        readiness: None,
        shutdown: OwnedShutdown {
            remaining: vec![],
            observation_complete: false,
        },
    }));
    assert!(error.is_incomplete());
    assert_eq!(
        error.public_failure_reason(),
        ProviderError::Incomplete.public_failure_reason()
    );
    assert_eq!(
        error.recovery().unwrap().messages[0].content,
        "PRIVATE_CANONICAL_HISTORY"
    );
    assert!(!format!("{error:?}").contains("PRIVATE_CANONICAL_HISTORY"));
}
