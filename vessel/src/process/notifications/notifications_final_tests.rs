use super::*;
use crate::process::{database, test_support::Fixture};
use tokio::net::UnixListener;
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

async fn setup() -> (Fixture, Supervisor, ProcessRegistration, Destination) {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut r = f.registration();
    r.state = ProcessState::Live;
    database::save(&f.0, &r).await.unwrap();
    registry::private_directory(&registry::directory(&f.0, r.session_id)).unwrap();
    let vessel = crate::process::identity::public(&f.0).unwrap().vessel_id;
    let d = Destination {
        id: Uuid::new_v4(),
        recipient_grant_id: vessel,
        recipient_principal_id: vessel,
        recipient_grant_revision: 1,
        source_vessel_id: vessel,
        source_session_id: r.session_id,
        event_kinds: vec![
            NotificationKind::Budget,
            NotificationKind::Attention,
            NotificationKind::Test,
            NotificationKind::Completed,
        ],
        expires_at_ms: access::now().unwrap() + 60_000,
        notification_ttl_ms: 60_000,
        quiet_hours_utc: None,
    };
    for op in [
        NotificationOperation::Configure {
            command_id: Uuid::new_v4(),
            destination: d.clone(),
        },
        NotificationOperation::Accept {
            command_id: Uuid::new_v4(),
            destination_id: d.id,
        },
    ] {
        s.notifications(op, None).await.unwrap();
    }
    (f, s, r, d)
}
fn peer(
    f: &Fixture,
    r: &ProcessRegistration,
    result: Value,
) -> tokio::task::JoinHandle<RuntimeRequest> {
    let path = registry::directory(&f.0, r.session_id).join("runtime.sock");
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(path).unwrap();
    let r = r.clone();
    tokio::spawn(async move {
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
        let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
        write_frame(
            &mut stream,
            &RuntimeResponse {
                protocol: PROCESS_PROTOCOL,
                session_id: r.session_id,
                incarnation: r.incarnation,
                resumed_from: None,
                result,
                error: None,
                outcome_unknown: false,
            },
        )
        .await
        .unwrap();
        request
    })
}
#[tokio::test]
async fn unsupported_budget_protocol_records_unavailability_and_bounded_backoff() {
    let (f, s, r, d) = setup().await;
    let path =
        f.0.join("notifications")
            .join(format!("budget-delivery-{}.json", d.id));
    assert!(s.budget_delivery_status(d.id).is_null());
    // This protocol version has no budget_events command. Never fabricate totals
    // or fall back to a different owner RPC when decoding the command fails.
    assert!(
        serde_json::from_value::<voyage_protocol::process::RuntimeCommand>(
            json!({"op":"budget_events","after":0,"limit":64})
        )
        .is_err()
    );
    s.deliver_budget_notifications(d.id).await.unwrap();
    let a: BudgetDeliveryAttempt = access::load(&path).unwrap();
    assert_eq!(a.after, 0);
    assert_eq!(a.state.failures, 1);
    let store = store::Store::open_current(f.0.join("notifications")).unwrap();
    assert_eq!(
        store.cursor(d.id).unwrap().error,
        Some(ProducerError::Unavailable)
    );
    s.deliver_budget_notifications(d.id).await.unwrap();
    assert_eq!(s.budget_delivery_status(d.id)["attempts"], 1);
    for state in [
        DeliveryAttempt {
            failures: 5,
            ..Default::default()
        },
        DeliveryAttempt {
            drained_incarnation: Some(r.incarnation),
            ..Default::default()
        },
    ] {
        access::save(&path, &BudgetDeliveryAttempt { after: 7, state }).unwrap();
        s.deliver_budget_notifications(d.id).await.unwrap();
        assert_eq!(s.budget_delivery_status(d.id)["after"], 7);
    }
    std::fs::write(&path, b"broken").unwrap();
    assert_eq!(s.budget_delivery_status(d.id)["state_unavailable"], true);
    s.deliver_budget_notifications(Uuid::new_v4())
        .await
        .unwrap();
}

fn publish(
    f: &Fixture,
    r: &ProcessRegistration,
    d: &Destination,
    decision: Option<Uuid>,
    incarnation: Option<Uuid>,
) -> Notification {
    let now = access::now().unwrap();
    let id = Uuid::new_v4();
    let n = Notification {
        event_id: id,
        source_event_id: id,
        session_id: r.session_id,
        run_id: Some(Uuid::new_v4()),
        incarnation,
        kind: if decision.is_some() {
            NotificationKind::Attention
        } else {
            NotificationKind::Completed
        },
        created_at_ms: now,
        expires_at_ms: now + 30_000,
        decision_id: decision,
        budget: None,
    };
    let mut store = store::Store::open_current(f.0.join("notifications")).unwrap();
    store.authorize(&f.0, d, ProcessRight::Observe, r);
    store.publish(d.id, n.clone(), now).unwrap();
    n
}
#[tokio::test]
async fn open_revalidates_decision_but_never_grants_approval() {
    let (f, s, r, d) = setup().await;
    let missing = s
        .notifications(
            NotificationOperation::Open {
                destination_id: d.id,
                event_id: Uuid::new_v4(),
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(missing["status"], "expired_or_revoked");
    let plain = publish(&f, &r, &d, None, Some(r.incarnation));
    let plain = s
        .notifications(
            NotificationOperation::Open {
                destination_id: d.id,
                event_id: plain.event_id,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(plain["status"], "current");
    assert_eq!(plain["execution_cleanup"], "not_implied");
    for mode in 0..3 {
        let decision = Uuid::new_v4();
        let n = publish(&f, &r, &d, Some(decision), Some(r.incarnation));
        let result = if mode == 0 {
            json!([])
        } else {
            json!([{"decision_id":decision,"incarnation":r.incarnation,"run_id":n.run_id,"expires_at_ms":if mode==1 {0} else {access::now().unwrap()+30_000}}])
        };
        let server = peer(&f, &r, result);
        let value = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            s.notifications(
                NotificationOperation::Open {
                    destination_id: d.id,
                    event_id: n.event_id,
                },
                None,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
        assert_eq!(
            value["status"],
            ["resolved_or_expired", "stale", "current"][mode]
        );
        assert_eq!(value["actionable"], false);
        if mode == 2 {
            assert_eq!(value["approval_granted"], false);
            assert_eq!(
                value["decision_requires"],
                "separate_explicit_owner_response"
            );
        }
    }
    let receipt = s
        .notifications(
            NotificationOperation::Receipt {
                destination_id: d.id,
                event_id: plain["notification"]["event_id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                state: ReceiptState::Seen,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(receipt["state"], "seen");
    s.notifications(
        NotificationOperation::Revoke {
            command_id: Uuid::new_v4(),
            destination_id: d.id,
        },
        None,
    )
    .await
    .unwrap();
    let inbox = s
        .notifications(
            NotificationOperation::Inbox {
                destination_id: d.id,
                after: 0,
                limit: 10,
            },
            None,
        )
        .await;
    assert!(
        inbox.is_err()
            || inbox.unwrap()["page"]["entries"]
                .as_array()
                .is_some_and(Vec::is_empty)
    );
}
