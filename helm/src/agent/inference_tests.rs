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

struct ResumableTitleWarning {
    ready: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl EventSink for ResumableTitleWarning {
    async fn emit(&self, event: AgentEvent) {
        if matches!(event, AgentEvent::InferenceWarning(_)) {
            self.ready.notify_one();
            self.release.notified().await;
        }
    }
}

async fn title_profile_boundary(after_permit: bool) {
    use crate::policy_profile::{
        Builtin, Overrides,
        selection::{Selection, SelectionRequest},
        store::{Action, ProfileChange, ProfileStore},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (temp, mut agent, _, project, session_id, _) = setup(1, false, false).await;
    let directory = temp.path().join("profiles");
    let profiles = ProfileStore::open(&directory).unwrap();
    let snapshot = profiles
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "title-fixture".into(),
            expected_revision: 0,
            action: Action::Create {
                rules: Builtin::Restricted.document().rules,
            },
        })
        .unwrap()
        .snapshot;
    let mut config = crate::config::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        ..Default::default()
    };
    let selection = SelectionRequest {
        directory,
        name: snapshot.name.clone(),
        revision: snapshot.revision,
        digest: snapshot.digest().unwrap(),
        explicit: Overrides::default(),
    };
    config.policy_profile = Some(Selection::bind(&config, temp.path(), selection, None).unwrap());
    agent.context.policy =
        Arc::new(crate::policy::Policy::new(&config, temp.path().into()).unwrap());
    agent.check_current_policy().unwrap();

    let ready = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    agent.provider = Box::new(crate::provider::OpenAiProvider::new(
        "offline-title-fixture".into(),
        Some(format!("http://{}/v1", listener.local_addr().unwrap())),
    ));
    let posts = Arc::new(AtomicUsize::new(0));
    let server_posts = posts.clone();
    let server_ready = ready.clone();
    let server_release = release.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let (end, length) = loop {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() < 128 * 1024);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                    assert!(headers.contains("authorization: bearer offline-title-fixture"));
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .map(|text| text.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    break (end + 4, length);
                }
            };
            while bytes.len() < end + length {
                let mut buffer = [0; 4096];
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let first = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .next()
                .unwrap()
                .to_owned();
            let body = if first.starts_with("GET /v1/models ") {
                if !after_permit {
                    server_ready.notify_one();
                    server_release.notified().await;
                }
                serde_json::to_string(
                    &serde_json::json!({"data":[{"id":crate::titles::title_model().unwrap()}]}),
                )
                .unwrap()
            } else {
                assert!(first.starts_with("POST /v1/chat/completions "), "{first}");
                server_posts.fetch_add(1, Ordering::SeqCst);
                let request: serde_json::Value =
                    serde_json::from_slice(&bytes[end..end + length]).unwrap();
                assert_eq!(request["model"], crate::titles::title_model().unwrap());
                format!(
                    "data: {}\n\ndata: [DONE]\n\n",
                    serde_json::json!({"choices":[{"delta":{"content":"Useful fixture title"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}})
                )
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",if first.starts_with("GET ") {"application/json"} else {"text/event-stream"},body.len(),body).as_bytes()).await.unwrap();
        }
    });
    if after_permit {
        let mut ledger = Store::open(temp.path().join("inference")).unwrap();
        ledger
            .configure(&Change {
                operation: Uuid::new_v4(),
                scope: Scope::Project(project),
                expected_revision: 1,
                limit: Some(1),
                warning: Some(1),
                reason: "Observe committed title admission".into(),
            })
            .unwrap();
        agent.sink = Arc::new(ResumableTitleWarning {
            ready: ready.clone(),
            release: release.clone(),
        });
    }
    let mut session = crate::session::Session::new(temp.path().into(), "fixture".into());
    session.id = session_id;
    session.messages.push(Message::new(
        crate::model::Role::User,
        "Explain a useful fixture",
    ));
    session
        .completion_runs
        .push(agent.context.completion.as_ref().unwrap().reference());
    let work = agent.generate_title_for_session(&session, CancellationToken::new());
    tokio::pin!(work);
    tokio::select! {result=&mut work=>panic!("title completed before boundary: {result:?}"), _=ready.notified()=>{}}
    let action = if after_permit {
        let mut rules = Builtin::Restricted.document().rules;
        rules.deny_commands.push("fixture-title-command".into());
        Action::Replace { rules }
    } else {
        Action::Delete {}
    };
    profiles
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "title-fixture".into(),
            expected_revision: 1,
            action,
        })
        .unwrap();
    assert!(
        agent.check_current_policy().is_err(),
        "real selected profile must now be stale"
    );
    assert!(
        agent.context.policy.check_execution_authority().is_ok(),
        "foreground guard alone does not validate profiles"
    );
    release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), &mut work)
        .await
        .unwrap();
    let posts = posts.load(Ordering::SeqCst);
    server.abort();
    let _ = server.await;
    let ledger = Store::open(temp.path().join("inference")).unwrap();
    let state = ledger.inspect(Scope::Project(project)).unwrap();
    let attempts = ledger.attempts(Scope::Project(project), 0, 100).unwrap();
    assert_eq!(
        posts, 0,
        "stale policy dispatched native title inference; consumed={}, attempts={attempts:?}",
        state.consumed
    );
    assert!(result.is_none());
    assert_eq!(state.consumed, u64::from(after_permit));
    assert_eq!(attempts.len(), usize::from(after_permit));
    if after_permit {
        assert_eq!(
            attempts[0].outcome,
            crate::inference::AttemptOutcome::Unknown
        );
        assert_eq!(
            attempts[0].attribution.purpose,
            crate::inference::Purpose::Title
        );
        assert_eq!(attempts[0].attribution.session, session_id);
        assert!(attempts[0].input_tokens.is_none() && attempts[0].output_tokens.is_none());
    }
}

#[tokio::test]
async fn title_profile_changed_during_model_discovery_prevents_admission() {
    title_profile_boundary(false).await;
}
#[tokio::test]
async fn title_profile_changed_after_admission_keeps_unknown_permit_without_dispatch() {
    title_profile_boundary(true).await;
}
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
