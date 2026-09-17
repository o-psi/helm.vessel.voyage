use super::*;
use crate::model::Role;
use rusqlite::Connection;
use std::time::{Duration, Instant};

async fn fixture() -> (tempfile::TempDir, ManagedSessionOwner, RunOwner) {
    fixture_authorized(None).await
}

pub(super) async fn fixture_authorized(
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
) -> (tempfile::TempDir, ManagedSessionOwner, RunOwner) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    let session = crate::session::Session::new(root.path().into(), "fixture".into());
    let mut journal = Journal::open(directory.clone()).unwrap();
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session.id)
        .await
        .unwrap();
    owner.initialize_process_commands().await.unwrap();
    let Admission::New(run) = owner
        .admit_authorized_after(
            TurnAdmission {
                coordination: None,
                operator_name: None,
                command_id: Uuid::new_v4(),
                machine_id: Uuid::new_v4(),
                principal_id: Uuid::new_v4(),
                session_id: session.id,
                expected_revision: 0,
                expires_at_ms: chrono::Utc::now().timestamp_millis() + 60000,
                prompt: "checkpoint fixture".into(),
                parts: vec![],
            },
            Arc::new(SystemClock),
            || Ok(()),
            authority,
        )
        .await
        .unwrap()
    else {
        panic!("new run required")
    };
    run.storage(|store| store.journal.mark_running(&store.guard, store.run_id))
        .await
        .unwrap();
    (root, owner, run)
}

fn reader(root: &std::path::Path) -> Connection {
    let db = Connection::open_with_flags(
        root.join("journal/journal.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    db.execute_batch("BEGIN; SELECT version FROM attachment_schema;")
        .unwrap();
    db
}

#[derive(Debug)]
struct Authority(AtomicBool);
impl crate::policy::ExecutionAuthority for Authority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.0.load(Ordering::SeqCst), "private grant diagnostic");
        Ok(())
    }
}

#[tokio::test]
async fn grant_revocation_interrupts_canonical_wait_but_allows_outcome_metadata() {
    let authority = Arc::new(Authority(AtomicBool::new(false)));
    let (root, owner, run) = fixture_authorized(Some(authority.clone())).await;
    let mut messages = owner.snapshot().await.unwrap().session.messages;
    messages.push(Message::new(Role::Assistant, "must not be accepted"));
    let db = reader(root.path());
    let checkpoint = run.checkpoint();
    let task =
        tokio::spawn(async move { checkpoint.canonical(&messages, &Usage::default()).await });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    authority.0.store(true, Ordering::SeqCst);
    assert!(
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(owner.snapshot().await.unwrap().session.messages.len(), 1);
    assert_eq!(
        run.token.checkpoint_failure.lock().unwrap().as_deref(),
        Some("Checkpoint failed during canonical history: state or authority validation failed.")
    );
    let attempt = voyage_protocol::provider_attempt::ProviderAttempt {
        retry: Default::default(),
        request_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        provider: "fixture".into(),
        model: "fixture".into(),
        attempt: 1,
        limit: 1,
        started_at_ms: 1,
        duration_ms: 1,
        phase: voyage_protocol::provider_attempt::AttemptPhase::Stream,
        category: None,
        http_status: Some(200),
        text_observed: false,
        tool_fragment_observed: false,
        retry_delay_ms: None,
        decision: voyage_protocol::provider_attempt::RetryDecision::PolicyRevoked,
    };
    // Revocation cannot erase why an already-dispatched request stopped.
    let db = reader(root.path());
    let checkpoint = run.checkpoint();
    let task = tokio::spawn(async move { checkpoint.provider_attempt(&attempt).await });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    db.execute_batch("ROLLBACK").unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        owner
            .snapshot()
            .await
            .unwrap()
            .session
            .run_summaries
            .last()
            .unwrap()
            .provider_attempts
            .len(),
        1
    );
}

#[tokio::test]
async fn actual_catalogue_refresh_can_overlap_repeated_checkpoints() {
    let (root, owner, run) = fixture().await;
    let directory = root.path().join("journal");
    let session = owner.session_id();
    let stopped = Arc::new(AtomicBool::new(false));
    let done = stopped.clone();
    let (started, start) = tokio::sync::oneshot::channel();
    let reads = std::thread::spawn(move || {
        let mut count = 0;
        started.send(()).unwrap();
        while !done.load(Ordering::SeqCst) {
            if crate::catalogue::read(&directory, session).is_ok() {
                count += 1;
            }
        }
        count
    });
    start.await.unwrap();
    let mut result = Ok(());
    for _ in 0..50 {
        result = run.checkpoint().partial("x").await;
        if result.is_err() {
            break;
        }
    }
    stopped.store(true, Ordering::SeqCst);
    assert!(reads.join().unwrap() > 0);
    result.unwrap();
    assert_eq!(run.record().await.unwrap().partial_text, "x".repeat(50));
}

#[tokio::test]
async fn catalogue_reader_overlap_waits_without_repeating_callback_or_text() {
    let (root, owner, run) = fixture().await;
    assert_eq!(
        crate::catalogue::read(&root.path().join("journal"), owner.session_id())
            .unwrap()
            .session_id,
        owner.session_id()
    );
    let db = reader(root.path());
    let checkpoint = run.checkpoint();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = calls.clone();
    let (entered, entering) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        checkpoint
            .storage_named(Operation::PartialText, move |store| {
                count.fetch_add(1, Ordering::SeqCst);
                entered.send(()).unwrap();
                store
                    .journal
                    .append_text(&store.guard, store.run_id, "exactly once")
            })
            .await
    });
    entering.await.unwrap();
    // A current-thread runtime must remain responsive while SQLite waits.
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    db.execute_batch("ROLLBACK").unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(run.record().await.unwrap().partial_text, "exactly once");
    assert!(run.token.checkpoint_failure.lock().unwrap().is_none());

    let mut messages = owner.snapshot().await.unwrap().session.messages;
    messages.push(Message::new(Role::Assistant, "exactly once"));
    let db = reader(root.path());
    let checkpoint = run.checkpoint();
    let task =
        tokio::spawn(async move { checkpoint.canonical(&messages, &Usage::default()).await });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    db.execute_batch("ROLLBACK").unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(
        owner
            .snapshot()
            .await
            .unwrap()
            .session
            .messages
            .last()
            .unwrap()
            .content,
        "exactly once"
    );

    // Waiting must not leak onto unrelated journal callers sharing this store.
    let _db = reader(root.path());
    let mut store = run.store.lock().unwrap();
    let before = Instant::now();
    let Store {
        journal,
        guard,
        run_id,
        ..
    } = &mut *store;
    assert!(
        journal
            .append_text(guard, *run_id, "must roll back")
            .is_err()
    );
    assert!(before.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn busy_exhaustion_rolls_back_and_retains_safe_operation_diagnostic() {
    let (root, owner, run) = fixture().await;
    let db = reader(root.path());
    let before = Instant::now();
    assert!(run.checkpoint().partial("not committed").await.is_err());
    assert!(before.elapsed() >= Duration::from_secs(1));
    assert!(before.elapsed() < Duration::from_secs(4));
    db.execute_batch("ROLLBACK").unwrap();
    assert_eq!(run.record().await.unwrap().partial_text, "");
    let reason = run.token.checkpoint_failure.lock().unwrap().clone();
    assert_eq!(
        reason.as_deref(),
        Some("Checkpoint failed during partial text: database busy.")
    );
    run.persist_terminal(
        RunState::Failed,
        reason.clone(),
        None,
        CancellationToken::new(),
    )
    .await
    .unwrap();
    let saved = owner.snapshot().await.unwrap().session;
    assert_eq!(saved.run_summaries.last().unwrap().detail, reason);
    assert!(!run.record().await.unwrap().final_checkpointed);
}

#[tokio::test]
async fn cancellation_interrupts_lock_wait_without_committing_partial_output() {
    let (root, _owner, run) = fixture().await;
    let db = reader(root.path());
    let cancel = CancellationToken::new();
    run.token.checkpoint_cancel.set(cancel.clone()).unwrap();
    let checkpoint = run.checkpoint();
    let before = Instant::now();
    let task = tokio::spawn(async move { checkpoint.partial("cancelled output").await });
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    cancel.cancel();
    assert!(task.await.unwrap().is_err());
    assert!(before.elapsed() < Duration::from_secs(1));
    db.execute_batch("ROLLBACK").unwrap();
    assert!(run.record().await.unwrap().partial_text.is_empty());
    assert_eq!(
        run.persist_terminal(RunState::Completed, None, None, cancel)
            .await
            .unwrap()
            .state,
        RunState::Cancelled
    );
}

#[tokio::test]
async fn storage_and_validation_failures_do_not_retry_or_expose_raw_diagnostics() {
    let (root, _owner, run) = fixture().await;
    let db = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.execute_batch("CREATE TRIGGER reject_update BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT, 'private-secret-path'); END;").unwrap();
    let before = Instant::now();
    assert!(run.checkpoint().partial("rejected").await.is_err());
    assert!(before.elapsed() < Duration::from_secs(1));
    assert_eq!(
        run.token.checkpoint_failure.lock().unwrap().as_deref(),
        Some("Checkpoint failed during partial text: storage error.")
    );
    assert!(run.record().await.unwrap().partial_text.is_empty());
    assert_eq!(
        checkpoint_failure::reason(
            Operation::CanonicalHistory,
            &anyhow::anyhow!("private-secret-path")
        ),
        "Checkpoint failed during canonical history: state or authority validation failed."
    );
    assert!(!checkpoint_failure::is_public_reason(
        "Checkpoint failed during partial text: private-secret-path."
    ));
}

#[tokio::test]
async fn root_grant_human_decision_roundtrip_is_durable_and_cancellable() {
    use crate::server::decisions::Decisions;
    use crate::tools::roots::{RootGrantRequest, RootLifetime, RootPermission};
    use crate::tools::{ApprovalOutcome, Approver};
    let (root, owner, run) = fixture().await;
    let incarnation = Uuid::new_v4();
    let cancel = CancellationToken::new();
    let decisions = Decisions {
        owner: owner.clone(),
        run: run.run_id,
        incarnation,
        cancel: cancel.clone(),
        timeout: Duration::from_secs(2),
    };
    let mut context = crate::tools::reliability_tests::context(root.path());
    context.cancellation = cancel.clone();
    let request = RootGrantRequest {
        id: Uuid::new_v4(),
        path: root.path().into(),
        permission: RootPermission::Write,
        lifetime: RootLifetime::CurrentRun,
        reason: "fixture".into(),
    };
    let worker = {
        let decisions = decisions.clone();
        let request = request.clone();
        let context = context.clone();
        tokio::spawn(async move { decisions.request_root(&request, &context).await })
    };
    for _ in 0..100 {
        if !owner
            .decisions(incarnation)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let pending = owner.decisions(incarnation).await.unwrap();
    assert_eq!(
        pending[0]["request"]["root_grant"]["lifetime"],
        "current_run"
    );
    let command = voyage_protocol::process::RuntimeCommand::Respond {
        command_id: Uuid::new_v4(),
        expected_revision: owner.snapshot().await.unwrap().revision,
        expires_at_ms: pending[0]["expires_at_ms"].as_u64().unwrap(),
        run_id: run.run_id,
        decision_id: request.id,
        response: serde_json::json!({"root_grant":"approved"}),
    };
    owner
        .respond_decision(incarnation, command, || Ok(()))
        .await
        .unwrap();
    assert_eq!(worker.await.unwrap(), ApprovalOutcome::Approved);
    cancel.cancel();
    let request = RootGrantRequest {
        id: Uuid::new_v4(),
        ..request
    };
    assert_eq!(
        decisions.request_root(&request, &context).await,
        ApprovalOutcome::Cancelled
    );
}
