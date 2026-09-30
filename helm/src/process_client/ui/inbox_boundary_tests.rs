use super::*;
use serde_json::json;

fn notification(event: Uuid, run: Uuid, incarnation: Uuid, decision: Uuid) -> Value {
    json!({"event_id":event,"session_id":Uuid::new_v4(),"run_id":run,"incarnation":incarnation,"kind":"attention","source_event_id":Uuid::new_v4(),"created_at_ms":1,"expires_at_ms":100,"decision_id":decision})
}

#[test]
fn current_owner_decision_requires_exact_event_run_incarnation_and_decision() {
    let event = Uuid::new_v4();
    let run = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let decision = Uuid::new_v4();
    let operation = NotificationOperation::Open {
        destination_id: Uuid::new_v4(),
        event_id: event,
    };
    let value = json!({"status":"current","notification":notification(event,run,incarnation,decision),"decision":{"decision_id":decision,"run_id":run,"incarnation":incarnation,"expires_at_ms":50,"request":{"kind":"question","text":"private-untrusted-request"}},"url":"https://untrusted.invalid/"});
    let text = render(&operation, value.clone()).unwrap();
    assert!(text.contains(&format!("Authorized owner decision: {decision}")));
    assert!(text.contains("No response sent"));
    assert!(text.contains("Execution cleanup is not implied"));
    assert!(!text.contains("private-untrusted-request"));
    assert!(!text.contains("untrusted.invalid"));
    for (object, field) in [
        ("notification", "event_id"),
        ("decision", "decision_id"),
        ("decision", "run_id"),
        ("decision", "incarnation"),
    ] {
        let mut changed = value.clone();
        changed[object][field] = json!(Uuid::new_v4());
        assert!(
            render(&operation, changed).is_err(),
            "changed {object}.{field}"
        );
    }
    let mut absent = value.clone();
    absent["notification"] = Value::Null;
    assert!(render(&operation, absent).is_err());
    let mut secret = value;
    secret["notification"]["summary"] = json!("private-model-content");
    assert!(render(&operation, secret).is_err());
}

#[test]
fn unavailable_owner_states_never_render_unsolicited_decisions_as_authority() {
    let operation = NotificationOperation::Open {
        destination_id: Uuid::new_v4(),
        event_id: Uuid::new_v4(),
    };
    for status in [
        "resolved_or_expired",
        "stale",
        "expired_or_revoked",
        "decision_authority_unavailable",
        "unavailable",
    ] {
        let text=render(&operation,json!({"status":status,"decision":{"unexpected":"private-untrusted-request"},"summary":"private-summary"})).unwrap();
        assert!(text.contains("No response sent"));
        assert!(!text.contains("Authorized owner decision"));
        assert!(!text.contains("private-"));
    }
}

#[test]
fn destination_receipts_cannot_be_attributed_to_a_different_event() {
    let event = Uuid::new_v4();
    for state in [ReceiptState::Seen, ReceiptState::Dismissed] {
        let operation = NotificationOperation::Receipt {
            destination_id: Uuid::new_v4(),
            event_id: event,
            state,
        };
        let text = render(
            &operation,
            json!({"sequence":8,"event_id":event,"state":state}),
        )
        .unwrap();
        assert!(text.contains(&event.to_string()));
        assert!(text.contains("not owner resolution or proof of human reading"));
        assert!(
            render(
                &operation,
                json!({"sequence":8,"event_id":Uuid::new_v4(),"state":state})
            )
            .is_err()
        );
        assert!(
            render(
                &operation,
                json!({"sequence":8,"event_id":event,"state":state,"private_text":"untrusted"})
            )
            .is_err()
        );
    }
}

#[test]
fn paged_notification_metadata_requires_correlated_receipts_and_bounded_entries() {
    let event = Uuid::new_v4();
    let destination = Uuid::new_v4();
    let operation = NotificationOperation::Inbox {
        destination_id: destination,
        after: 0,
        limit: 50,
    };
    let entry = json!({"receipt":{"sequence":9,"event_id":event,"state":"available"},"notification":notification(event,Uuid::new_v4(),Uuid::new_v4(),Uuid::new_v4())});
    let value = json!({"page":{"entries":[entry.clone()],"next_after":9,"has_more":true},"producer":{"after":9,"error":"capacity"},"attention_deferred":true,"destination_expires_at_ms":100,"budget_delivery":{"pending_or_unavailable":true}});
    let text = render(&operation, value.clone()).unwrap();
    assert!(text.contains("Capacity"));
    assert!(text.contains("not task success"));
    assert!(text.contains("receipt is not established"));
    assert!(text.contains("Reading does not mark seen"));
    assert!(text.contains(&format!("/inbox list {destination} 9")));
    let mut wrong = value.clone();
    wrong["page"]["entries"][0]["receipt"]["event_id"] = json!(Uuid::new_v4());
    assert!(render(&operation, wrong).is_err());
    let mut oversized = value.clone();
    oversized["page"]["entries"] = json!(vec![entry; 101]);
    assert!(render(&operation, oversized).is_err());
    for field in [
        "producer",
        "attention_deferred",
        "destination_expires_at_ms",
    ] {
        let mut absent = value.clone();
        absent.as_object_mut().unwrap().remove(field);
        assert!(render(&operation, absent).is_err());
    }
}

#[test]
fn destination_inventory_is_bounded_and_never_includes_server_free_text() {
    let record = json!({"destination":{"id":Uuid::new_v4(),"recipient_grant_id":Uuid::new_v4(),"recipient_principal_id":Uuid::new_v4(),"recipient_grant_revision":1,"source_vessel_id":Uuid::new_v4(),"source_session_id":Uuid::new_v4(),"event_kinds":["attention"],"expires_at_ms":100,"notification_ttl_ms":10,"quiet_hours_utc":null},"revoked_at_ms":null,"accepted_at_ms":5});
    let text=render(&NotificationOperation::Destinations,json!({"destinations":[record.clone()],"local_recipient_id":Uuid::new_v4(),"summary":"private-untrusted"})).unwrap();
    assert!(text.contains("Accepted: true"));
    assert!(text.contains("Revoked: false"));
    assert!(!text.contains("private-untrusted"));
    assert!(
        render(
            &NotificationOperation::Destinations,
            json!({"destinations":vec![record;257]})
        )
        .is_err()
    );
    assert!(
        render(
            &NotificationOperation::Destinations,
            json!({"destinations":[],"local_recipient_id":"private-untrusted"})
        )
        .is_err()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn attention_probe_preserves_unknown_instead_of_fabricating_zero() {
    use super::super::socket_support_tests::Server;
    use voyage_protocol::vessel::VesselCommand;
    for (response, expected) in [
        (json!({"available":0}), Some(0)),
        (json!({"available":16384}), Some(16384)),
        (json!({"available":16385}), None),
        (json!({"available":-1}), None),
        (json!({"available":"0"}), None),
        (json!({}), None),
    ] {
        let mut server = Server::new(move |command| {
            assert!(matches!(
                command,
                VesselCommand::Notifications {
                    operation: NotificationOperation::Attention
                }
            ));
            Ok(response.clone())
        })
        .await;
        assert_eq!(attention(&server.client).await, expected);
        assert!(matches!(
            server.requests.recv().await.unwrap(),
            VesselCommand::Notifications {
                operation: NotificationOperation::Attention
            }
        ));
        assert!(server.requests.try_recv().is_err());
    }
}
