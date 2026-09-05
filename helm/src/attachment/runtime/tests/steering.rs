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
        .enable_steering(Arc::new(move |_, _, _| {
            anyhow::ensure!(observed.load(Ordering::SeqCst), "actor revoked");
            Ok(())
        }))
        .unwrap();
    let guidance = request(&owner, "fresh authorization").await;
    assert!(handle.submit(guidance.clone(), 1).await.is_err());
    allowed.store(true, Ordering::SeqCst);
    handle.submit(guidance, 1).await.unwrap();
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
        let handle = owner.enable_steering(Arc::new(allow_actor)).unwrap();
        let guidance = request(&owner, "must not dispatch").await;
        let db = fixture_database(dir.path().join("attachment/journal.sqlite3")).unwrap();
        if rejecting {
            drop(owner.steering_receiver.take());
            db.execute_batch("CREATE TRIGGER fail_reject BEFORE UPDATE ON steering WHEN NEW.status='not_applied' BEGIN SELECT RAISE(ABORT,'reject unavailable'); END;").unwrap();
        } else {
            db.execute_batch("CREATE TRIGGER fail_queue BEFORE INSERT ON steering BEGIN SELECT RAISE(ABORT,'queue unavailable'); END;").unwrap();
        }
        assert!(handle.submit(guidance.clone(), 1).await.is_err());
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
