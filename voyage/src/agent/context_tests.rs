//! Synthetic provider execution; no paid calls or external credentials.
use super::*;
use crate::{
    model::{ModelResponse, Role, ToolCall, ToolDefinition},
    tools::{InteractionMode, Tool, ToolError, UnattendedApprover},
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

struct FixtureProvider {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    step: AtomicUsize,
    reject_forever: bool,
}
#[async_trait]
impl Provider for FixtureProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let step = self.step.fetch_add(1, Ordering::SeqCst);
        if self.reject_forever || step == 1 {
            return Err(ProviderError::ContextLength);
        }
        let mut message = Message::new(Role::Assistant, "Finished without replay.");
        if step == 0 {
            message.content.clear();
            message.tool_calls.push(ToolCall {
                id: "durable-effect-64".into(),
                name: "fixture_effect".into(),
                arguments: serde_json::json!({}),
            });
        }
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
struct Effect(Arc<AtomicUsize>);
#[async_trait]
impl Tool for Effect {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "fixture_effect".into(),
            description: "Synthetic effect counter".into(),
            input_schema: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> Result<String, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(format!("BEGIN {} END", "result".repeat(8_000)))
    }
}
struct Checkpoint {
    id: uuid::Uuid,
    canonical: Mutex<Vec<Message>>,
    working: Mutex<crate::context::WorkingContext>,
    fail_compaction: bool,
    observations: Mutex<Vec<voyage_protocol::context_accounting::ContextObservation>>,
    cancel_after_compaction: Option<CancellationToken>,
}
#[async_trait]
impl RunCheckpoint for Checkpoint {
    fn run_id(&self) -> uuid::Uuid {
        self.id
    }
    async fn context_observation(
        &self,
        observation: &voyage_protocol::context_accounting::ContextObservation,
    ) -> Result<(), CheckpointError> {
        assert_eq!(observation.execution_id, self.id);
        assert_eq!(
            observation.projection_generation,
            self.working.lock().unwrap().generation
        );
        self.observations.lock().unwrap().push(observation.clone());
        Ok(())
    }
    async fn canonical(&self, messages: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        *self.canonical.lock().unwrap() = messages.to_vec();
        Ok(())
    }
    async fn working_context(&self) -> Result<crate::context::WorkingContext, CheckpointError> {
        Ok(self.working.lock().unwrap().clone())
    }
    async fn save_working_context(
        &self,
        working: &crate::context::WorkingContext,
    ) -> Result<(), CheckpointError> {
        if self.fail_compaction {
            return Err(CheckpointError);
        }
        working
            .validate(&self.canonical.lock().unwrap())
            .map_err(|_| CheckpointError)?;
        *self.working.lock().unwrap() = working.clone();
        if let Some(cancel) = &self.cancel_after_compaction {
            cancel.cancel();
        }
        Ok(())
    }
    async fn partial(&self, _: &str) -> Result<(), CheckpointError> {
        Ok(())
    }
}

fn fixture(
    root: &std::path::Path,
    forever: bool,
) -> (
    Agent,
    Arc<Mutex<Vec<ModelRequest>>>,
    Arc<AtomicUsize>,
    Checkpoint,
) {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let effects = Arc::new(AtomicUsize::new(0));
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
        timeout: Duration::from_secs(3),
        max_output_bytes: 200_000,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(crate::tools::Redactor::default()),
    };
    let mut tools = ToolRegistry::default();
    tools.register(Effect(effects.clone()));
    let agent = Agent::new(
        Box::new(FixtureProvider {
            requests: requests.clone(),
            step: AtomicUsize::new(0),
            reject_forever: forever,
        }),
        tools,
        context,
        Arc::new(SilentSink),
        "fixture".into(),
        "Preserve task and tool identities.".into(),
        0,
        None,
    );
    let checkpoint = Checkpoint {
        id: uuid::Uuid::new_v4(),
        canonical: Mutex::new(vec![]),
        working: Mutex::new(Default::default()),
        fail_compaction: false,
        observations: Mutex::new(vec![]),
        cancel_after_compaction: None,
    };
    (agent, requests, effects, checkpoint)
}

#[tokio::test]
async fn actual_rejection_reduces_followup_without_executing_tool_twice() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, effects, checkpoint) = fixture(root.path(), false);
    let outcome = agent
        .run_checkpointed(
            vec![],
            "Do one effect".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.answer, "Finished without replay.");
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        crate::context::payload_bytes(&requests[2]) + 128
            < crate::context::payload_bytes(&requests[1])
    );
    let result = outcome
        .messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert!(result.content.len() > 45_000);
    assert_eq!(result.tool_call_id.as_deref(), Some("durable-effect-64"));
    assert_eq!(
        requests[2]
            .messages
            .iter()
            .find(|m| m.role == Role::Tool)
            .unwrap()
            .tool_call_id,
        result.tool_call_id
    );
    assert!(checkpoint.working.lock().unwrap().generation > 0);
}

#[tokio::test]
async fn irreducible_input_is_actionable_and_not_retried_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, effects, checkpoint) = fixture(root.path(), true);
    let error = agent
        .run_checkpointed(
            vec![],
            "original task ".repeat(10_000),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::ContextExhausted(_)));
    assert!(error.recovery().unwrap().messages[0].content.len() > 100_000);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_compaction_checkpoint_preserves_effect_evidence_and_never_dispatches_retry() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, effects, mut checkpoint) = fixture(root.path(), false);
    checkpoint.fail_compaction = true;
    let error = agent
        .run_checkpointed(
            vec![],
            "Do one effect".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::Checkpoint(_)));
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(
        checkpoint
            .canonical
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.role == Role::Tool && m.content.len() > 45_000)
    );
    assert_eq!(checkpoint.working.lock().unwrap().generation, 0);
}

#[tokio::test]
async fn cancellation_after_durable_reduction_stops_before_next_provider_request() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, effects, mut checkpoint) = fixture(root.path(), false);
    let cancel = CancellationToken::new();
    checkpoint.cancel_after_compaction = Some(cancel.clone());
    let error = agent
        .run_checkpointed(
            vec![],
            "Do one effect".into(),
            cancel,
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(checkpoint.working.lock().unwrap().generation > 0);
}

struct PartialProvider(Arc<AtomicUsize>);
#[async_trait]
impl Provider for PartialProvider {
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(
        &self,
        _: ModelRequest,
    ) -> Result<crate::provider::ProviderStream, ProviderError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(crate::provider::ProviderStreamEvent::Delta(
                crate::provider::ProviderDelta::ToolCall {
                    index: 0,
                    id: Some("uncertain-call".into()),
                    name: Some("fixture_effect".into()),
                    arguments: "{".into(),
                },
            )),
            Err(ProviderError::ContextLength),
        ])))
    }
}

#[tokio::test]
async fn context_error_after_tool_delta_is_incomplete_and_never_replayed() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, _, effects, checkpoint) = fixture(root.path(), false);
    let requests = Arc::new(AtomicUsize::new(0));
    agent.provider = Box::new(PartialProvider(requests.clone()));
    let history = vec![
        Message::new(Role::User, "earlier task"),
        Message::new(Role::Assistant, "old text ".repeat(2000)),
    ];
    let error = agent
        .run_checkpointed(
            history,
            "continue".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Provider(ProviderError::Incomplete)
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert_eq!(checkpoint.working.lock().unwrap().generation, 0);
}

#[tokio::test]
async fn repeated_provider_rejection_is_bounded_and_each_dispatch_shrinks() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, _, checkpoint) = fixture(root.path(), true);
    let history = vec![
        Message::new(Role::User, "earlier task"),
        Message::new(Role::Assistant, "old text ".repeat(8000)),
    ];
    let error = agent
        .run_checkpointed(
            history,
            "continue".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::ContextExhausted(_)));
    let requests = requests.lock().unwrap();
    assert!(requests.len() > 1 && requests.len() <= 5);
    for pair in requests.windows(2) {
        assert!(
            crate::context::payload_bytes(&pair[1]) + 128 < crate::context::payload_bytes(&pair[0])
        );
    }
    assert!(checkpoint.canonical.lock().unwrap()[1].content.len() > 60_000);
}

#[tokio::test]
async fn explicit_context_limit_without_counting_never_uses_bytes_or_drops_constraints() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, effects, checkpoint) = fixture(root.path(), false);
    let outcome = agent
        .with_context_window(1)
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(outcome.messages[0].content, "Keep my exact task");
    assert_eq!(outcome.answer, "Finished without replay.");
    assert_eq!(requests.lock().unwrap().len(), 3);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

struct CountedProvider {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    rejections: Arc<AtomicUsize>,
}
fn full_fixture_result(request: &ModelRequest) -> bool {
    request
        .messages
        .iter()
        .any(|m| m.role == Role::Tool && m.content.starts_with("BEGIN "))
}
#[async_trait]
impl Provider for CountedProvider {
    fn context_window(&self, _: &str) -> Option<usize> {
        Some(100)
    }
    async fn input_tokens(
        &self,
        request: &ModelRequest,
    ) -> Result<voyage_protocol::context_accounting::RequestTokenCount, ProviderError> {
        use sha2::{Digest, Sha256};
        use voyage_protocol::context_accounting::*;
        // A synthetic model's fixture contract, also enforced by complete().
        // These API-like counts are deliberately independent of UTF-8 size.
        let tokens = if full_fixture_result(request) {
            128
        } else {
            32
        };
        Ok(RequestTokenCount {
            scope: ContextScope {
                model: request.model.clone(),
                transport: "fixture".into(),
                endpoint_fingerprint: None,
                account: None,
            },
            observed_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            input_tokens: Some(tokens),
            precision: CountPrecision::ProviderExact,
            method: "fixture_token_counter".into(),
            complete: true,
            input_fingerprint: Some(format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(request).unwrap())
            )),
            limitations: vec![],
        })
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        assert!(
            request
                .messages
                .iter()
                .any(|m| m.role == Role::User && m.content == "Keep my exact task")
        );
        let full = full_fixture_result(&request);
        let has_result = request.messages.iter().any(|m| m.role == Role::Tool);
        self.requests.lock().unwrap().push(request);
        if full {
            self.rejections.fetch_add(1, Ordering::SeqCst);
            return Err(ProviderError::ContextLength);
        }
        let mut message = Message::new(Role::Assistant, "Finished with the exact constraint.");
        if !has_result {
            message.content.clear();
            message.tool_calls.push(ToolCall {
                id: "durable-effect-64".into(),
                name: "fixture_effect".into(),
                arguments: serde_json::json!({}),
            });
        }
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
#[tokio::test]
async fn trustworthy_pressure_prepares_before_predictable_rejection_without_effect_replay() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, effects, checkpoint) = fixture(root.path(), false);
    let rejections = Arc::new(AtomicUsize::new(0));
    agent.provider = Box::new(CountedProvider {
        requests: requests.clone(),
        rejections: rejections.clone(),
    });
    let outcome = agent
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(rejections.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(outcome.messages[0].content, "Keep my exact task");
    let result = outcome
        .messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    assert_eq!(result.tool_call_id.as_deref(), Some("durable-effect-64"));
    assert!(result.content.starts_with("BEGIN ") && result.content.ends_with(" END"));
    assert!(checkpoint.working.lock().unwrap().generation > 0);
    let observations = checkpoint.observations.lock().unwrap();
    let final_observation = observations.last().unwrap();
    let preparation = final_observation.preparation.as_ref().unwrap();
    assert!(
        preparation.before_count.reliable_input_tokens().unwrap()
            > final_observation.count.reliable_input_tokens().unwrap()
    );
    assert!(preparation.before_generation < final_observation.projection_generation);
    assert!(preparation.reduced_messages > 0 && preparation.steps <= 4);
    assert_eq!(
        final_observation.pressure,
        voyage_protocol::context_accounting::ContextPressure::WithinBudget
    );
}
#[tokio::test]
async fn trustworthy_explicit_limit_refuses_irreducible_input_with_canonical_recovery() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, effects, checkpoint) = fixture(root.path(), false);
    agent.provider = Box::new(CountedProvider {
        requests: requests.clone(),
        rejections: Arc::new(AtomicUsize::new(0)),
    });
    let error = agent
        .with_context_window(1)
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::Context(_)));
    assert_eq!(
        error.recovery().unwrap().messages[0].content,
        "Keep my exact task"
    );
    assert!(requests.lock().unwrap().is_empty());
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

struct SteeringDuringCount {
    sender: SteeringSender,
    counts: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
}
#[async_trait]
impl Provider for SteeringDuringCount {
    async fn input_tokens(
        &self,
        request: &ModelRequest,
    ) -> Result<voyage_protocol::context_accounting::RequestTokenCount, ProviderError> {
        if self.counts.fetch_add(1, Ordering::SeqCst) == 0 {
            assert!(
                !request
                    .messages
                    .iter()
                    .any(|m| m.content == "Keep the unresolved obligation")
            );
            self.sender
                .try_send("Keep the unresolved obligation".into())
                .unwrap();
            tokio::task::yield_now().await;
        } else {
            assert!(
                request
                    .messages
                    .iter()
                    .any(|m| m.content == "Keep the unresolved obligation")
            );
        }
        Ok(crate::provider::context_accounting::unknown(
            request,
            "fixture",
            None,
            "fixture_unknown",
        ))
    }
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        assert!(
            request
                .messages
                .iter()
                .any(|m| m.content == "Keep the unresolved obligation")
        );
        self.requests.lock().unwrap().push(request);
        Ok(ModelResponse {
            message: Message::new(Role::Assistant, "Guidance applied before dispatch"),
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
#[tokio::test]
async fn steering_during_count_invalidates_the_prepared_request_before_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, _, checkpoint) = fixture(root.path(), false);
    let (sender, receiver) = steering_channel(2);
    let counts = Arc::new(AtomicUsize::new(0));
    agent.provider = Box::new(SteeringDuringCount {
        sender,
        counts: counts.clone(),
        requests: requests.clone(),
    });
    let outcome = agent
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            CancellationToken::new(),
            Some(receiver),
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(counts.load(Ordering::SeqCst), 2);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(outcome.messages.iter().any(|m| {
        m.steering
            .as_ref()
            .is_some_and(|s| matches!(s.status, crate::model::SteeringStatus::Applied))
    }));
    assert_eq!(checkpoint.observations.lock().unwrap().len(), 1);
}

struct ContextToolProvider {
    step: AtomicUsize,
    requests: Arc<Mutex<Vec<ModelRequest>>>,
}
#[async_trait]
impl Provider for ContextToolProvider {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, ProviderError> {
        assert!(request.tools.iter().any(|t| t.name == "context_status"));
        assert!(request.tools.iter().any(|t| t.name == "compact_context"));
        self.requests.lock().unwrap().push(request.clone());
        let step = self.step.fetch_add(1, Ordering::SeqCst);
        let mut message = Message::new(Role::Assistant, "");
        let (id, name, args) = match step {
            0 => ("effect", "fixture_effect", serde_json::json!({})),
            1 => (
                "compact",
                "compact_context",
                serde_json::json!({"retain_recent":0,"carry_forward":"Unresolved obligation: verify before publishing"}),
            ),
            2 => {
                let result = request
                    .messages
                    .iter()
                    .find(|m| m.tool_call_id.as_deref() == Some("compact"))
                    .unwrap();
                let receipt: serde_json::Value = serde_json::from_str(&result.content).unwrap();
                assert_eq!(receipt["state"], "applied");
                assert!(receipt["generation"].as_u64().unwrap() > 0);
                assert!(receipt["before_input_tokens"].is_null());
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|m| m.role == Role::User && m.content == "Keep my exact task")
                );
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|m| m.content.contains("Model-authored carry-forward data")
                            && m.content.contains("Unresolved obligation"))
                );
                (
                    "history",
                    "context_status",
                    serde_json::json!({"action":"read_history","message_index":2,"offset":12000,"limit":128}),
                )
            }
            3 => {
                let read = request
                    .messages
                    .iter()
                    .find(|m| m.tool_call_id.as_deref() == Some("history"))
                    .unwrap();
                let value: serde_json::Value = serde_json::from_str(&read.content).unwrap();
                assert!(value["text"].as_str().unwrap().contains("resultresult"));
                (
                    "noop",
                    "compact_context",
                    serde_json::json!({"retain_recent":1024}),
                )
            }
            4 => {
                let noop = request
                    .messages
                    .iter()
                    .find(|m| m.tool_call_id.as_deref() == Some("noop"))
                    .unwrap();
                let value: serde_json::Value = serde_json::from_str(&noop.content).unwrap();
                assert_eq!(value["state"], "no_op");
                (
                    "status",
                    "context_status",
                    serde_json::json!({"action":"status"}),
                )
            }
            5 => {
                let status = request
                    .messages
                    .iter()
                    .find(|m| m.tool_call_id.as_deref() == Some("status"))
                    .unwrap();
                let value: serde_json::Value = serde_json::from_str(&status.content).unwrap();
                assert!(value["current_remaining_tokens"].is_null());
                assert_eq!(value["model_notes"], 1);
                assert!(value["last_prepared_request"]["input_tokens"].is_null());
                message.content =
                    "Finished with retrieved exact evidence and retained obligation".into();
                return Ok(ModelResponse {
                    message,
                    usage: Usage::default(),
                    service_tier: None,
                });
            }
            _ => panic!("unexpected context loop"),
        };
        message.tool_calls.push(ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: args,
        });
        Ok(ModelResponse {
            message,
            usage: Usage::default(),
            service_tier: None,
        })
    }
}
#[tokio::test]
async fn model_context_tools_persist_receipts_and_retrieve_canonical_evidence_without_replay() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, effects, checkpoint) = fixture(root.path(), false);
    agent.provider = Box::new(ContextToolProvider {
        step: AtomicUsize::new(0),
        requests: requests.clone(),
    });
    let outcome = agent
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(requests.lock().unwrap().len(), 6);
    assert!(
        outcome
            .messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some("effect"))
            .unwrap()
            .content
            .len()
            > 45000
    );
    let saved = checkpoint.working.lock().unwrap().clone();
    assert_eq!(saved.model_note_count(), 1);
    saved.validate(&outcome.messages).unwrap();
}

#[tokio::test]
async fn model_compaction_checkpoint_failure_never_claims_applied_or_dispatches_continuation() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, effects, mut checkpoint) = fixture(root.path(), false);
    checkpoint.fail_compaction = true;
    agent.provider = Box::new(ContextToolProvider {
        step: AtomicUsize::new(0),
        requests: requests.clone(),
    });
    let error = agent
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::Checkpoint(_)));
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(checkpoint.working.lock().unwrap().generation, 0);
    assert!(
        !checkpoint
            .canonical
            .lock()
            .unwrap()
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("compact"))
    );
}
#[tokio::test]
async fn cancellation_after_context_persistence_retains_receipt_without_inference_or_effect_replay()
{
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, effects, mut checkpoint) = fixture(root.path(), false);
    let cancel = CancellationToken::new();
    checkpoint.cancel_after_compaction = Some(cancel.clone());
    agent.provider = Box::new(ContextToolProvider {
        step: AtomicUsize::new(0),
        requests: requests.clone(),
    });
    let error = agent
        .run_checkpointed(
            vec![],
            "Keep my exact task".into(),
            cancel,
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, AgentError::Cancelled));
    assert_eq!(requests.lock().unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let working = checkpoint.working.lock().unwrap().clone();
    let canonical = checkpoint.canonical.lock().unwrap().clone();
    let receipt = working
        .model_receipt(&canonical, 3, "compact")
        .unwrap()
        .unwrap();
    assert_eq!(receipt["state"], "applied");
    let serialized = serde_json::to_value(&working).unwrap();
    let restored: crate::context::WorkingContext = serde_json::from_value(serialized).unwrap();
    let projected = restored.project(&canonical).unwrap();
    let receipt_text = receipt.to_string();
    assert!(
        projected
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("compact") && m.content == receipt_text)
    );
    assert!(
        !canonical
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("compact"))
    );
}

#[tokio::test]
async fn stale_or_switched_catalogue_cannot_supply_an_enabled_window() {
    use voyage_protocol::context_accounting::*;
    let root = tempfile::tempdir().unwrap();
    let (mut agent, requests, _, _) = fixture(root.path(), false);
    agent.provider = Box::new(CountedProvider {
        requests,
        rejections: Arc::new(AtomicUsize::new(0)),
    });
    let request = ModelRequest {
        model: "fixture".into(),
        messages: vec![Message::new(Role::User, "Keep my exact task")],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: Some(10),
    };
    let count = agent.provider.input_tokens(&request).await.unwrap();
    let now = count.observed_at_ms;
    let capacity = ModelContextCapacity {
        scope: count.scope.clone(),
        observed_at_ms: now,
        source: "test_catalogue".into(),
        default_window_tokens: Some(1000),
        maximum_selectable_window_tokens: Some(10000),
        enabled_window_tokens: Some(1000),
        default_output_tokens: Some(10),
        maximum_output_tokens: None,
    };
    for case in ["fresh", "stale", "future", "model", "transport", "endpoint"] {
        let mut capacity = capacity.clone();
        match case {
            "stale" => capacity.observed_at_ms = now.saturating_sub(300_001),
            "future" => capacity.observed_at_ms = now + 60_000,
            "model" => capacity.scope.model = "other".into(),
            "transport" => capacity.scope.transport = "other".into(),
            "endpoint" => capacity.scope.endpoint_fingerprint = Some("b".repeat(64)),
            _ => {}
        }
        let mut model = crate::provider::ModelInfo::minimal("fixture");
        model.context_capacity = Some(capacity);
        *agent.model_cache.lock().await = Some((std::time::Instant::now(), vec![model]));
        let observed = agent
            .observe_request_context(&request, uuid::Uuid::new_v4(), 0, &CancellationToken::new())
            .await
            .unwrap();
        let observed_capacity = observed.capacity.unwrap();
        assert_eq!(
            observed_capacity.source,
            if case == "fresh" {
                "test_catalogue"
            } else {
                "provider_adapter_effective_limit"
            },
            "{case}"
        );
        assert_eq!(
            observed_capacity.enabled_window_tokens,
            Some(if case == "fresh" { 1000 } else { 100 }),
            "{case}"
        );
    }
}
