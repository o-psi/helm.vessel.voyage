use super::*;
use crate::inference::{Change, Scope, Store};
use crate::provider::{ProviderStream, ProviderStreamEvent, ReportedUsage};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Script {
    calls: Arc<AtomicUsize>,
    fail: bool,
    hold: bool,
    ready: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl Provider for Script {
    async fn complete(
        &self,
        _: ModelRequest,
    ) -> Result<crate::model::ModelResponse, ProviderError> {
        unreachable!()
    }
    async fn models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        Ok(vec![ModelInfo::minimal(
            crate::titles::title_model().unwrap(),
        )])
    }
    async fn stream(&self, _: ModelRequest) -> Result<ProviderStream, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(ProviderError::Unavailable("retry fixture".into()));
        }
        let hold = self.hold;
        let ready = self.ready.clone();
        Ok(Box::pin(async_stream::stream! {
            yield Ok(ProviderStreamEvent::UsageReported(ReportedUsage { input_tokens:Some(7),output_tokens:None }));
            ready.notify_one();
            if hold { std::future::pending::<()>().await; }
            yield Ok(ProviderStreamEvent::Completed(crate::model::ModelResponse {
                message: Message::new(crate::model::Role::Assistant,"done"),usage:Usage { input_tokens:7,output_tokens:0 }
            }));
        }))
    }
}
async fn setup(
    limit: u64,
    fail: bool,
    hold: bool,
) -> (
    tempfile::TempDir,
    Agent,
    Arc<AtomicUsize>,
    Uuid,
    Uuid,
    Arc<tokio::sync::Notify>,
) {
    let temp = tempfile::tempdir().unwrap();
    let coordinator =
        crate::completion::runtime::Coordinator::open(temp.path().join("completion"), temp.path())
            .unwrap();
    let scope =
        crate::completion::runtime::RunHandle::create(coordinator, Uuid::new_v4(), Uuid::new_v4())
            .await
            .unwrap();
    let mut store = Store::open(temp.path().join("inference")).unwrap();
    let project = store.project(temp.path()).unwrap();
    let session = scope.reference().session_id;
    store.bind_session(project, session).unwrap();
    store
        .configure(&Change {
            operation: Uuid::new_v4(),
            scope: Scope::Project(project),
            expected_revision: 0,
            limit: Some(limit),
            warning: None,
            reason: "fixture allowance".into(),
        })
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let ready = Arc::new(tokio::sync::Notify::new());
    let mut agent = super::tests::agent(
        Box::new(Script {
            calls: calls.clone(),
            fail,
            hold,
            ready: ready.clone(),
        }),
        &temp,
    )
    .with_inference_accounting(crate::inference::runtime::Accounting::fixture(
        store, project,
    ))
    .with_retry_policy(RetryPolicy {
        max_attempts: 4,
        initial_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    });
    agent.context.completion = Some(scope);
    (temp, agent, calls, project, session, ready)
}
use uuid::Uuid;
#[tokio::test]
async fn each_retry_needs_a_new_permit_and_exhaustion_never_dispatches() {
    let (temp, agent, calls, project, _, _) = setup(2, true, false).await;
    let error = agent.run(Vec::new(), "fixture".into()).await.unwrap_err();
    assert!(error.to_string().contains("allowance exhausted"), "{error}");
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let store = Store::open(temp.path().join("inference")).unwrap();
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.outcome == crate::inference::AttemptOutcome::Failed)
    );
}
#[tokio::test]
async fn cancellation_after_partial_usage_retains_unknown_outcome_without_refund() {
    let (temp, agent, calls, project, _, ready) = setup(1, false, true).await;
    let cancel = CancellationToken::new();
    let work = agent.run_with_cancel(Vec::new(), "fixture".into(), cancel.clone());
    tokio::pin!(work);
    tokio::select! { result=&mut work => panic!("unexpected result: {result:?}"), _=ready.notified() => {} }
    cancel.cancel();
    assert!(work.await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let store = Store::open(temp.path().join("inference")).unwrap();
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows[0].input_tokens, Some(7));
    assert_eq!(rows[0].output_tokens, None);
    assert_eq!(rows[0].outcome, crate::inference::AttemptOutcome::Unknown);
    assert_eq!(
        store.inspect(Scope::Project(project)).unwrap().remaining(),
        Some(0)
    );
}
#[tokio::test]
async fn title_requires_session_attribution_and_cannot_bypass_exhaustion() {
    let (_temp, agent, calls, _, session_id, _) = setup(1, false, false).await;
    agent.run(Vec::new(), "fixture".into()).await.unwrap();
    let mut session = crate::session::Session::new(
        agent.context.policy.workspace().to_path_buf(),
        "fixture".into(),
    );
    session.id = session_id;
    session.messages.push(Message::new(
        crate::model::Role::User,
        "Explain the fixture",
    ));
    session
        .completion_runs
        .push(agent.context.completion.as_ref().unwrap().reference());
    assert!(
        agent
            .generate_title_for_session(&session, CancellationToken::new())
            .await
            .is_none()
    );
    assert!(
        agent
            .generate_title(&session.messages, CancellationToken::new())
            .await
            .is_none()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn cancellation_before_admission_consumes_nothing() {
    let (temp, agent, calls, project, _, _) = setup(1, false, false).await;
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(
        agent
            .run_with_cancel(Vec::new(), "fixture".into(), cancel)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        Store::open(temp.path().join("inference"))
            .unwrap()
            .inspect(Scope::Project(project))
            .unwrap()
            .consumed,
        0
    );
}
#[tokio::test]
async fn failed_final_attempt_record_preserves_known_completed_usage() {
    let (temp, agent, calls, project, session_id, _) = setup(2, false, false).await;
    // Failure after report publication but before outcome finalization must not
    // turn a completed provider response into another provider dispatch.
    let connection =
        rusqlite::Connection::open(temp.path().join("inference/journal.sqlite3")).unwrap();
    connection.execute_batch("CREATE TRIGGER failed_outcome BEFORE UPDATE ON attempts WHEN json_extract(NEW.record,'$.outcome')='completed' BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(agent.run(Vec::new(), "fixture".into()).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let store = Store::open(temp.path().join("inference")).unwrap();
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows[0].attribution.session, session_id);
    assert_eq!(rows[0].input_tokens, Some(7));
    assert_eq!(rows[0].outcome, crate::inference::AttemptOutcome::Unknown);
}

struct CompleteOnly;
#[async_trait]
impl Provider for CompleteOnly {
    async fn complete(
        &self,
        _: ModelRequest,
    ) -> Result<crate::model::ModelResponse, ProviderError> {
        Ok(crate::model::ModelResponse {
            message: Message::new(crate::model::Role::Assistant, "complete-only"),
            usage: Usage {
                input_tokens: 9,
                output_tokens: 2,
            },
        })
    }
}
#[tokio::test]
async fn default_complete_adapter_does_not_invent_usage_availability() {
    let (temp, mut agent, _, project, _, _) = setup(1, false, false).await;
    agent.provider = Box::new(CompleteOnly);
    let outcome = agent.run(Vec::new(), "fixture".into()).await.unwrap();
    assert_eq!(outcome.usage.input_tokens, 9);
    let store = Store::open(temp.path().join("inference")).unwrap();
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows[0].outcome, crate::inference::AttemptOutcome::Completed);
    assert_eq!(rows[0].input_tokens, None);
    assert_eq!(rows[0].output_tokens, None);
}
#[tokio::test]
async fn title_uses_same_session_project_and_explicit_title_purpose() {
    let (temp, agent, calls, project, session_id, _) = setup(2, false, false).await;
    agent.run(Vec::new(), "fixture".into()).await.unwrap();
    let mut session = crate::session::Session::new(
        agent.context.policy.workspace().to_owned(),
        "fixture".into(),
    );
    session.id = session_id;
    session.messages.push(Message::new(
        crate::model::Role::User,
        "Explain a useful fixture",
    ));
    session
        .completion_runs
        .push(agent.context.completion.as_ref().unwrap().reference());
    assert!(
        agent
            .generate_title_for_session(&session, CancellationToken::new())
            .await
            .is_some()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    let rows = Store::open(temp.path().join("inference"))
        .unwrap()
        .attempts(Scope::Project(project), 0, 100)
        .unwrap();
    assert_eq!(
        rows[1].attribution.purpose,
        crate::inference::Purpose::Title
    );
    assert_eq!(rows[1].attribution.session, session_id);
    assert_eq!(rows[1].attribution.run, rows[0].attribution.run);
}
struct PausedWarning(Arc<tokio::sync::Notify>);
#[async_trait]
impl EventSink for PausedWarning {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::InferenceWarning(_)) {
            self.0.notify_one();
            std::future::pending::<()>().await;
        }
    }
}
#[tokio::test]
async fn cancellation_after_durable_admission_before_dispatch_keeps_unknown_permit() {
    let (temp, mut agent, calls, project, _, _) = setup(1, false, false).await;
    let ready = Arc::new(tokio::sync::Notify::new());
    agent.sink = Arc::new(PausedWarning(ready.clone()));
    let mut store = Store::open(temp.path().join("inference")).unwrap();
    store
        .configure(&Change {
            operation: Uuid::new_v4(),
            scope: Scope::Project(project),
            expected_revision: 1,
            limit: Some(1),
            warning: Some(1),
            reason: "warning-boundary fixture".into(),
        })
        .unwrap();
    let cancel = CancellationToken::new();
    let work = agent.run_with_cancel(Vec::new(), "fixture".into(), cancel.clone());
    tokio::pin!(work);
    tokio::select! { result=&mut work=>panic!("unexpected {result:?}"), _=ready.notified()=>{} }
    cancel.cancel();
    assert!(work.await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let rows = store.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, crate::inference::AttemptOutcome::Unknown);
    assert_eq!(
        store.inspect(Scope::Project(project)).unwrap().remaining(),
        Some(0)
    );
}
