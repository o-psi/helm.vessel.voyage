use super::*;
#[path = "../../dispatch_fixture_final_tests.rs"]
// Each consumer gets an independent scripted fixture module.
#[allow(clippy::duplicate_mod)]
mod fixture;
use fixture::*;
use serde_json::json;

#[test]
fn administrative_files_are_bounded_typed_and_not_directories() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.json");
    for text in ["null", "{}", "[1,2]", "true", "\"public\""] {
        std::fs::write(&path, text).unwrap();
        assert_eq!(
            read_json::<Value>(&path).unwrap(),
            serde_json::from_str::<Value>(text).unwrap()
        );
    }
    std::fs::write(&path, "{").unwrap();
    assert!(read_json::<Value>(&path).is_err());
    std::fs::write(&path, "x".repeat(65537)).unwrap();
    assert!(
        read_json::<Value>(&path)
            .unwrap_err()
            .to_string()
            .contains("limit")
    );
    std::fs::write(&path, format!("{}0", " ".repeat(65535))).unwrap();
    assert_eq!(read_json::<Value>(&path).unwrap(), json!(0));
    assert!(read_json::<Value>(root.path()).is_err());
    assert!(read_json::<Value>(&root.path().join("missing")).is_err());
}

#[tokio::test]
async fn malformed_trust_and_participant_files_never_connect() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("invalid.json");
    std::fs::write(&path, "{}").unwrap();
    let client = Client::local(root.path().join("absent"));
    assert!(
        execute(
            &client,
            AdminCommand::Trust {
                identity_file: path.clone()
            }
        )
        .await
        .is_err()
    );
    assert!(
        execute(
            &client,
            AdminCommand::AcceptParticipant {
                binding_file: path,
                command_id: uuid::Uuid::new_v4()
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn recover_optional_attestations_and_participant_drain_stay_explicit() {
    let s = uuid::Uuid::new_v4();
    let i = uuid::Uuid::new_v4();
    let c = uuid::Uuid::new_v4();
    for cancel in [false, true] {
        let peer = Peer::new(vec![(
            wire(VesselCommand::RemoveParticipant {
                binding_id: s,
                command_id: c,
                expected_revision: 4,
                cancel,
            }),
            json!({"cancel":cancel}),
        )])
        .await;
        assert_eq!(
            execute(
                &peer.client,
                AdminCommand::RemoveParticipant {
                    binding_id: s,
                    command_id: c,
                    expected_revision: 4,
                    cancel
                }
            )
            .await
            .unwrap()["cancel"],
            cancel
        );
        peer.finish().await;
    }
    let peer = Peer::new(vec![(
        wire(VesselCommand::Recover {
            session_id: s,
            incarnation: i,
            command_id: c,
            acknowledge_cleanup: None,
            reconcile_tools: None,
            expected_revision: None,
            acknowledge_resources: vec![],
        }),
        json!({"observed":true}),
    )])
    .await;
    assert_eq!(
        execute(
            &peer.client,
            AdminCommand::Recover {
                session: s,
                incarnation: i,
                command_id: c,
                acknowledge_cleanup: None,
                reconcile_tools: None,
                expected_revision: None,
                acknowledge_resources: vec![]
            }
        )
        .await
        .unwrap()["observed"],
        true
    );
    peer.finish().await;
}
