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

#[test]
fn administration_json_accepts_exact_byte_limit_and_rejects_bad_inputs() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("input.json");
    let payload = format!("\"{}\"", "x".repeat(65534));
    std::fs::write(&file, payload).unwrap();
    assert_eq!(read_json::<String>(&file).unwrap().len(), 65534);
    for payload in [
        vec![b'x'; 65537],
        b"{".to_vec(),
        vec![0xff],
        b"true false".to_vec(),
    ] {
        std::fs::write(&file, payload).unwrap();
        assert!(read_json::<serde_json::Value>(&file).is_err());
    }
    assert!(read_json::<serde_json::Value>(root.path()).is_err());
    assert!(read_json::<serde_json::Value>(&root.path().join("missing")).is_err());
}

#[tokio::test]
async fn assignment_uses_observed_owner_and_forwards_exact_identifiers() {
    for cancel in [false, true] {
        let mut peer = Peer::open().await;
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let run = Uuid::new_v4();
        let assignment = Uuid::new_v4();
        let c = peer.client.clone();
        let task = tokio::spawn(async move {
            execute(
                &c,
                AdminCommand::Assignment {
                    session,
                    run,
                    assignment,
                    participant: "worker-α".into(),
                    cancel,
                },
            )
            .await
        });
        let (id, command) = peer.command().await;
        assert!(matches!(command, VesselCommand::Inspect { session_id } if session_id == session));
        peer.reply(id, json!({"session_id":session,"incarnation":incarnation,"workspace":"/synthetic","state":"live"})).await;
        let (id, command) = peer.command().await;
        assert!(
            matches!(command, VesselCommand::Voyage(VoyageRequest { session_id, incarnation: None, command: VoyageCommand::AssignmentObserve { run_id, assignment_id, participant, cancel: actual }, .. }) if session_id == session && run_id == run && assignment_id == assignment && participant == "worker-α" && actual == cancel)
        );
        peer.voyage_reply(id, session, incarnation, json!({"observed":true}))
            .await;
        assert_eq!(task.await.unwrap().unwrap(), json!({"observed":true}));
    }
}

#[tokio::test]
async fn malformed_admin_files_fail_without_connecting() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("malformed.json");
    std::fs::write(&file, "{}").unwrap();
    let client = Client::local(root.path().join("no-vessel"));
    for command in [
        AdminCommand::Trust {
            identity_file: file.clone(),
        },
        AdminCommand::AcceptParticipant {
            binding_file: file,
            command_id: Uuid::new_v4(),
        },
    ] {
        let error = execute(&client, command).await.unwrap_err().to_string();
        assert!(error.contains("missing field"), "{error}");
        assert!(client.connection_state().borrow().socket_id.is_none());
    }
}
