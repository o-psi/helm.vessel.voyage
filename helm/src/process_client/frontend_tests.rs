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

#[tokio::test]
async fn invalid_prompts_fail_before_any_owner_or_workspace_access() {
    for (prompt, expected) in [
        (" \t\r\n".to_owned(), "blank"),
        ("x".repeat(65537), "64 KiB"),
        ("界".repeat(21846), "64 KiB"),
    ] {
        for no_save in [false, true] {
            let result = run(
                crate::Config::default(),
                Some("/missing/synthetic/workspace".into()),
                Some("unresolved-owner".into()),
                prompt.clone(),
                no_save,
                true,
                true,
            )
            .await;
            assert!(result.unwrap_err().to_string().contains(expected));
        }
    }
}

#[tokio::test]
async fn resume_name_skips_failed_snapshots_and_accepts_canonical_workspace() {
    let mut peer = Peer::open().await;
    let root = tempfile::tempdir().unwrap();
    let mut selected = process();
    selected.workspace = root.path().canonicalize().unwrap();
    let other = process();
    let c = peer.client.clone();
    let workspace = root.path().join(".");
    let task = tokio::spawn(async move {
        resume::open(&c, &crate::Config::default(), Some(workspace), "wanted").await
    });
    let (id, _) = peer.command().await;
    peer.reply(id, json!([other, selected.clone()])).await;
    let (id, _) = peer.command().await;
    send(
        &mut peer.socket,
        ServerFrame::Reply {
            request_id: id,
            response: VesselResponse {
                error: Some("owner unavailable".into()),
                ..response(json!(null))
            },
        },
    )
    .await;
    let (id, _) = peer.command().await;
    peer.voyage_reply(
        id,
        selected.session_id,
        selected.incarnation,
        json!({"name":"wanted"}),
    )
    .await;
    assert_eq!(task.await.unwrap().unwrap().session_id, selected.session_id);
}

#[tokio::test]
async fn resume_rejects_malformed_catalogue_and_restart_reply() {
    let mut peer = Peer::open().await;
    let c = peer.client.clone();
    let task =
        tokio::spawn(
            async move { resume::open(&c, &crate::Config::default(), None, "fixture").await },
        );
    let (id, _) = peer.command().await;
    peer.reply(id, json!({"not":"a catalogue"})).await;
    assert!(task.await.unwrap().is_err());
    let mut p = process();
    p.state = ProcessState::Stopped;
    let reference = p.session_id.to_string();
    let c = peer.client.clone();
    let task =
        tokio::spawn(
            async move { resume::open(&c, &crate::Config::default(), None, &reference).await },
        );
    let (id, _) = peer.command().await;
    peer.reply(id, json!([p])).await;
    let (id, command) = peer.command().await;
    assert!(matches!(command, VesselCommand::Restart { .. }));
    peer.reply(id, json!(null)).await;
    assert!(task.await.unwrap().is_err());
}

#[tokio::test]
async fn fresh_connected_open_captures_private_config_and_never_submits_a_run() {
    let root = tempfile::tempdir().unwrap();
    let mut peer = Peer::open().await;
    let mut client = peer.client.clone();
    client.directory = root.path().into();
    let workspace = root.path().canonicalize().unwrap();
    let expected = workspace.clone();
    let task = tokio::spawn(async move {
        open_connected(
            &crate::Config::default(),
            Some(workspace),
            None,
            false,
            false,
            false,
            client,
        )
        .await
    });
    let (id, command) = peer.command().await;
    let VesselCommand::StartConfigured {
        command_id,
        session_id,
        workspace,
        config_path,
    } = command
    else {
        panic!("fresh configured start expected")
    };
    assert!(!command_id.is_nil());
    assert_eq!(workspace, expected);
    assert!(config_path.is_file());
    let stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
    assert_eq!(
        stored["workspace"],
        serde_json::to_value(&expected).unwrap()
    );
    let incarnation = Uuid::new_v4();
    peer.reply(id,json!({"session_id":session_id,"incarnation":incarnation,"workspace":expected,"state":"live"})).await;
    let (_, process) = task.await.unwrap().unwrap();
    assert_eq!(process.session_id, session_id);
    assert_eq!(process.incarnation, incarnation);
}
#[tokio::test]
async fn connected_resume_rejects_active_overrides_without_configuration_or_new_owner() {
    for state in [
        "accepted",
        "running",
        "awaiting_decision",
        "cancel_requested",
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut peer = Peer::open().await;
        let mut client = peer.client.clone();
        client.directory = root.path().into();
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let workspace = root.path().canonicalize().unwrap();
        let reference = session.to_string();
        let task = tokio::spawn(async move {
            open_connected(
                &crate::Config::default(),
                None,
                Some(reference),
                true,
                true,
                false,
                client,
            )
            .await
        });
        let (id, command) = peer.command().await;
        assert!(matches!(command, VesselCommand::Catalogue));
        peer.reply(id,json!([{"session_id":session,"incarnation":incarnation,"workspace":workspace,"state":"live"}])).await;
        let (id, command) = peer.command().await;
        assert!(matches!(command, VesselCommand::Voyage(_)));
        peer.voyage_reply(
            id,
            session,
            incarnation,
            json!({"revision":7,"model":"old","run":{"state":state}}),
        )
        .await;
        assert!(task.await.unwrap().is_err());
        assert!(!root.path().join("launch").exists());
    }
}
#[tokio::test]
async fn connected_idle_resume_retains_saved_model_until_explicit_override_and_pins_revision() {
    for overridden in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let mut peer = Peer::open().await;
        let mut client = peer.client.clone();
        client.directory = root.path().into();
        let session = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let workspace = root.path().canonicalize().unwrap();
        let reference = session.to_string();
        let mut config = crate::Config::default();
        config.model = "explicit-model".into();
        let task = tokio::spawn(async move {
            open_connected(
                &config,
                None,
                Some(reference),
                overridden,
                true,
                false,
                client,
            )
            .await
        });
        let (id, _) = peer.command().await;
        peer.reply(id,json!([{"session_id":session,"incarnation":incarnation,"workspace":workspace,"state":"live"}])).await;
        let (id, _) = peer.command().await;
        peer.voyage_reply(
            id,
            session,
            incarnation,
            json!({"revision":17,"model":"saved-model","run":{"state":"completed"}}),
        )
        .await;
        let (id, command) = peer.command().await;
        let VesselCommand::Voyage(VoyageRequest {
            session_id,
            incarnation: sent,
            command:
                VoyageCommand::Configure {
                    expected_revision,
                    config_path,
                    ..
                },
            ..
        }) = command
        else {
            panic!("configure expected")
        };
        assert_eq!(session_id, session);
        assert_eq!(sent, Some(incarnation));
        assert_eq!(expected_revision, 17);
        let stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
        assert_eq!(
            stored["config"]["model"],
            if overridden {
                "explicit-model"
            } else {
                "saved-model"
            }
        );
        peer.voyage_reply(
            id,
            session,
            incarnation,
            json!({"status":"applied","revision":18}),
        )
        .await;
        assert_eq!(task.await.unwrap().unwrap().1.session_id, session);
    }
}
#[tokio::test]
async fn connected_resume_without_overrides_preserves_live_process_and_does_not_read_configuration()
{
    let root = tempfile::tempdir().unwrap();
    let mut peer = Peer::open().await;
    let mut client = peer.client.clone();
    client.directory = root.path().into();
    let session = Uuid::new_v4();
    let inc = Uuid::new_v4();
    let reference = session.to_string();
    let task = tokio::spawn(async move {
        open_connected(
            &crate::Config::default(),
            None,
            Some(reference),
            false,
            false,
            false,
            client,
        )
        .await
    });
    let (id, command) = peer.command().await;
    assert!(matches!(command, VesselCommand::Catalogue));
    peer.reply(
        id,
        json!([{"session_id":session,"incarnation":inc,"workspace":root.path(),"state":"live"}]),
    )
    .await;
    let (_, process) = task.await.unwrap().unwrap();
    assert_eq!(process.incarnation, inc);
    assert!(!root.path().join("launch").exists());
}
