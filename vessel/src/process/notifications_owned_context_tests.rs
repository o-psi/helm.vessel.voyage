//! Owned metadata/IPC fault boundaries, separate from native courier journeys.
use super::*;
use crate::process::{database, test_support::Fixture};
use std::sync::Arc;
use tokio::net::UnixListener;
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

async fn setup() -> (Fixture, Arc<Supervisor>, ProcessRegistration, Destination) {
    let f = Fixture::new();
    let supervisor = Arc::new(f.supervisor().await);
    let mut registration = f.registration();
    registration.state = ProcessState::Live;
    database::save(&f.0, &registration).await.unwrap();
    registry::private_directory(&registry::directory(&f.0, registration.session_id)).unwrap();
    let vessel = crate::process::identity::public(&f.0).unwrap().vessel_id;
    let destination = Destination {
        id: Uuid::new_v4(),
        recipient_grant_id: vessel,
        recipient_principal_id: vessel,
        recipient_grant_revision: 1,
        source_vessel_id: vessel,
        source_session_id: registration.session_id,
        event_kinds: vec![
            NotificationKind::Completed,
            NotificationKind::Attention,
            NotificationKind::Test,
        ],
        expires_at_ms: access::now().unwrap() + 60000,
        notification_ttl_ms: 30000,
        quiet_hours_utc: None,
    };
    supervisor
        .notifications(
            NotificationOperation::Configure {
                command_id: Uuid::new_v4(),
                destination: destination.clone(),
            },
            None,
        )
        .await
        .unwrap();
    supervisor
        .notifications(
            NotificationOperation::Accept {
                command_id: Uuid::new_v4(),
                destination_id: destination.id,
            },
            None,
        )
        .await
        .unwrap();
    (f, supervisor, registration, destination)
}
fn source(registration: &ProcessRegistration, sequence: u64) -> Value {
    json!({"sequence":sequence,"source_event_id":Uuid::new_v4(),"session_id":registration.session_id,"run_id":Uuid::new_v4(),"incarnation":registration.incarnation,"kind":"completed","created_at_ms":access::now().unwrap(),"expires_at_ms":null,"decision_id":null})
}
fn page(events: Vec<Value>, after: u64) -> Value {
    json!({"events":events,"next_after":after,"has_more":false,"gap":false})
}
async fn peer(
    f: &Fixture,
    s: &Arc<Supervisor>,
    registration: &ProcessRegistration,
    destination: &Destination,
    value: Value,
    change: &str,
) {
    let path = registry::directory(&f.0, registration.session_id).join("runtime.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let producer = s.clone();
    let id = destination.id;
    let delivery = tokio::spawn(async move { producer.deliver_notifications(id).await });
    let (mut stream, _) =
        tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
            .await
            .unwrap()
            .unwrap();
    let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
    assert_eq!(request.session_id, registration.session_id);
    assert_eq!(request.incarnation, registration.incarnation);
    assert!(request.authorization.is_none());
    assert!(matches!(
        request.command,
        voyage_protocol::process::RuntimeCommand::NotificationEvents { limit: 64, .. }
    ));
    // Mutate only explicit owned fixture metadata after the actual IPC request,
    // proving that routing-time approval cannot authorize the later publication.
    match change {
        "revoke" => {
            store::Store::open_current(f.0.join("notifications"))
                .unwrap()
                .revoke(Uuid::new_v4(), destination.id, access::now().unwrap())
                .unwrap();
        }
        "owner" => {
            let mut next = registration.clone();
            next.incarnation = Uuid::new_v4();
            next.restart_from = Some(registration.incarnation);
            database::save(&f.0, &next).await.unwrap();
        }
        "relinquished" => {
            let mut next = registration.clone();
            next.state = ProcessState::Relinquished;
            database::save(&f.0, &next).await.unwrap();
        }
        "identity" => {
            let path = f.0.join("identity/key.json");
            let mut key: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            key["vessel_id"] = json!(Uuid::new_v4());
            std::fs::write(path, serde_json::to_vec(&key).unwrap()).unwrap();
        }
        _ => (),
    }
    write_frame(
        &mut stream,
        &RuntimeResponse {
            protocol: PROCESS_PROTOCOL,
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            resumed_from: None,
            result: value,
            error: matches!(change, "refused" | "unknown")
                .then(|| "owned metadata unavailable".into()),
            outcome_unknown: change == "unknown",
        },
    )
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), delivery)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(stream);
    drop(listener);
    std::fs::remove_file(path).unwrap();
}
fn cursor(f: &Fixture, d: &Destination) -> ProducerCursor {
    store::Store::open_current(f.0.join("notifications"))
        .unwrap()
        .cursor(d.id)
        .unwrap()
}
fn entries(f: &Fixture, d: &Destination) -> Vec<InboxEntry> {
    store::Store::open_current(f.0.join("notifications"))
        .unwrap()
        .inbox(d.id, 0, 100, access::now().unwrap())
        .unwrap()
        .entries
}

#[tokio::test]
async fn delivery_rechecks_destination_source_and_owner_after_actual_metadata_ipc() {
    for change in [
        "revoke",
        "owner",
        "relinquished",
        "identity",
        "refused",
        "unknown",
    ] {
        let (f, s, r, d) = setup().await;
        let event = source(&r, 1);
        peer(&f, &s, &r, &d, page(vec![event], 1), change).await;
        assert_eq!(cursor(&f, &d).after, 0);
        assert_eq!(cursor(&f, &d).error, Some(ProducerError::Unavailable));
        assert!(entries(&f, &d).is_empty());
        let attempt: DeliveryAttempt = access::load(
            &f.0.join("notifications")
                .join(format!("delivery-{}.json", d.id)),
        )
        .unwrap();
        assert_eq!(attempt.failures, 1);
        assert!(attempt.next_attempt_ms > 0);
        assert!(attempt.drained_incarnation.is_none());
    }
}

#[tokio::test]
async fn page_failure_keeps_exact_earlier_metadata_without_advancing_or_republishing() {
    let (f, s, r, d) = setup().await;
    let first = source(&r, 1);
    let mut bad = source(&r, 2);
    bad["session_id"] = json!(Uuid::new_v4());
    peer(&f, &s, &r, &d, page(vec![first.clone(), bad], 2), "").await;
    assert_eq!(cursor(&f, &d).after, 0);
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::InvalidSource));
    let retained = entries(&f, &d);
    assert_eq!(retained.len(), 1);
    assert_eq!(
        retained[0].notification.event_id.to_string(),
        first["source_event_id"]
    );
    let path =
        f.0.join("notifications")
            .join(format!("delivery-{}.json", d.id));
    let mut attempt: DeliveryAttempt = access::load(&path).unwrap();
    attempt.next_attempt_ms = 0;
    access::save(&path, &attempt).unwrap();
    let second = source(&r, 2);
    peer(&f, &s, &r, &d, page(vec![first, second], 2), "").await;
    let all = entries(&f, &d);
    assert_eq!(all.len(), 2);
    assert_eq!(all[0], retained[0]);
    assert_eq!(
        cursor(&f, &d),
        ProducerCursor {
            after: 2,
            error: None
        }
    );
}

#[tokio::test]
async fn changed_source_fingerprint_is_unavailable_and_never_repaired_or_relabelled() {
    let (f, s, r, d) = setup().await;
    let first = source(&r, 1);
    peer(&f, &s, &r, &d, page(vec![first.clone()], 1), "").await;
    let retained = entries(&f, &d);
    let mut changed = first;
    changed["sequence"] = json!(2);
    changed["run_id"] = json!(Uuid::new_v4());
    peer(&f, &s, &r, &d, page(vec![changed], 2), "").await;
    assert_eq!(entries(&f, &d), retained);
    assert_eq!(
        cursor(&f, &d),
        ProducerCursor {
            after: 1,
            error: Some(ProducerError::Unavailable)
        }
    );
}

#[tokio::test]
async fn exhausted_corrupt_and_future_attempts_have_no_ipc_or_fake_drained_proof() {
    for case in 0..4 {
        let (f, s, r, d) = setup().await;
        let path =
            f.0.join("notifications")
                .join(format!("delivery-{}.json", d.id));
        if case == 0 {
            std::fs::write(&path, b"owned-corrupt-attempt").unwrap();
        } else {
            let mut attempt = DeliveryAttempt::default();
            match case {
                1 => attempt.failures = 5,
                2 => attempt.next_attempt_ms = access::now().unwrap() + 60000,
                _ => attempt.drained_incarnation = Some(r.incarnation),
            };
            access::save(&path, &attempt).unwrap();
        }
        let before = std::fs::read(&path).unwrap();
        let result = s.deliver_notifications(d.id).await;
        assert_eq!(result.is_err(), case == 0);
        assert_eq!(std::fs::read(path).unwrap(), before);
        assert_eq!(
            cursor(&f, &d),
            ProducerCursor {
                after: 0,
                error: None
            }
        );
        assert!(entries(&f, &d).is_empty());
        assert!(
            !registry::directory(&f.0, r.session_id)
                .join("runtime.sock")
                .exists()
        );
    }
}

#[tokio::test]
async fn recipient_binding_and_command_collisions_refuse_without_new_inbox_effects() {
    let (f, s, r, d) = setup().await;
    let test_id = Uuid::new_v4();
    let test = NotificationOperation::Test {
        command_id: test_id,
        destination_id: d.id,
    };
    let receipt = s.notifications(test.clone(), None).await.unwrap();
    for case in 0..3 {
        let mut actor = GrantBinding {
            grant_id: d.recipient_grant_id,
            principal_id: d.recipient_principal_id,
            revision: d.recipient_grant_revision,
        };
        match case {
            0 => actor.grant_id = Uuid::new_v4(),
            1 => actor.principal_id = Uuid::new_v4(),
            _ => actor.revision += 1,
        };
        for op in [
            NotificationOperation::Inbox {
                destination_id: d.id,
                after: 0,
                limit: 100,
            },
            NotificationOperation::Receipt {
                destination_id: d.id,
                event_id: test_id,
                state: ReceiptState::Seen,
            },
            NotificationOperation::Open {
                destination_id: d.id,
                event_id: test_id,
            },
            NotificationOperation::Revoke {
                command_id: Uuid::new_v4(),
                destination_id: d.id,
            },
        ] {
            assert!(s.notifications(op, Some(actor.clone())).await.is_err());
        }
        assert_eq!(entries(&f, &d).len(), 1);
        assert_eq!(entries(&f, &d)[0].receipt.state, ReceiptState::Available);
    }
    assert!(
        s.notifications(
            NotificationOperation::Accept {
                command_id: test_id,
                destination_id: d.id
            },
            None
        )
        .await
        .is_err()
    );
    assert_eq!(s.notifications(test, None).await.unwrap(), receipt);
    assert_eq!(entries(&f, &d).len(), 1);
    assert_eq!(
        cursor(&f, &d),
        ProducerCursor {
            after: 0,
            error: None
        }
    );
    assert!(
        !registry::directory(&f.0, r.session_id)
            .join("runtime.sock")
            .exists()
    );
}
