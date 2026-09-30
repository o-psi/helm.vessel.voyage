use super::*;
use serde_json::json;

#[test]
fn cursor_progress_is_required_only_for_nonterminal_pages() {
    assert_eq!(
        next_cursor(&json!({"next_after":7,"has_more":false}), 7).unwrap(),
        7
    );
    assert_eq!(
        next_cursor(&json!({"next_after":8,"has_more":true}), 7).unwrap(),
        8
    );
    for page in [
        json!({}),
        json!({"next_after":7}),
        json!({"next_after":7,"has_more":true}),
        json!({"next_after":6,"has_more":false}),
        json!({"next_after":8,"has_more":"true"}),
        json!({"next_after":-1,"has_more":false}),
    ] {
        assert!(next_cursor(&page, 7).is_err());
    }
}

#[test]
fn typed_destination_file_is_bounded_and_error_does_not_quote_secrets() {
    let value = json!({"id":Uuid::new_v4(),"recipient_grant_id":Uuid::new_v4(),"recipient_principal_id":Uuid::new_v4(),"recipient_grant_revision":1,"source_vessel_id":Uuid::new_v4(),"source_session_id":Uuid::new_v4(),"event_kinds":[],"expires_at_ms":1000,"notification_ttl_ms":100,"quiet_hours_utc":null});
    let bytes = serde_json::to_vec(&value).unwrap();
    let destination = decode_destination(&bytes).unwrap();
    assert_eq!(serde_json::to_value(&destination).unwrap(), value);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("destination.json");
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(load_destination(&path).unwrap(), destination);
    assert!(load_destination(temp.path()).is_err());
    assert!(load_destination(&temp.path().join("absent")).is_err());
    for bytes in [
        b"secret-fixture-not-json".to_vec(),
        vec![b'x'; 65_537],
        b"{}".to_vec(),
    ] {
        let error = decode_destination(&bytes).unwrap_err();
        assert!(!error.to_string().contains("secret-fixture"));
    }
    std::fs::write(path.clone(), vec![b'x'; 65_537]).unwrap();
    assert!(load_destination(&path).is_err());
}

#[cfg(unix)]
#[path = "../dispatch_fixture_final_tests.rs"]
#[allow(clippy::duplicate_mod)]
mod fixture;

#[cfg(unix)]
#[tokio::test]
async fn explicit_inbox_operations_preserve_scope_and_exact_mutation_identity() {
    use fixture::{Peer, wire};
    let destination = Uuid::new_v4();
    let event = Uuid::new_v4();
    let command_id = Uuid::new_v4();
    let cases = [
        (
            InboxCommand::Accept {
                destination,
                command_id,
            },
            NotificationOperation::Accept {
                destination_id: destination,
                command_id,
            },
        ),
        (
            InboxCommand::Revoke {
                destination,
                command_id,
            },
            NotificationOperation::Revoke {
                destination_id: destination,
                command_id,
            },
        ),
        (
            InboxCommand::Test {
                destination,
                command_id,
            },
            NotificationOperation::Test {
                destination_id: destination,
                command_id,
            },
        ),
        (
            InboxCommand::Destinations,
            NotificationOperation::Destinations,
        ),
        (
            InboxCommand::List {
                destination,
                after: 41,
                limit: 100,
            },
            NotificationOperation::Inbox {
                destination_id: destination,
                after: 41,
                limit: 100,
            },
        ),
        (
            InboxCommand::Inspect { destination, event },
            NotificationOperation::Open {
                destination_id: destination,
                event_id: event,
            },
        ),
        (
            InboxCommand::Seen { destination, event },
            NotificationOperation::Receipt {
                destination_id: destination,
                event_id: event,
                state: ReceiptState::Seen,
            },
        ),
        (
            InboxCommand::Dismiss { destination, event },
            NotificationOperation::Receipt {
                destination_id: destination,
                event_id: event,
                state: ReceiptState::Dismissed,
            },
        ),
    ];
    for (command, operation) in cases {
        let receipt = json!({"destination_id":destination,"event_id":event,"status":"stored"});
        let peer = Peer::new(vec![(
            wire(VesselCommand::Notifications { operation }),
            receipt.clone(),
        )])
        .await;
        assert_eq!(execute(&peer.client, command).await.unwrap(), receipt);
        // Any extra navigation, decision response or cancellation fails the fixture.
        peer.finish().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn configure_reads_typed_local_destination_without_adding_message_content() {
    use fixture::{Peer, wire};
    let value = json!({"id":Uuid::new_v4(),"recipient_grant_id":Uuid::new_v4(),"recipient_principal_id":Uuid::new_v4(),"recipient_grant_revision":7,"source_vessel_id":Uuid::new_v4(),"source_session_id":Uuid::new_v4(),"event_kinds":[],"expires_at_ms":1000,"notification_ttl_ms":100,"quiet_hours_utc":null});
    let destination = decode_destination(&serde_json::to_vec(&value).unwrap()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("typed.json");
    std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
    let command_id = Uuid::new_v4();
    let receipt = json!({"status":"configured","command_id":command_id});
    let peer = Peer::new(vec![(
        wire(VesselCommand::Notifications {
            operation: NotificationOperation::Configure {
                command_id,
                destination,
            },
        }),
        receipt.clone(),
    )])
    .await;
    assert_eq!(
        execute(&peer.client, InboxCommand::Configure { file, command_id })
            .await
            .unwrap(),
        receipt
    );
    peer.finish().await;
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_watch_pages_stop_without_receipts_or_owner_mutations() {
    use fixture::{Peer, wire};
    for page in [
        json!({}),
        json!({"next_after":8,"has_more":true}),
        json!({"next_after":7,"has_more":false}),
        json!({"next_after":9,"has_more":"yes"}),
    ] {
        let destination = Uuid::new_v4();
        let peer = Peer::new(vec![(
            wire(VesselCommand::Notifications {
                operation: NotificationOperation::Inbox {
                    destination_id: destination,
                    after: 8,
                    limit: 100,
                },
            }),
            json!({"page":page}),
        )])
        .await;
        assert!(watch(&peer.client, destination, 8, 1).await.is_err());
        peer.finish().await;
    }
}

#[tokio::test]
async fn invalid_inbox_dispatch_is_refused_before_connection_or_local_file_input() {
    let root = tempfile::tempdir().unwrap();
    let client = Client::local(root.path().to_path_buf());
    let destination = Uuid::new_v4();
    for limit in [0, 101, u32::MAX] {
        let error = execute(
            &client,
            InboxCommand::List {
                destination,
                after: 0,
                limit,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.to_string(), "limit must be 1..100");
    }
    for seconds in [0, 3601, u64::MAX] {
        let error = watch(&client, destination, 0, seconds).await.unwrap_err();
        assert_eq!(error.to_string(), "watch duration must be 1..3600 seconds");
    }
    assert_eq!(
        execute(
            &client,
            InboxCommand::Watch {
                destination,
                after: 0,
                seconds: 1
            }
        )
        .await
        .unwrap_err()
        .to_string(),
        "watch requires the streaming CLI dispatcher"
    );
    let file = root.path().join("invalid-destination.json");
    std::fs::write(&file, "private-synthetic-not-a-destination").unwrap();
    let error = execute(
        &client,
        InboxCommand::Configure {
            file,
            command_id: Uuid::new_v4(),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "invalid typed destination JSON");
    assert!(!root.path().join("process-http.json").exists());
}
