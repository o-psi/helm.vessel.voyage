use super::*;
use crate::process_client::loopback_tests::{Peer, response, send};
use serde_json::json;
use voyage_protocol::{duplex::ServerFrame, vessel::*};
fn process() -> ProcessInfo {
    serde_json::from_value(json!({"session_id":Uuid::new_v4(),"incarnation":Uuid::new_v4(),"workspace":"/synthetic","state":"live"})).unwrap()
}

#[tokio::test]
async fn no_save_retains_active_cleanup_and_unversioned_voyages() {
    for snapshot in [
        json!({"run":{"state":"accepted"}}),
        json!({"run":{"state":"running"}}),
        json!({"run":{"state":"awaiting_decision"}}),
        json!({"run":{"state":"cancel_requested"}}),
        json!({"pending_cleanup_run":Uuid::new_v4()}),
        json!({"run":{"state":"completed"}}),
    ] {
        let mut peer = Peer::open().await;
        let process = process();
        let c = peer.client.clone();
        let p = process.clone();
        let task = tokio::spawn(async move { discard(&c, &p).await });
        let (id, command) = peer.command().await;
        assert!(matches!(
            command,
            VesselCommand::Voyage(VoyageRequest {
                command: VoyageCommand::Snapshot,
                ..
            })
        ));
        peer.voyage_reply(id, process.session_id, process.incarnation, snapshot)
            .await;
        assert!(task.await.unwrap().is_err());
        assert!(peer.client.connection_state().borrow().socket_id.is_some());
    }
}

#[tokio::test]
async fn no_save_delete_refusal_is_not_retried() {
    let mut peer = Peer::open().await;
    let process = process();
    let c = peer.client.clone();
    let p = process.clone();
    let task = tokio::spawn(async move { discard(&c, &p).await });
    let (id, _) = peer.command().await;
    peer.voyage_reply(
        id,
        process.session_id,
        process.incarnation,
        json!({"revision":8,"run":{"state":"completed"}}),
    )
    .await;
    let (id, command) = peer.command().await;
    assert!(matches!(
        command,
        VesselCommand::Voyage(VoyageRequest {
            command: VoyageCommand::Delete {
                expected_revision: 8,
                ..
            },
            ..
        })
    ));
    send(
        &mut peer.socket,
        ServerFrame::Reply {
            request_id: id,
            response: VesselResponse {
                error: Some("explicit refusal".into()),
                ..response(json!(null))
            },
        },
    )
    .await;
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .downcast_ref::<crate::process_client::transport::Refusal>()
            .is_some()
    );
}

#[tokio::test]
async fn no_save_observes_exact_deletion_receipt_without_stop_or_replay() {
    let mut peer = Peer::open().await;
    let process = process();
    let c = peer.client.clone();
    let p = process.clone();
    let task = tokio::spawn(async move { discard(&c, &p).await });
    let (id, _) = peer.command().await;
    peer.voyage_reply(
        id,
        process.session_id,
        process.incarnation,
        json!({"revision":8}),
    )
    .await;
    let (id, command) = peer.command().await;
    let VesselCommand::Voyage(VoyageRequest {
        command:
            VoyageCommand::Delete {
                command_id,
                expected_revision: 8,
                ..
            },
        ..
    }) = command
    else {
        panic!("delete")
    };
    peer.voyage_reply(
        id,
        process.session_id,
        process.incarnation,
        json!({"accepted":true}),
    )
    .await;
    for valid in [false, true] {
        let (id, command) = peer.command().await;
        assert!(
            matches!(command,VesselCommand::Inspect { session_id } if session_id == process.session_id)
        );
        let mut p = process.clone();
        p.state = ProcessState::Stopped;
        p.deletion = Some(
            json!({"command_id":if valid {command_id} else {Uuid::new_v4()},"status":"applied","deleted":true,"cleanup":"observed"}),
        );
        peer.reply(id, serde_json::to_value(p).unwrap()).await;
    }
    task.await.unwrap().unwrap();
    assert!(deadline().unwrap() > chrono::Utc::now().timestamp_millis() as u64);
}

#[tokio::test]
async fn resume_catalogue_reuses_owner_and_only_restarts_stopped_owner() {
    for state in [
        ProcessState::Live,
        ProcessState::Suspended,
        ProcessState::Stopped,
        ProcessState::Unavailable,
        ProcessState::CleanupUnconfirmed,
        ProcessState::Relinquished,
    ] {
        let mut peer = Peer::open().await;
        let mut process = process();
        process.state = state.clone();
        let c = peer.client.clone();
        let reference = process.session_id.to_string();
        let task = tokio::spawn(async move {
            resume::open(&c, &crate::Config::default(), None, &reference).await
        });
        let (id, command) = peer.command().await;
        assert!(matches!(command, VesselCommand::Catalogue));
        peer.reply(id, json!([process.clone()])).await;
        if state == ProcessState::Stopped {
            let (id, command) = peer.command().await;
            assert!(
                matches!(command,VesselCommand::Restart {session_id,incarnation,..} if session_id==process.session_id && incarnation==process.incarnation)
            );
            process.state = ProcessState::Live;
            peer.reply(id, serde_json::to_value(&process).unwrap())
                .await;
        }
        let result = task.await.unwrap();
        assert_eq!(
            result.is_ok(),
            matches!(
                state,
                ProcessState::Live | ProcessState::Suspended | ProcessState::Stopped
            )
        );
        if let Ok(found) = result {
            assert_eq!(found.session_id, process.session_id);
        }
    }
}

#[tokio::test]
async fn resume_name_ambiguity_and_workspace_mismatch_do_not_start_owners() {
    let mut peer = Peer::open().await;
    let a = process();
    let b = process();
    let c = peer.client.clone();
    let task = tokio::spawn(async move {
        resume::open(&c, &crate::Config::default(), None, "same-name").await
    });
    let (id, _) = peer.command().await;
    peer.reply(id, json!([a.clone(), b.clone()])).await;
    for p in [a, b] {
        let (id, _) = peer.command().await;
        peer.voyage_reply(id, p.session_id, p.incarnation, json!({"name":"same-name"}))
            .await;
    }
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    let p = process();
    let root = tempfile::tempdir().unwrap();
    let c = peer.client.clone();
    let reference = p.session_id.to_string();
    let workspace = root.path().to_path_buf();
    let task = tokio::spawn(async move {
        resume::open(&c, &crate::Config::default(), Some(workspace), &reference).await
    });
    let (id, _) = peer.command().await;
    peer.reply(id, json!([p])).await;
    assert!(
        task.await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("workspace differs")
    );
}
