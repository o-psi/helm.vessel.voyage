use super::*;
use crate::process::{database, test_support::Fixture};
use tokio::net::UnixListener;
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

async fn configured(accepted: bool) -> (Fixture, Supervisor, ProcessRegistration, Destination) {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut r = f.registration();
    r.state = ProcessState::Live;
    database::save(&f.0, &r).await.unwrap();
    registry::private_directory(&registry::directory(&f.0, r.session_id)).unwrap();
    let vessel = super::super::identity::public(&f.0).unwrap().vessel_id;
    let d = Destination {
        id: Uuid::new_v4(),
        recipient_grant_id: vessel,
        recipient_principal_id: vessel,
        recipient_grant_revision: 1,
        source_vessel_id: vessel,
        source_session_id: r.session_id,
        event_kinds: vec![NotificationKind::Attention, NotificationKind::Test],
        expires_at_ms: access::now().unwrap() + 60_000,
        notification_ttl_ms: 60_000,
        quiet_hours_utc: None,
    };
    s.notifications(
        NotificationOperation::Configure {
            command_id: Uuid::new_v4(),
            destination: d.clone(),
        },
        None,
    )
    .await
    .unwrap();
    if accepted {
        s.notifications(
            NotificationOperation::Accept {
                command_id: Uuid::new_v4(),
                destination_id: d.id,
            },
            None,
        )
        .await
        .unwrap();
    }
    (f, s, r, d)
}

fn event(r: &ProcessRegistration) -> Value {
    json!({"sequence":1,"source_event_id":Uuid::new_v4(),"session_id":r.session_id,
        "run_id":Uuid::new_v4(),"incarnation":r.incarnation,"kind":"attention",
        "created_at_ms":access::now().unwrap(),"expires_at_ms":null,"decision_id":Uuid::new_v4()})
}
fn page(events: Vec<Value>, next: u64) -> Value {
    json!({"events":events,"next_after":next,"has_more":false,"gap":false})
}
async fn deliver(
    f: &Fixture,
    s: &Supervisor,
    r: &ProcessRegistration,
    d: &Destination,
    result: Value,
) {
    let path = registry::directory(&f.0, r.session_id).join("runtime.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let registration = r.clone();
    let peer = tokio::spawn(async move {
        let (mut stream, _) =
            tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                .await
                .unwrap()
                .unwrap();
        let request: RuntimeRequest = read_frame(&mut stream).await.unwrap();
        assert_eq!(request.session_id, registration.session_id);
        assert!(request.authorization.is_none());
        assert!(matches!(
            request.command,
            voyage_protocol::process::RuntimeCommand::NotificationEvents { limit: 64, .. }
        ));
        write_frame(
            &mut stream,
            &RuntimeResponse {
                protocol: PROCESS_PROTOCOL,
                session_id: registration.session_id,
                incarnation: registration.incarnation,
                resumed_from: None,
                error: None,
                outcome_unknown: false,
                result,
            },
        )
        .await
        .unwrap();
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        s.deliver_notifications(d.id),
    )
    .await
    .unwrap()
    .unwrap();
    peer.await.unwrap();
    std::fs::remove_file(path).unwrap();
}
fn cursor(f: &Fixture, d: &Destination) -> voyage_protocol::notifications::ProducerCursor {
    store::Store::open_current(f.0.join("notifications"))
        .unwrap()
        .cursor(d.id)
        .unwrap()
}
async fn inbox(s: &Supervisor, d: &Destination) -> Value {
    s.notifications(
        NotificationOperation::Inbox {
            destination_id: d.id,
            after: 0,
            limit: 64,
        },
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn delivery_publishes_metadata_and_advances_durable_cursor() {
    let (f, s, r, d) = configured(true).await;
    let e = event(&r);
    deliver(&f, &s, &r, &d, page(vec![e.clone()], 1)).await;
    assert_eq!(cursor(&f, &d).after, 1);
    assert_eq!(cursor(&f, &d).error, None);
    let entries = inbox(&s, &d).await;
    assert_eq!(entries["page"]["entries"].as_array().unwrap().len(), 1);
    assert_eq!(
        entries["page"]["entries"][0]["notification"]["event_id"],
        e["source_event_id"]
    );
    let attempt: DeliveryAttempt = access::load(
        &f.0.join("notifications")
            .join(format!("delivery-{}.json", d.id)),
    )
    .unwrap();
    assert_eq!(attempt.failures, 0);
    assert_eq!(attempt.next_attempt_ms, 0);
}

macro_rules! invalid_event {
    ($name:ident,$field:literal,$value:expr) => {
        #[tokio::test]
        async fn $name() {
            let (f, s, r, d) = configured(true).await;
            let mut e = event(&r);
            e[$field] = $value;
            deliver(&f, &s, &r, &d, page(vec![e], 1)).await;
            assert_eq!(cursor(&f, &d).after, 0);
            assert_eq!(cursor(&f, &d).error, Some(ProducerError::InvalidSource));
            assert!(
                inbox(&s, &d).await["page"]["entries"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
    };
}
invalid_event!(rejects_zero_sequence, "sequence", json!(0));
invalid_event!(rejects_sequence_past_page, "sequence", json!(2));
invalid_event!(rejects_foreign_session, "session_id", json!(Uuid::new_v4()));
invalid_event!(
    rejects_nil_source_event,
    "source_event_id",
    json!(Uuid::nil())
);
invalid_event!(rejects_nil_run, "run_id", json!(Uuid::nil()));
invalid_event!(rejects_future_timestamp, "created_at_ms", json!(u64::MAX));
invalid_event!(rejects_runtime_test_event, "kind", json!("test"));
invalid_event!(rejects_runtime_budget_event, "kind", json!("budget"));
invalid_event!(
    rejects_attention_without_decision,
    "decision_id",
    Value::Null
);
invalid_event!(
    rejects_nonattention_with_decision,
    "kind",
    json!("completed")
);

#[tokio::test]
async fn malformed_page_is_unavailable_not_an_empty_success() {
    let (f, s, r, d) = configured(true).await;
    deliver(&f, &s, &r, &d, json!({"events":[]})).await;
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::Unavailable));
    assert_eq!(cursor(&f, &d).after, 0);
}
#[tokio::test]
async fn oversized_page_is_rejected_before_publishing() {
    let (f, s, r, d) = configured(true).await;
    deliver(&f, &s, &r, &d, page(vec![event(&r); 65], 65)).await;
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::InvalidSource));
    assert!(
        inbox(&s, &d).await["page"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn gap_survives_successful_empty_followup() {
    let (f, s, r, d) = configured(true).await;
    let mut p = page(vec![], 3);
    p["gap"] = json!(true);
    deliver(&f, &s, &r, &d, p).await;
    deliver(&f, &s, &r, &d, page(vec![], 4)).await;
    assert_eq!(cursor(&f, &d).after, 4);
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::Gap));
}
#[tokio::test]
async fn regressing_cursor_does_not_erase_previous_progress() {
    let (f, s, r, d) = configured(true).await;
    deliver(&f, &s, &r, &d, page(vec![], 5)).await;
    deliver(&f, &s, &r, &d, page(vec![], 4)).await;
    assert_eq!(cursor(&f, &d).after, 5);
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::InvalidSource));
}
#[tokio::test]
async fn expired_event_is_skipped_but_cursor_advances() {
    let (f, s, r, d) = configured(true).await;
    let mut e = event(&r);
    e["expires_at_ms"] = json!(1);
    deliver(&f, &s, &r, &d, page(vec![e], 1)).await;
    assert_eq!(cursor(&f, &d).after, 1);
    assert_eq!(cursor(&f, &d).error, None);
    assert!(
        inbox(&s, &d).await["page"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[tokio::test]
async fn unavailable_runtime_records_backoff_and_does_not_retry_immediately() {
    let (f, s, _, d) = configured(true).await;
    s.deliver_notifications(d.id).await.unwrap();
    let path =
        f.0.join("notifications")
            .join(format!("delivery-{}.json", d.id));
    let first = std::fs::read(&path).unwrap();
    let a: DeliveryAttempt = access::load(&path).unwrap();
    assert_eq!(a.failures, 1);
    assert!(a.next_attempt_ms > access::now().unwrap());
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::Unavailable));
    s.deliver_notifications(d.id).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), first);
}
#[tokio::test]
async fn unaccepted_destination_never_attempts_delivery() {
    let (f, s, _, d) = configured(false).await;
    s.deliver_notifications(d.id).await.unwrap();
    assert!(
        !f.0.join("notifications")
            .join(format!("delivery-{}.json", d.id))
            .exists()
    );
    assert_eq!(cursor(&f, &d).error, None);
}
#[tokio::test]
async fn test_receipts_remove_attention_without_implying_cleanup() {
    let (_f, s, _, d) = configured(true).await;
    let command_id = Uuid::new_v4();
    let receipt = s
        .notifications(
            NotificationOperation::Test {
                command_id,
                destination_id: d.id,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        s.notifications(
            NotificationOperation::Test {
                command_id,
                destination_id: d.id
            },
            None
        )
        .await
        .unwrap(),
        receipt
    );
    assert_eq!(
        s.notifications(NotificationOperation::Attention, None)
            .await
            .unwrap()["available"],
        1
    );
    let event_id = serde_json::from_value(receipt["event_id"].clone()).unwrap();
    s.notifications(
        NotificationOperation::Receipt {
            destination_id: d.id,
            event_id,
            state: ReceiptState::Dismissed,
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        s.notifications(NotificationOperation::Attention, None)
            .await
            .unwrap()["available"],
        0
    );
    let revoked = s
        .notifications(
            NotificationOperation::Revoke {
                command_id: Uuid::new_v4(),
                destination_id: d.id,
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(revoked["execution_cleanup"], "not_implied");
}

#[tokio::test]
async fn events_before_recipient_acceptance_are_not_backfilled() {
    let (f, s, r, d) = configured(true).await;
    let mut e = event(&r);
    e["created_at_ms"] = json!(0);
    deliver(&f, &s, &r, &d, page(vec![e], 1)).await;
    assert_eq!(cursor(&f, &d).after, 1);
    assert_eq!(cursor(&f, &d).error, None);
    assert!(
        inbox(&s, &d).await["page"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn unsubscribed_kind_is_filtered_without_stalling_source_cursor() {
    let (f, s, r, d) = configured(true).await;
    let mut e = event(&r);
    e["kind"] = json!("completed");
    e["decision_id"] = Value::Null;
    deliver(&f, &s, &r, &d, page(vec![e], 1)).await;
    assert_eq!(cursor(&f, &d).after, 1);
    assert_eq!(cursor(&f, &d).error, None);
    assert!(
        inbox(&s, &d).await["page"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn revoked_destination_stops_before_ipc_and_hides_inbox() {
    let (f, s, _, d) = configured(true).await;
    s.notifications(
        NotificationOperation::Revoke {
            command_id: Uuid::new_v4(),
            destination_id: d.id,
        },
        None,
    )
    .await
    .unwrap();
    s.deliver_notifications(d.id).await.unwrap();
    assert!(
        !f.0.join("notifications")
            .join(format!("delivery-{}.json", d.id))
            .exists()
    );
    assert!(
        inbox(&s, &d).await["page"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn relinquished_source_is_unavailable_without_attempting_ipc() {
    let (f, s, mut r, d) = configured(true).await;
    r.state = ProcessState::Relinquished;
    database::save(&f.0, &r).await.unwrap();
    s.deliver_notifications(d.id).await.unwrap();
    assert_eq!(cursor(&f, &d).error, Some(ProducerError::Unavailable));
    assert!(
        !f.0.join("notifications")
            .join(format!("delivery-{}.json", d.id))
            .exists()
    );
}
