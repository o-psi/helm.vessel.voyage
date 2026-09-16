use super::*;
use crate::process_client::loopback_tests::Peer;
use serde_json::json;
use uuid::Uuid;

#[test]
fn administrative_json_is_bounded_typed_and_never_overwrites_input() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("input.json");
    for bytes in [
        b"{\"fixture\":true}".to_vec(),
        b"invalid".to_vec(),
        vec![b' '; 65537],
    ] {
        std::fs::write(&path, &bytes).unwrap();
        let result = read_json::<serde_json::Value>(&path);
        assert_eq!(result.is_ok(), bytes.starts_with(b"{"));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    assert!(read_json::<serde_json::Value>(root.path()).is_err());
    assert!(read_json::<serde_json::Value>(&root.path().join("missing")).is_err());
}

#[tokio::test]
async fn admin_recovery_and_removal_preserve_explicit_command_authority() {
    let mut peer = Peer::open().await;
    let binding = Uuid::new_v4();
    let command = Uuid::new_v4();
    let c = peer.client.clone();
    let task = tokio::spawn(async move {
        execute(
            &c,
            AdminCommand::RemoveParticipant {
                binding_id: binding,
                command_id: command,
                expected_revision: 42,
                cancel: true,
            },
        )
        .await
    });
    let (id, request) = peer.command().await;
    assert!(
        matches!(request,VesselCommand::RemoveParticipant { binding_id,command_id,expected_revision:42,cancel:true } if binding_id==binding && command_id==command)
    );
    peer.reply(id, json!({"removed":true})).await;
    assert_eq!(task.await.unwrap().unwrap(), json!({"removed":true}));
    let session = Uuid::new_v4();
    let incarnation = Uuid::new_v4();
    let command = Uuid::new_v4();
    let resource = Uuid::new_v4();
    let c = peer.client.clone();
    let task = tokio::spawn(async move {
        execute(
            &c,
            AdminCommand::Recover {
                session,
                incarnation,
                command_id: command,
                acknowledge_cleanup: None,
                reconcile_tools: None,
                expected_revision: Some(9),
                acknowledge_resources: vec![resource],
            },
        )
        .await
    });
    let (id, request) = peer.command().await;
    let VesselCommand::Recover {
        session_id,
        command_id,
        acknowledge_resources,
        expected_revision,
        ..
    } = request
    else {
        panic!("recover")
    };
    assert_eq!(session_id, session);
    assert_eq!(command_id, command);
    assert_eq!(acknowledge_resources, vec![resource]);
    assert_eq!(expected_revision, Some(9));
    peer.reply(id, json!({"recovered":true})).await;
    assert_eq!(task.await.unwrap().unwrap(), json!({"recovered":true}));
}
