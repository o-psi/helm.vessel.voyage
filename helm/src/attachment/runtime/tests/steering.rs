use super::*;
use crate::attachment::journal::{SteeringActor, SteeringAdmission};

pub(super) fn allow_actor(_: SteeringActor, _: Uuid, _: Uuid) -> anyhow::Result<()> {
    Ok(())
}
pub(super) async fn request(owner: &RunOwner, text: &str) -> SteeringAdmission {
    let record = owner.record().await.unwrap();
    let session_id = record.session_id;
    let revision = owner
        .storage(move |store| Ok(store.journal.load_session(session_id)?.revision))
        .await
        .unwrap();
    SteeringAdmission {
        receipt_id: Uuid::new_v4(),
        session_id,
        run_id: record.id,
        actor: SteeringActor {
            machine_id: record.machine_id,
            principal_id: record.principal_id,
        },
        expected_revision: revision,
        expires_at_ms: 60_000,
        text: text.into(),
    }
}
#[tokio::test]
async fn admission_requires_current_authority_and_revocation_stops_dispatch() {
    use std::sync::atomic::AtomicBool;
    let (_dir, mut owner, agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    let allowed = Arc::new(AtomicBool::new(false));
    let observed = allowed.clone();
    let handle = owner
        .enable_steering_with_clock(
            Arc::new(move |_, _, _| {
                anyhow::ensure!(observed.load(Ordering::SeqCst), "actor revoked");
                Ok(())
            }),
            Arc::new(|| Ok(1)),
        )
        .unwrap();
    let guidance = request(&owner, "fresh authorization").await;
    assert!(handle.submit(guidance.clone()).await.is_err());
    allowed.store(true, Ordering::SeqCst);
    handle.submit(guidance).await.unwrap();
    allowed.store(false, Ordering::SeqCst);
    assert!(
        owner
            .execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn raw_managed_receiver_is_rejected_without_dispatch() {
    let (_dir, mut owner, agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    let (sender, receiver) = crate::agent::steering_channel(1);
    sender.try_send("unacknowledged".into()).unwrap();
    assert!(
        owner
            .execute(&agent, CancellationToken::new(), Some(receiver))
            .await
            .is_err()
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn failed_queue_never_reaches_runtime_and_failed_rejection_poisons_turn() {
    for rejecting in [false, true] {
        let (dir, mut owner, agent, requests, effects, _) =
            setup("success", Arc::new(SilentSink)).await;
        let handle = owner
            .enable_steering_with_clock(Arc::new(allow_actor), Arc::new(|| Ok(1)))
            .unwrap();
        let guidance = request(&owner, "must not dispatch").await;
        let db = fixture_database(dir.path().join("attachment/journal.sqlite3")).unwrap();
        if rejecting {
            drop(owner.steering_receiver.take());
            db.execute_batch("CREATE TRIGGER fail_reject BEFORE UPDATE ON steering WHEN NEW.status='not_applied' BEGIN SELECT RAISE(ABORT,'reject unavailable'); END;").unwrap();
        } else {
            db.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON steering BEGIN SELECT RAISE(ABORT,'queue unavailable'); END;").unwrap();
        }
        assert!(handle.submit(guidance.clone()).await.is_err());
        if rejecting {
            assert!(
                owner
                    .execute(&agent, CancellationToken::new(), None)
                    .await
                    .is_err()
            );
            assert_eq!(requests.load(Ordering::SeqCst), 0);
            assert_eq!(effects.load(Ordering::SeqCst), 0);
        } else {
            db.execute_batch("DROP TRIGGER fail_queue;").unwrap();
            assert!(
                owner
                    .execute(&agent, CancellationToken::new(), None)
                    .await
                    .is_ok()
            );
            let journal = Journal::open(dir.path().join("attachment")).unwrap();
            assert!(journal.steering_record(guidance.receipt_id).is_err());
        }
    }
}

#[tokio::test]
async fn steering_clock_is_sampled_at_submission_and_again_before_application() {
    use std::sync::atomic::AtomicI64;
    for expire_before_queue in [false, true] {
        let (dir, mut owner, agent, requests, effects, _) =
            setup("success", Arc::new(SilentSink)).await;
        let now = Arc::new(AtomicI64::new(1));
        let observed = now.clone();
        let authorization_clock = now.clone();
        let handle = owner
            .enable_steering_with_clock(
                Arc::new(move |_, _, _| {
                    if expire_before_queue {
                        authorization_clock.store(60_000, Ordering::SeqCst);
                    }
                    Ok(())
                }),
                Arc::new(move || Ok(observed.load(Ordering::SeqCst))),
            )
            .unwrap();
        let guidance = request(&owner, "expiry cannot expand authority").await;
        if expire_before_queue {
            // A completed authorization callback can advance time before the
            // transactional deadline check; no caller-sampled timestamp exists.
            assert!(handle.submit(guidance.clone()).await.is_err());
            let journal = Journal::open(dir.path().join("attachment")).unwrap();
            assert!(journal.steering_record(guidance.receipt_id).is_err());
        } else {
            handle.submit(guidance.clone()).await.unwrap();
            now.store(guidance.expires_at_ms, Ordering::SeqCst);
            assert!(handle.submit(guidance.clone()).await.unwrap().duplicate);
            assert!(
                owner
                    .execute(&agent, CancellationToken::new(), None)
                    .await
                    .is_err()
            );
            let journal = Journal::open(dir.path().join("attachment")).unwrap();
            let receipt = journal.steering_record(guidance.receipt_id).unwrap();
            assert_eq!(receipt.status, crate::model::SteeringStatus::NotApplied);
        }
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abandoned_submit_keeps_fence_until_durable_closed_rejection() {
    let (dir, mut owner, _agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    let handle = owner
        .enable_steering_with_clock(Arc::new(allow_actor), Arc::new(|| Ok(1)))
        .unwrap();
    let guidance = request(&owner, "acknowledgement can be lost").await;
    let session_id = guidance.session_id;
    let receipt_id = guidance.receipt_id;
    let (queued_tx, queued_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let submit = tokio::spawn(async move {
        handle
            .submit_after_queue(guidance, move || {
                queued_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .unwrap();
            })
            .await
    });
    queued_rx.await.unwrap();
    let journal = Journal::open(dir.path().join("attachment")).unwrap();
    assert_eq!(
        journal.steering_record(receipt_id).unwrap().status,
        crate::model::SteeringStatus::Queued
    );
    submit.abort();
    assert!(submit.await.unwrap_err().is_cancelled());
    drop(owner); // closes the receiver, but the worker still owns its fence
    assert!(journal.acquire_execution(session_id).is_err());
    release_tx.send(()).unwrap();
    // Arc's strong count reaches zero before Store finishes dropping its
    // journal and execution guard. Observe the OS lease itself, not refcounts.
    let _guard = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(guard) = journal.acquire_execution(session_id) {
                break guard;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let receipt = journal.steering_record(receipt_id).unwrap();
    assert_eq!(receipt.status, crate::model::SteeringStatus::NotApplied);
    assert_eq!(
        receipt.reason,
        Some(crate::attachment::journal::SteeringRejection::Closed)
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_settles_queued_receipt_without_dispatch() {
    let (dir, mut owner, agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    let handle = owner
        .enable_steering_with_clock(Arc::new(allow_actor), Arc::new(|| Ok(1)))
        .unwrap();
    let guidance = request(&owner, "cancelled input").await;
    handle.submit(guidance.clone()).await.unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(owner.execute(&agent, cancel, None).await.is_err());
    let receipt = handle.submit(guidance).await.unwrap();
    assert!(receipt.duplicate);
    assert_eq!(
        receipt.record.status,
        crate::model::SteeringStatus::NotApplied
    );
    assert_eq!(
        receipt.record.reason,
        Some(crate::attachment::journal::SteeringRejection::Cancelled)
    );
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let journal = Journal::open(dir.path().join("attachment")).unwrap();
    assert_eq!(
        journal
            .load_session(receipt.record.request.session_id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
}

#[tokio::test]
async fn channel_full_is_durable_not_applied_and_exact_retry_does_not_resend() {
    let (_dir, mut owner, agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    let (sender, receiver) = crate::agent::steering_channel(1);
    owner.steering_sender = sender;
    owner.steering_receiver = Some(receiver);
    let handle = owner
        .enable_steering_with_clock(Arc::new(allow_actor), Arc::new(|| Ok(1)))
        .unwrap();
    let first = request(&owner, "first queued").await;
    handle.submit(first).await.unwrap();
    let second = request(&owner, "full channel rejected").await;
    let rejected = handle.submit(second.clone()).await.unwrap();
    assert_eq!(
        rejected.record.status,
        crate::model::SteeringStatus::NotApplied
    );
    assert_eq!(
        rejected.record.reason,
        Some(crate::attachment::journal::SteeringRejection::QueueFull)
    );
    owner
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    let duplicate = handle.submit(second).await.unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.record.revision, rejected.record.revision);
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}
