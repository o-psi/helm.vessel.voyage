//! Synthetic adverse retries: no provider credentials, billing or external effects.
use super::*;
use crate::provider::{ProviderDelta, ProviderStream, ProviderStreamEvent};
use crate::tools::{InteractionMode, UnattendedApprover};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use voyage_protocol::provider_attempt::{AttemptPhase, ProviderAttempt, RetryDecision};

#[derive(Clone, Copy)]
enum Failure {
    RefreshedAuthentication,
    Timeout,
    SilentStart,
    Idle,
    Activity,
    Text,
    Tool,
    Reasoning,
    Request,
    LongWait,
    Eof,
    ContextText,
    TransportText,
}
struct ProviderFixture {
    failure: Failure,
    calls: Arc<AtomicUsize>,
}
#[async_trait]
impl Provider for ProviderFixture {
    async fn complete(
        &self,
        _: ModelRequest,
    ) -> Result<crate::model::ModelResponse, ProviderError> {
        unreachable!()
    }
    async fn stream(&self, _: ModelRequest) -> Result<ProviderStream, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.failure {
            Failure::RefreshedAuthentication => {
                Err(ProviderError::AuthenticationRefreshed.with_http_status(401))
            }
            Failure::SilentStart => futures_util::future::pending().await,
            Failure::Idle => Ok(Box::pin(futures_util::stream::pending())),
            Failure::Activity => Ok(Box::pin(futures_util::stream::repeat_with(|| {
                Ok(ProviderStreamEvent::Activity)
            }))),
            Failure::Timeout => Err(ProviderError::Timeout("SECRET_DIAGNOSTIC".into())),
            Failure::Request => Err(ProviderError::Request("SECRET_DIAGNOSTIC".into())),
            Failure::LongWait => Err(ProviderError::RateLimit {
                message: "SECRET_DIAGNOSTIC".into(),
                retry_after: Some(Duration::from_secs(60)),
            }),
            Failure::Eof => Ok(Box::pin(futures_util::stream::empty())),
            Failure::Text
            | Failure::Tool
            | Failure::Reasoning
            | Failure::ContextText
            | Failure::TransportText => {
                let delta = if matches!(self.failure, Failure::Reasoning) {
                    ProviderDelta::Reasoning {
                        index: 2,
                        kind: voyage_protocol::reasoning_preview::ReasoningKind::Summary,
                        text: "disclosure".into(),
                    }
                } else if matches!(self.failure, Failure::Tool) {
                    ProviderDelta::ToolCall {
                        index: 0,
                        id: Some("effect".into()),
                        name: Some("shell".into()),
                        arguments: "{".into(),
                    }
                } else {
                    ProviderDelta::Text("partial text".into())
                };
                let error = if matches!(self.failure, Failure::ContextText) {
                    ProviderError::ContextLength
                } else if matches!(self.failure, Failure::TransportText) {
                    ProviderError::TransportTimeout
                } else {
                    ProviderError::Timeout("SECRET_DIAGNOSTIC".into())
                };
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(ProviderStreamEvent::Delta(delta)),
                    Err(error),
                ])))
            }
        }
    }
}
struct Checkpoint {
    previews: Mutex<Vec<voyage_protocol::tool_preview::ToolPreview>>,
    reasoning: Mutex<Vec<voyage_protocol::reasoning_preview::ReasoningPreview>>,
    attempts: Mutex<Vec<ProviderAttempt>>,
    cancel: Option<CancellationToken>,
    revoke: Option<Arc<AtomicBool>>,
    fail_intent: bool,
}
#[async_trait]
impl RunCheckpoint for Checkpoint {
    fn run_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
    async fn canonical(&self, _: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn reasoning_previews(
        &self,
        previews: &[voyage_protocol::reasoning_preview::ReasoningPreview],
    ) -> Result<(), CheckpointError> {
        *self.reasoning.lock().unwrap() = previews.to_vec();
        Ok(())
    }
    async fn tool_previews(
        &self,
        previews: &[voyage_protocol::tool_preview::ToolPreview],
    ) -> Result<(), CheckpointError> {
        *self.previews.lock().unwrap() = previews.to_vec();
        Ok(())
    }
    async fn partial(&self, _: &str) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn provider_attempt(&self, record: &ProviderAttempt) -> Result<(), CheckpointError> {
        if self.fail_intent {
            return Err(CheckpointError);
        }
        let mut records = self.attempts.lock().unwrap();
        if let Some(existing) = records
            .iter_mut()
            .find(|r| r.attempt_id == record.attempt_id)
        {
            *existing = record.clone();
        } else {
            records.push(record.clone());
        }
        if record.decision == RetryDecision::RetryScheduled {
            if let Some(cancel) = &self.cancel {
                cancel.cancel();
            }
            if let Some(revoke) = &self.revoke {
                revoke.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Authority(Arc<AtomicBool>);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.0.load(Ordering::SeqCst), "revoked");
        Ok(())
    }
}
struct Sink(Mutex<Vec<String>>);
#[async_trait]
impl EventSink for Sink {
    async fn emit(&self, event: AgentEvent) {
        if let AgentEvent::ProviderRetry { error, .. } = event {
            self.0.lock().unwrap().push(error);
        }
    }
}
fn fixture(
    root: &std::path::Path,
    failure: Failure,
) -> (
    Agent,
    Arc<AtomicUsize>,
    Checkpoint,
    Arc<Sink>,
    Arc<AtomicBool>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let revoked = Arc::new(AtomicBool::new(false));
    let config = crate::Config {
        access: Some(AccessMode::Unrestricted),
        ..Default::default()
    };
    let policy = crate::policy::Policy::new(&config, root.into())
        .unwrap()
        .with_execution_authority(Arc::new(Authority(revoked.clone())));
    let context = ToolContext {
        tool_call_id: None,
        artifact_scope: None,
        github: None,
        completion: None,
        policy: Arc::new(policy),
        approver: Arc::new(UnattendedApprover { allow: false }),
        timeout: Duration::from_secs(3),
        max_output_bytes: 200_000,
        environment: Default::default(),
        cancellation: CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Unattended,
        redactor: Arc::new(crate::tools::Redactor::default()),
    };
    let sink = Arc::new(Sink(Mutex::new(vec![])));
    let agent = Agent::new(
        Box::new(ProviderFixture {
            failure,
            calls: calls.clone(),
        }),
        ToolRegistry::default(),
        context,
        sink.clone(),
        "fixture".into(),
        "test".into(),
        0,
        None,
    )
    .with_retry_policy(RetryPolicy {
        max_attempts: 3,
        initial_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(4),
        max_elapsed: Duration::from_secs(10),
        ..Default::default()
    });
    let checkpoint = Checkpoint {
        attempts: Mutex::new(vec![]),
        previews: Mutex::new(vec![]),
        reasoning: Mutex::new(vec![]),
        cancel: None,
        revoke: None,
        fail_intent: false,
    };
    (agent, calls, checkpoint, sink, revoked)
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "fixture".into(),
        messages: vec![],
        tools: vec![],
        temperature: None,
        reasoning_effort: None,
        service_tier: None,
        max_tokens: None,
    }
}
async fn execute(
    agent: &Agent,
    checkpoint: &dyn RunCheckpoint,
    cancel: &CancellationToken,
) -> Result<provider_attempts::RequestOutcome, AgentError> {
    agent
        .provider_request_attempts(
            request(),
            cancel,
            Some(checkpoint),
            &mut String::new(),
            None,
            &mut provider_attempts::RecoveryState::new(&agent.retry),
        )
        .await
}

#[tokio::test]
async fn refreshed_authentication_uses_recorded_bounded_retry_admission() {
    let root = tempfile::tempdir().unwrap();
    let (agent, calls, checkpoint, _, _) = fixture(root.path(), Failure::RefreshedAuthentication);
    assert!(
        execute(&agent, &checkpoint, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let records = checkpoint.attempts.lock().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].decision, RetryDecision::RetryScheduled);
    assert_eq!(records[2].decision, RetryDecision::AttemptsExhausted);
    assert!(
        records
            .iter()
            .all(|r| r.http_status == Some(401) && r.category.as_deref() == Some("authentication"))
    );
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
}

#[tokio::test]
async fn timeout_exhaustion_is_bounded_attributed_and_secret_safe() {
    let root = tempfile::tempdir().unwrap();
    let (agent, calls, checkpoint, sink, _) = fixture(root.path(), Failure::Timeout);
    assert!(
        execute(&agent, &checkpoint, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let records = checkpoint.attempts.lock().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[2].decision, RetryDecision::AttemptsExhausted);
    assert!(
        records
            .iter()
            .all(|r| r.request_id == records[0].request_id && r.phase == AttemptPhase::Dispatch)
    );
    assert_ne!(records[0].attempt_id, records[1].attempt_id);
    assert!(!serde_json::to_string(&*records).unwrap().contains("SECRET"));
    assert!(!format!("{:?}", sink.0.lock().unwrap()).contains("SECRET"));
}

#[tokio::test]
async fn partial_transient_failure_requests_checkpointed_continuation_but_context_rejection_stops()
{
    for failure in [
        Failure::Text,
        Failure::Tool,
        Failure::ContextText,
        Failure::TransportText,
    ] {
        let root = tempfile::tempdir().unwrap();
        let (agent, calls, checkpoint, _, _) = fixture(root.path(), failure);
        let result = execute(&agent, &checkpoint, &CancellationToken::new()).await;
        if matches!(failure, Failure::ContextText) {
            assert!(matches!(result, Err(ref error) if error.is_incomplete()));
        } else {
            assert!(matches!(
                result,
                Ok(provider_attempts::RequestOutcome::Interrupted(_))
            ));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let previews = checkpoint.previews.lock().unwrap();
        assert_eq!(
            previews.len(),
            usize::from(matches!(failure, Failure::Tool))
        );
        if let Some(preview) = previews.first() {
            assert_eq!(preview.name, "shell");
            assert_eq!(preview.arguments, "{");
        }
        let records = checkpoint.attempts.lock().unwrap();
        assert_eq!(
            records[0].decision,
            if matches!(failure, Failure::ContextText) {
                RetryDecision::PartialResponse
            } else {
                RetryDecision::ContinuationScheduled
            }
        );
        assert_eq!(records[0].phase, AttemptPhase::Stream);
        if matches!(failure, Failure::TransportText) {
            assert_eq!(records[0].category.as_deref(), Some("transport_timeout"));
        }
        assert_eq!(
            records[0].tool_fragment_observed,
            matches!(failure, Failure::Tool)
        );
    }
}

#[tokio::test]
async fn nonretryable_eof_and_long_server_delay_are_explained_without_early_retry() {
    for (failure, decision) in [
        (Failure::Request, RetryDecision::NonRetryable),
        (Failure::Eof, RetryDecision::AttemptsExhausted),
        (Failure::LongWait, RetryDecision::ServerDelayLimit),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (agent, calls, checkpoint, _, _) = fixture(root.path(), failure);
        assert!(
            execute(&agent, &checkpoint, &CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            if matches!(failure, Failure::Eof) {
                3
            } else {
                1
            }
        );
        assert_eq!(
            checkpoint.attempts.lock().unwrap().last().unwrap().decision,
            decision
        );
    }
}

#[tokio::test]
async fn elapsed_window_refuses_wait_without_dispatching_again() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, calls, checkpoint, _, _) = fixture(root.path(), Failure::Timeout);
    agent.retry.max_elapsed = Duration::from_nanos(1);
    assert!(
        execute(&agent, &checkpoint, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        checkpoint.attempts.lock().unwrap()[0].decision,
        RetryDecision::ElapsedBudget
    );
}

#[tokio::test]
async fn backoff_cancellation_and_authority_revocation_are_durable_stops() {
    for revoke in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (agent, calls, mut checkpoint, _, authority) = fixture(root.path(), Failure::Timeout);
        let cancel = CancellationToken::new();
        if revoke {
            checkpoint.revoke = Some(authority);
        } else {
            checkpoint.cancel = Some(cancel.clone());
        }
        let result = execute(&agent, &checkpoint, &cancel).await;
        assert!(if revoke {
            matches!(result, Err(AgentError::Policy(_)))
        } else {
            matches!(result, Err(AgentError::Cancelled))
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let records = checkpoint.attempts.lock().unwrap();
        assert_eq!(records[0].phase, AttemptPhase::Backoff);
        assert_eq!(
            records[0].decision,
            if revoke {
                RetryDecision::PolicyRevoked
            } else {
                RetryDecision::Cancelled
            }
        );
    }
}

#[tokio::test]
async fn missing_durable_intent_prevents_provider_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let (agent, calls, mut checkpoint, _, _) = fixture(root.path(), Failure::Timeout);
    checkpoint.fail_intent = true;
    assert!(matches!(
        execute(&agent, &checkpoint, &CancellationToken::new()).await,
        Err(AgentError::Checkpoint(_))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn active_provider_waits_time_out_and_observe_revocation() {
    for failure in [Failure::SilentStart, Failure::Idle] {
        let root = tempfile::tempdir().unwrap();
        let (mut agent, calls, checkpoint, _, authority) = fixture(root.path(), failure);
        agent.retry.response_timeout = Duration::from_millis(5);
        agent.retry.stream_idle = Duration::from_millis(5);
        assert!(
            execute(&agent, &checkpoint, &CancellationToken::new())
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert_eq!(
            checkpoint.attempts.lock().unwrap().last().unwrap().decision,
            RetryDecision::AttemptsExhausted
        );
        agent.retry.response_timeout = Duration::from_secs(5);
        agent.retry.stream_idle = Duration::from_secs(5);
        let revoke = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            authority.store(true, Ordering::SeqCst);
        };
        let cancel = CancellationToken::new();
        let (result, _) = tokio::join!(execute(&agent, &checkpoint, &cancel), revoke);
        assert!(matches!(result, Err(AgentError::Policy(_))));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
        assert_eq!(
            checkpoint.attempts.lock().unwrap().last().unwrap().decision,
            RetryDecision::PolicyRevoked
        );
    }
}

struct SlowSecondIntent(Checkpoint);
#[async_trait]
impl RunCheckpoint for SlowSecondIntent {
    fn run_id(&self) -> uuid::Uuid {
        uuid::Uuid::nil()
    }
    async fn canonical(&self, _: &[Message], _: &Usage) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn tool_previews(
        &self,
        previews: &[voyage_protocol::tool_preview::ToolPreview],
    ) -> Result<(), CheckpointError> {
        *self.0.previews.lock().unwrap() = previews.to_vec();
        Ok(())
    }
    async fn partial(&self, _: &str) -> Result<(), CheckpointError> {
        Ok(())
    }
    async fn provider_attempt(&self, record: &ProviderAttempt) -> Result<(), CheckpointError> {
        self.0.provider_attempt(record).await?;
        if record.attempt == 2 && record.decision == RetryDecision::InFlight {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(())
    }
}
#[tokio::test]
async fn checkpoint_delay_cannot_dispatch_beyond_retry_window() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, calls, checkpoint, _, _) = fixture(root.path(), Failure::Timeout);
    agent.retry.max_elapsed = Duration::from_millis(40);
    let checkpoint = SlowSecondIntent(checkpoint);
    assert!(
        execute(&agent, &checkpoint, &CancellationToken::new())
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let rows = checkpoint.0.attempts.lock().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].decision, RetryDecision::ElapsedBudget);
}

mod continuation_tests;

#[tokio::test]
async fn activity_flood_remains_cancellable_without_becoming_conversation_text() {
    let root = tempfile::tempdir().unwrap();
    let (mut agent, calls, checkpoint, _, _) = fixture(root.path(), Failure::Activity);
    agent.retry.stream_idle = Duration::from_millis(5);
    let cancel = CancellationToken::new();
    let trigger = async {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();
    };
    let (result, _) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(execute(&agent, &checkpoint, &cancel), trigger)
    })
    .await
    .unwrap();
    assert!(matches!(result, Err(AgentError::Cancelled)));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let rows = checkpoint.attempts.lock().unwrap();
    assert_eq!(rows[0].decision, RetryDecision::Cancelled);
    assert!(!rows[0].text_observed && !rows[0].tool_fragment_observed);
}

#[tokio::test]
async fn interrupted_reasoning_is_checkpointed_separately_without_tool_or_answer() {
    let root = tempfile::tempdir().unwrap();
    let (agent, _, checkpoint, _, _) = fixture(root.path(), Failure::Reasoning);
    assert!(
        execute(&agent, &checkpoint, &CancellationToken::new())
            .await
            .is_err()
    );
    let disclosures = checkpoint.reasoning.lock().unwrap();
    assert_eq!(disclosures.len(), 1);
    assert_eq!(disclosures[0].text, "disclosure");
    assert!(!disclosures[0].finalized);
    assert!(checkpoint.previews.lock().unwrap().is_empty());
}
