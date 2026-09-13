//! Exercise the real agent loop and private durable session checkpoints.
use super::*;
use crate::model::{ModelResponse, Role, ToolCall, ToolDefinition};
use crate::session::{Session, SessionCheckpoint, SessionStore};
use crate::tools::{Tool, ToolError};

struct RecoveringProvider {
    requests: Arc<Mutex<Vec<ModelRequest>>>,
    tool_fragment: bool,
    effect_first: bool,
    interrupt_forever: bool,
    slow_first: bool,
}
#[async_trait]
impl Provider for RecoveringProvider {
    async fn complete(&self, _: ModelRequest) -> Result<ModelResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(&self, request: ModelRequest) -> Result<ProviderStream, ProviderError> {
        let step = {
            let mut requests = self.requests.lock().unwrap();
            let step = requests.len();
            requests.push(request);
            step
        };
        let mut message = Message::new(Role::Assistant, "Recovered answer.");
        if step == 0 && self.effect_first {
            message.content.clear();
            message.tool_calls.push(ToolCall {
                id: "recorded-effect".into(),
                name: "effect".into(),
                arguments: serde_json::json!({}),
            });
        } else if step == usize::from(self.effect_first) || self.interrupt_forever {
            let delta = if self.tool_fragment {
                ProviderDelta::ToolCall {
                    index: 0,
                    id: Some("unfinished-effect".into()),
                    name: Some("effect".into()),
                    arguments: "{".into(),
                }
            } else {
                ProviderDelta::Text("Retained partial segment.".into())
            };
            let slow = self.slow_first;
            return Ok(Box::pin(async_stream::stream! {
                // A healthy request may be longer than the later recovery budget.
                if slow { tokio::time::sleep(Duration::from_millis(80)).await; }
                yield Ok(ProviderStreamEvent::Delta(delta));
                yield Err(ProviderError::StreamInterrupted);
            }));
        }
        Ok(Box::pin(futures_util::stream::iter([Ok(
            ProviderStreamEvent::Completed(Box::new(ModelResponse {
                message,
                usage: Usage::default(),
                service_tier: None,
            })),
        )])))
    }
}
struct Effect(Arc<AtomicUsize>);
#[async_trait]
impl Tool for Effect {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "effect".into(),
            description: "Count an actual effect".into(),
            input_schema: serde_json::json!({"type":"object"}),
            output_schema: None,
            annotations: None,
        }
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> Result<String, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok("Recorded result".into())
    }
}
fn agent(
    root: &std::path::Path,
    tool_fragment: bool,
    effect_first: bool,
    forever: bool,
    slow: bool,
) -> (Agent, Arc<Mutex<Vec<ModelRequest>>>, Arc<AtomicUsize>) {
    let (mut agent, _, _, _, _) = fixture(root, Failure::Timeout);
    let requests = Arc::new(Mutex::new(vec![]));
    let effects = Arc::new(AtomicUsize::new(0));
    agent.provider = Box::new(RecoveringProvider {
        requests: requests.clone(),
        tool_fragment,
        effect_first,
        interrupt_forever: forever,
        slow_first: slow,
    });
    agent.tools.register(Effect(effects.clone()));
    if slow {
        agent.retry.max_elapsed = Duration::from_millis(50);
    }
    (agent, requests, effects)
}
async fn checkpoint(root: &std::path::Path) -> (SessionCheckpoint, SessionStore, uuid::Uuid) {
    let store = SessionStore::new(root.join("sessions"));
    let mut session = Session::new(root.into(), "fixture".into());
    let id = session.id;
    let run = uuid::Uuid::new_v4();
    session.begin_run_summary(run);
    store.save(&mut session).await.unwrap();
    (
        SessionCheckpoint::new(session, Some(store.clone()), run),
        store,
        id,
    )
}
#[tokio::test]
async fn partial_continuation_is_distinct_durable_and_retains_executed_tool_result() {
    for fragments in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (agent, requests, effects) = agent(root.path(), fragments, true, false, false);
        let (checkpoint, store, id) = checkpoint(root.path()).await;
        let result = agent
            .run_checkpointed(
                vec![],
                "Perform one effect then answer".into(),
                CancellationToken::new(),
                None,
                &checkpoint,
                "fixture".into(),
            )
            .await
            .unwrap();
        assert_eq!(result.answer, "Recovered answer.");
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        let loaded = store.load(id).await.unwrap();
        let segments: Vec<_> = loaded
            .messages
            .iter()
            .filter(|m| m.interrupted_attempt.is_some())
            .collect();
        assert_eq!(segments.len(), 1);
        assert_eq!(
            segments[0].content,
            if fragments {
                ""
            } else {
                "Retained partial segment."
            }
        );
        assert!(segments[0].tool_calls.is_empty());
        assert!(loaded.messages.iter().all(|m| m.role != Role::System));
        let attempts = &loaded.run_summaries[0].provider_attempts;
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts[1].decision, RetryDecision::ContinuationScheduled);
        assert_eq!(attempts[2].decision, RetryDecision::Completed);
        assert_eq!(attempts[2].retry.recovery_of, Some(attempts[1].attempt_id));
        assert_ne!(attempts[1].request_id, attempts[2].request_id);
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[2].messages.iter().any(|m| m.role == Role::Tool
            && m.tool_call_id.as_deref() == Some("recorded-effect")
            && m.content.contains("Recorded result")));
        assert!(!requests[2].messages.iter().any(|m| {
            m.tool_calls
                .iter()
                .any(|call| call.id == "unfinished-effect")
        }));
        assert!(
            !requests[2]
                .messages
                .iter()
                .any(|m| m.role == Role::Assistant
                    && m.content.is_empty()
                    && m.tool_calls.is_empty())
        );
        assert!(
            requests[2].messages[0]
                .content
                .contains("interrupted assistant")
        );
    }
}
#[tokio::test]
async fn repeated_partial_responses_share_one_bounded_budget() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, _) = agent(root.path(), false, false, true, false);
    let (checkpoint, store, id) = checkpoint(root.path()).await;
    assert!(
        agent
            .run_checkpointed(
                vec![],
                "Answer".into(),
                CancellationToken::new(),
                None,
                &checkpoint,
                "fixture".into()
            )
            .await
            .is_err()
    );
    assert_eq!(requests.lock().unwrap().len(), 3);
    let loaded = store.load(id).await.unwrap();
    let attempts = &loaded.run_summaries[0].provider_attempts;
    assert_eq!(
        attempts.last().unwrap().decision,
        RetryDecision::AttemptsExhausted
    );
    assert_eq!(
        loaded
            .messages
            .iter()
            .filter(|m| m.interrupted_attempt.is_some())
            .count(),
        2
    );
    assert_eq!(
        loaded.run_summaries[0].partial_output,
        "Retained partial segment."
    );
    assert!(
        attempts
            .windows(2)
            .all(|pair| pair[1].attempt == pair[0].attempt + 1
                && pair[1].retry.recovery_deadline_at_ms == pair[0].retry.recovery_deadline_at_ms)
    );
}
#[tokio::test]
async fn healthy_long_first_response_does_not_consume_reconnection_window() {
    let root = tempfile::tempdir().unwrap();
    let (agent, requests, _) = agent(root.path(), false, false, false, true);
    let (checkpoint, _, _) = checkpoint(root.path()).await;
    let result = agent
        .run_checkpointed(
            vec![],
            "Answer".into(),
            CancellationToken::new(),
            None,
            &checkpoint,
            "fixture".into(),
        )
        .await
        .unwrap();
    assert_eq!(result.answer, "Recovered answer.");
    assert_eq!(requests.lock().unwrap().len(), 2);
}

struct FailingContinuation {
    inner: SessionCheckpoint,
    cancel: Option<CancellationToken>,
}
#[async_trait]
impl RunCheckpoint for FailingContinuation {
    fn run_id(&self) -> uuid::Uuid {
        self.inner.run_id()
    }
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        if messages.iter().any(|m| m.interrupted_attempt.is_some()) {
            return Err(CheckpointError);
        }
        self.inner.canonical(messages, usage).await
    }
    async fn provider_attempt(&self, attempt: &ProviderAttempt) -> Result<(), CheckpointError> {
        self.inner.provider_attempt(attempt).await?;
        if attempt.decision == RetryDecision::ContinuationScheduled
            && let Some(cancel) = &self.cancel
        {
            cancel.cancel();
        }
        Ok(())
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        self.inner.partial(text).await
    }
}
#[tokio::test]
async fn failed_segment_checkpoint_or_cancelled_backoff_never_dispatches_continuation() {
    for cancel_wait in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (agent, requests, _) = agent(root.path(), false, false, false, false);
        let (inner, store, id) = checkpoint(root.path()).await;
        let token = CancellationToken::new();
        let checkpoint = FailingContinuation {
            inner,
            cancel: cancel_wait.then(|| token.clone()),
        };
        let result = agent
            .run_checkpointed(
                vec![],
                "Answer".into(),
                token,
                None,
                &checkpoint,
                "fixture".into(),
            )
            .await;
        assert!(result.is_err());
        assert_eq!(requests.lock().unwrap().len(), 1);
        let loaded = store.load(id).await.unwrap();
        assert_eq!(
            loaded.run_summaries[0].partial_output,
            "Retained partial segment."
        );
        if cancel_wait {
            assert_eq!(
                loaded.run_summaries[0].provider_attempts[0].decision,
                RetryDecision::Cancelled
            );
        }
    }
}
