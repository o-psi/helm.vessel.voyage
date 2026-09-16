use super::*;
use crate::completion::runtime::RunReference;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

fn fixture() -> (tempfile::TempDir, Accounting, RunReference) {
    let root = tempfile::tempdir().unwrap();
    let mut store = Store::open(root.path().join("inference")).unwrap();
    let project = store.project(root.path()).unwrap();
    let accounting = Accounting {
        store: Arc::new(Mutex::new(store)),
        project: Some(project),
        provider: "offline".into(),
        agent: None,
    };
    let reference = RunReference {
        session_id: Uuid::new_v4(),
        run_id: Uuid::new_v4(),
    };
    (root, accounting, reference)
}
fn query() -> history::Query {
    history::Query {
        from: Utc::now() - chrono::Duration::hours(1),
        until: Utc::now() + chrono::Duration::hours(1),
        group_by: history::GroupBy::Model,
        detail: None,
        offset: 0,
        limit: 100,
        snapshot: None,
    }
}

#[tokio::test]
async fn accounting_binds_root_and_child_attempts_to_one_durable_project() {
    let (_root, accounting, reference) = fixture();
    accounting.bind(reference.session_id).await.unwrap();
    accounting.bind(reference.session_id).await.unwrap();
    let mut child = accounting.clone();
    child.project = None;
    child.agent = Some(Uuid::new_v4());
    child.bind(reference.session_id).await.unwrap();
    assert!(child.bind(Uuid::new_v4()).await.is_err());
    for (i, owner) in [&accounting, &child].into_iter().enumerate() {
        let permit = owner
            .admit(&reference, "offline-model", Purpose::Conversation)
            .await
            .unwrap();
        assert!(permit.retained);
        let bytes = voyage_protocol::provider_attempt::RequestBytes {
            instructions: 10,
            schemas: 20,
            history: 30,
            envelope: 40,
            total: 100,
        };
        owner.request_bytes(&permit, Some(bytes)).await.unwrap();
        owner
            .report(
                &permit,
                crate::provider::ReportedUsage {
                    input_tokens: Some(20),
                    output_tokens: Some(3),
                },
            )
            .await
            .unwrap();
        owner
            .finish(
                &permit,
                if i == 0 {
                    AttemptOutcome::Completed
                } else {
                    AttemptOutcome::Failed
                },
            )
            .await
            .unwrap();
    }
    for project_scope in [true, false] {
        let history = accounting
            .history(
                reference.session_id,
                project_scope,
                query(),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(history.totals.retained_attempts, 2);
        assert_eq!(history.totals.completed, 1);
        assert_eq!(history.totals.failed, 1);
        assert_eq!(history.totals.input.reported_sum, Some(40));
        assert_eq!(history.totals.output.reported_sum, Some(6));
    }
    let status = accounting.status(reference.session_id).await.unwrap();
    assert_eq!(status.len(), 2);
    assert!(status.iter().all(|s| s.consumed == 2));
    let attempts = accounting
        .store
        .lock()
        .unwrap()
        .attempts(Scope::Session(reference.session_id), 0, 10)
        .unwrap();
    assert!(attempts.iter().any(|a| a.attribution.agent == child.agent));
    assert!(
        attempts
            .iter()
            .all(|a| a.request_bytes.as_ref().unwrap().total == 100)
    );
}

#[tokio::test]
async fn cancelled_history_and_cross_project_admission_do_not_change_counters() {
    let (root, accounting, reference) = fixture();
    accounting.bind(reference.session_id).await.unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(
        accounting
            .history(reference.session_id, false, query(), cancellation.clone())
            .await
            .is_err()
    );
    // The pre-cancelled select must not even resolve the default store path.
    assert!(
        read_history(root.path().to_owned(), None, query(), cancellation)
            .await
            .is_err()
    );
    let other = root.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let other_project = accounting.store.lock().unwrap().project(&other).unwrap();
    let mut wrong = accounting.clone();
    wrong.project = Some(other_project);
    assert!(
        wrong
            .admit(&reference, "offline", Purpose::Title)
            .await
            .is_err()
    );
    assert!(accounting.status(Uuid::new_v4()).await.is_err());
    assert!(
        accounting
            .status(reference.session_id)
            .await
            .unwrap()
            .iter()
            .all(|s| s.consumed == 0)
    );
}

#[tokio::test]
async fn unretained_permits_skip_missing_rows_and_worker_errors_are_sanitized() {
    let (_root, accounting, _) = fixture();
    let permit = Permit {
        id: Uuid::new_v4(),
        retained: false,
        warnings: vec![],
    };
    accounting.request_bytes(&permit, None).await.unwrap();
    accounting
        .request_bytes(
            &permit,
            Some(voyage_protocol::provider_attempt::RequestBytes {
                instructions: 0,
                schemas: 0,
                history: 0,
                envelope: 0,
                total: 0,
            }),
        )
        .await
        .unwrap();
    accounting
        .report(&permit, crate::provider::ReportedUsage::default())
        .await
        .unwrap();
    accounting
        .finish(&permit, AttemptOutcome::Completed)
        .await
        .unwrap();
    let error = blocking::<()>(|| anyhow::bail!("private SQLite/path detail"))
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Failure>(),
        Some(Failure::Unavailable)
    ));
    assert!(!error.to_string().contains("private SQLite"));
    for failure in [
        Failure::Invalid,
        Failure::Busy,
        Failure::Capacity,
        Failure::Unavailable,
    ] {
        let error = blocking::<()>(move || Err(failure.into()))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), failure.to_string());
    }
}

#[tokio::test]
async fn allowance_exhaustion_is_enforced_across_runtime_clones() {
    let (_root, accounting, reference) = fixture();
    accounting.bind(reference.session_id).await.unwrap();
    let scope = Scope::Session(reference.session_id);
    let status = accounting.store.lock().unwrap().inspect(scope).unwrap();
    accounting
        .store
        .lock()
        .unwrap()
        .configure(&Change {
            operation: Uuid::new_v4(),
            scope,
            expected_revision: status.revision,
            limit: Some(1),
            warning: Some(1),
            reason: "offline allowance".into(),
        })
        .unwrap();
    let permit = accounting
        .admit(&reference, "model", Purpose::Title)
        .await
        .unwrap();
    assert!(!permit.warnings.is_empty());
    let error = accounting
        .clone()
        .admit(&reference, "model", Purpose::Conversation)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<Failure>(),
        Some(Failure::Exhausted)
    ));
    assert_eq!(
        accounting.status(reference.session_id).await.unwrap()[1].remaining(),
        Some(0)
    );
}
