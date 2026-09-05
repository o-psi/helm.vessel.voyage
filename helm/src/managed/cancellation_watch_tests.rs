use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn busy() -> anyhow::Error {
    rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None)
        .into()
}

#[tokio::test(start_paused = true)]
async fn polling_busy_exhaustion_keeps_original_five_second_budget() {
    let calls = AtomicUsize::new(0);
    let start = tokio::time::Instant::now();
    let result = poll_local_cancellation(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err(busy()) }
    })
    .await;
    assert!(result.is_err());
    assert_eq!(start.elapsed(), Duration::from_secs(5));
    assert!(calls.load(Ordering::SeqCst) > 1);
}

#[tokio::test]
async fn polling_nonbusy_failure_and_boolean_observations_are_not_retried() {
    for value in [false, true] {
        let calls = AtomicUsize::new(0);
        assert_eq!(
            poll_local_cancellation(|| {
                calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(value) }
            })
            .await
            .unwrap(),
            value
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
    let locked_calls = AtomicUsize::new(0);
    let locked = poll_local_cancellation(|| {
        locked_calls.fetch_add(1, Ordering::SeqCst);
        async {
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_LOCKED),
                None,
            )
            .into())
        }
    })
    .await
    .unwrap_err();
    assert_eq!(
        locked
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseLocked)
    );
    assert_eq!(locked_calls.load(Ordering::SeqCst), 1);
    let calls = AtomicUsize::new(0);
    let error = poll_local_cancellation(|| {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err(anyhow::anyhow!("invalid durable evidence")) }
    })
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "invalid durable evidence");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn polling_real_exclusive_lock_waits_then_observes_same_run_cancel() {
    use helm::attachment::{journal::TurnAdmission, runtime::Admission};
    let temporary = tempfile::tempdir().unwrap();
    let directory = temporary.path().join("journal");
    let session = Session::new(temporary.path().to_owned(), "fixture".into());
    let mut journal = Journal::open(directory.clone()).unwrap();
    journal.create_session(&session).unwrap();
    drop(journal);
    let owner = ManagedSessionOwner::open(directory.clone(), session.id)
        .await
        .unwrap();
    let actor = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    let Admission::New(run) = owner
        .admit(TurnAdmission {
            command_id: Uuid::new_v4(),
            machine_id: actor.installation_id,
            principal_id: actor.principal_id,
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: SystemClock.now_ms().unwrap() + 60000,
            prompt: "no effects".into(),
        })
        .await
        .unwrap()
    else {
        panic!("new run")
    };
    let run_id = run.record().await.unwrap().id;
    let blocker = rusqlite::Connection::open(directory.join("journal.sqlite3")).unwrap();
    blocker.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let reader = owner.clone();
    let (first, first_seen) = tokio::sync::oneshot::channel();
    let mut first = Some(first);
    let polling = tokio::spawn(async move {
        poll_local_cancellation(|| {
            let reader = reader.clone();
            let observed = observed.clone();
            let first = first.take();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                let result = reader.local_cancel_requested(run_id).await;
                if let Some(first) = first {
                    let code = result
                        .as_ref()
                        .unwrap_err()
                        .downcast_ref::<rusqlite::Error>()
                        .unwrap()
                        .sqlite_error_code();
                    first.send(code).unwrap();
                }
                result
            }
        })
        .await
    });
    assert_eq!(
        first_seen.await.unwrap(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    tokio::time::sleep(Duration::from_millis(20)).await;
    let prematurely_finished = polling.is_finished();
    blocker.execute_batch("ROLLBACK").unwrap();
    let result = polling.await.unwrap();
    assert!(
        !prematurely_finished,
        "transient Busy must remain a bounded observation attempt"
    );
    assert!(!result.unwrap());
    assert!(attempts.load(Ordering::SeqCst) > 1);
    let mut journal = Journal::open(directory).unwrap();
    journal
        .request_cancel_local(&LocalCancelRequest {
            session_id: session.id,
            run_id,
            installation_id: actor.installation_id,
            principal_id: actor.principal_id,
            expires_at_ms: SystemClock.now_ms().unwrap() + 60000,
        })
        .unwrap();
    assert!(
        poll_local_cancellation(|| owner.local_cancel_requested(run_id))
            .await
            .unwrap()
    );
    assert!(
        poll_local_cancellation(|| owner.local_cancel_requested(Uuid::new_v4()))
            .await
            .is_err()
    );
    assert_eq!(run.record().await.unwrap().state, RunState::Accepted);
}
