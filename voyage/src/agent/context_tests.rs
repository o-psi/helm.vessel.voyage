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
    cancel_after_compaction: Option<CancellationToken>,
}
#[async_trait]
impl RunCheckpoint for Checkpoint {
    fn run_id(&self) -> uuid::Uuid {
        self.id
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
    assert!(crate::context::estimate(&requests[2]) + 128 < crate::context::estimate(&requests[1]));
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
        assert!(crate::context::estimate(&pair[1]) + 128 < crate::context::estimate(&pair[0]));
    }
    assert!(checkpoint.canonical.lock().unwrap()[1].content.len() > 60_000);
}
