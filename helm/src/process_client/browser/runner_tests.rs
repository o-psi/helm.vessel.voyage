use super::*;

#[test]
fn sharing_requires_explicit_boolean_consent_not_just_agent_mode() {
    for (value, expected) in [
        (
            json!({"mode":"agent","shared":true}),
            BrowserControl::Shared,
        ),
        (
            json!({"mode":"agent","shared":false}),
            BrowserControl::Private,
        ),
        (
            json!({"mode":"agent","shared":"true"}),
            BrowserControl::Private,
        ),
        (json!({"mode":"agent"}), BrowserControl::Private),
        (json!({"mode":"human","shared":true}), BrowserControl::Human),
        (json!({"mode":"human"}), BrowserControl::Human),
        (
            json!({"mode":"private","shared":true}),
            BrowserControl::Private,
        ),
        (
            json!({"mode":"unknown","shared":true}),
            BrowserControl::Private,
        ),
        (json!(null), BrowserControl::Private),
    ] {
        assert_eq!(control(&value), expected);
    }
}

fn binding() -> BrowserBinding {
    BrowserBinding {
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        run_id: Some(Uuid::new_v4()),
        browser_id: Uuid::new_v4(),
        resource_id: Uuid::new_v4(),
        executor_id: Uuid::new_v4(),
        controller_epoch: 2,
        capture_epoch: 2,
        expires_at_ms: now() + 30_000,
    }
}

#[tokio::test]
async fn local_control_fences_validate_epochs_and_explain_private_reasons() {
    let client = Client::local("/synthetic/never-opened".into());
    let (helper, _peer) = super::super::helper::tests::synthetic();
    let root = tempfile::tempdir().unwrap();
    let socket = *client.connection_state().borrow();
    for (reason, expected) in [
        ("remote_cancellation", "cancellation"),
        ("connection_liveness_expired", "liveness"),
        ("binding_expired", "liveness"),
        ("controller_disconnected", "disconnected"),
        ("unknown", "suspended"),
    ] {
        let mut binding = binding();
        let mut offered = false;
        let mut current = BrowserControl::Human;
        let (status, _) = watch::channel(Status::default());
        update_control(
            &client,
            &helper,
            &json!({"mode":"private","epoch":3,"fence_reason":reason}),
            &mut binding,
            &mut offered,
            &mut current,
            &status,
            root.path(),
            socket,
        )
        .await
        .unwrap();
        assert_eq!(binding.controller_epoch, 3);
        assert_eq!(binding.capture_epoch, 3);
        assert_eq!(current, BrowserControl::Private);
        assert!(status.borrow().summary.contains(expected));
        assert!(!offered);
    }
    for value in [
        json!({}),
        json!({"epoch":1}),
        json!({"epoch":3,"capture_epoch":1}),
    ] {
        let mut binding = binding();
        let before = binding.clone();
        let (status, _) = watch::channel(Status::default());
        assert!(
            update_control(
                &client,
                &helper,
                &value,
                &mut binding,
                &mut false,
                &mut BrowserControl::Private,
                &status,
                root.path(),
                socket
            )
            .await
            .is_err()
        );
        assert_eq!(binding, before);
    }
}

#[tokio::test]
async fn dispatch_claim_result_and_cleanup_use_original_identity() {
    use super::super::helper::tests::{respond, synthetic};
    use crate::process_client::loopback_tests::Peer;
    use tokio::io::BufReader;
    for state in [
        BrowserRequestState::Completed,
        BrowserRequestState::Unresolved,
        BrowserRequestState::Refused,
    ] {
        let mut peer = Peer::open().await;
        let (helper, ipc) = synthetic();
        let mut ipc = BufReader::new(ipc);
        let root = tempfile::tempdir().unwrap();
        let binding = binding();
        let request = BrowserRequest {
            request_id: Uuid::new_v4(),
            binding: binding.clone(),
            action: BrowserAction::Inspect { page_id: None },
            action_sha256: "a".repeat(64),
            expires_at_ms: now() + 30_000,
        };
        let c = peer.client.clone();
        let h = helper.clone();
        let b = binding.clone();
        let r = request.clone();
        let path = root.path().to_path_buf();
        let socket = *c.connection_state().borrow();
        let task = tokio::spawn(async move {
            dispatch(c, h, b, r, CancellationToken::new(), path, socket).await
        });
        let (id, command) = peer.command().await;
        let VesselCommand::Voyage(v) = command else {
            panic!("voyage")
        };
        assert!(
            matches!(v.command, VoyageCommand::Browser { operation: BrowserOperation::Claim { request_id, .. } } if request_id == request.request_id)
        );
        let receipt = json!({"kind":"receipt","receipt":{"request_id":request.request_id,"action_sha256":request.action_sha256,"state":"dispatched","cleanup_pending":true}});
        peer.voyage_reply(id, binding.session_id, binding.incarnation, receipt.clone())
            .await;
        let result = json!({"request_id":request.request_id,"action_sha256":request.action_sha256,"state":state,"text":"synthetic observation","page_id":null,"observation_id":null,"image":null,"file":null});
        let envelope = if state == BrowserRequestState::Refused {
            json!({"ok":false})
        } else {
            json!({"ok":true,"cleanup_observed":true,"result":result})
        };
        let action = respond(&helper, &mut ipc, envelope).await;
        assert_eq!(action["id"], request.request_id.to_string());
        let (id, result) = loop {
            let (id, command) = peer.command().await;
            let VesselCommand::Voyage(v) = command else {
                panic!("voyage")
            };
            match v.command {
                VoyageCommand::Browser {
                    operation: BrowserOperation::Receipt { .. },
                } => {
                    peer.voyage_reply(id, binding.session_id, binding.incarnation, receipt.clone())
                        .await
                }
                VoyageCommand::Browser {
                    operation: BrowserOperation::Result { result, .. },
                } => break (id, result),
                _ => panic!("unexpected browser operation"),
            }
        };
        assert_eq!(result.request_id, request.request_id);
        assert_eq!(result.state, state);
        peer.voyage_reply(id, binding.session_id, binding.incarnation, receipt.clone())
            .await;
        if state == BrowserRequestState::Unresolved {
            let (id, command) = peer.command().await;
            let VesselCommand::Voyage(v) = command else {
                panic!("voyage")
            };
            assert!(
                matches!(v.command,VoyageCommand::Browser { operation: BrowserOperation::Cleanup { observed:true,request_id,.. } } if request_id == request.request_id)
            );
            peer.voyage_reply(id, binding.session_id, binding.incarnation, receipt)
                .await;
        }
        task.await.unwrap().unwrap();
        let evidence: Value = serde_json::from_slice(
            &std::fs::read(
                root.path()
                    .join(format!("action-{}.json", request.request_id)),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(evidence["local_dispatch_possible"], true);
        assert!(evidence.get("action").is_none());
        assert!(evidence.get("text").is_none());
    }
}

#[tokio::test]
async fn rejected_claim_records_positive_no_dispatch_cleanup() {
    use crate::process_client::loopback_tests::Peer;
    let mut peer = Peer::open().await;
    let (helper, _ipc) = super::super::helper::tests::synthetic();
    let root = tempfile::tempdir().unwrap();
    let binding = binding();
    let request = BrowserRequest {
        request_id: Uuid::new_v4(),
        binding: binding.clone(),
        action: BrowserAction::Inspect { page_id: None },
        action_sha256: "b".repeat(64),
        expires_at_ms: now() + 30_000,
    };
    let c = peer.client.clone();
    let b = binding.clone();
    let r = request.clone();
    let path = root.path().to_path_buf();
    let socket = *c.connection_state().borrow();
    let task = tokio::spawn(async move {
        dispatch(c, helper, b, r, CancellationToken::new(), path, socket).await
    });
    let receipt = json!({"kind":"receipt","receipt":{"request_id":request.request_id,"action_sha256":request.action_sha256,"state":"refused","cleanup_pending":false}});
    let (id, _) = peer.command().await;
    peer.voyage_reply(id, binding.session_id, binding.incarnation, receipt.clone())
        .await;
    let (id, command) = peer.command().await;
    assert!(
        matches!(command,VesselCommand::Voyage(v) if matches!(v.command,VoyageCommand::Browser {operation:BrowserOperation::Cleanup {observed:true,..}}))
    );
    peer.voyage_reply(id, binding.session_id, binding.incarnation, receipt)
        .await;
    task.await.unwrap().unwrap();
    let evidence: Value = serde_json::from_slice(
        &std::fs::read(
            root.path()
                .join(format!("action-{}.json", request.request_id)),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(evidence["local_dispatch_possible"], false);
}

#[tokio::test]
async fn dispatch_rejects_each_stale_binding_before_journaling_or_network() {
    let client = Client::local("/synthetic/not-opened".into());
    let root = tempfile::tempdir().unwrap();
    let expected = binding();
    for field in [
        "session_id",
        "incarnation",
        "browser_id",
        "resource_id",
        "executor_id",
        "run_id",
        "controller_epoch",
        "capture_epoch",
        "expires_at_ms",
    ] {
        let mut b = expected.clone();
        let mut expiry = now() + 30_000;
        match field {
            "session_id" => b.session_id = Uuid::new_v4(),
            "incarnation" => b.incarnation = Uuid::new_v4(),
            "browser_id" => b.browser_id = Uuid::new_v4(),
            "resource_id" => b.resource_id = Uuid::new_v4(),
            "executor_id" => b.executor_id = Uuid::new_v4(),
            "run_id" => b.run_id = None,
            "controller_epoch" => b.controller_epoch += 1,
            "capture_epoch" => b.capture_epoch += 1,
            _ => expiry = 0,
        }
        let request = BrowserRequest {
            request_id: Uuid::new_v4(),
            binding: b,
            action: BrowserAction::Inspect { page_id: None },
            action_sha256: "a".repeat(64),
            expires_at_ms: expiry,
        };
        let (helper, _ipc) = super::super::helper::tests::synthetic();
        assert!(
            dispatch(
                client.clone(),
                helper,
                expected.clone(),
                request,
                CancellationToken::new(),
                root.path().into(),
                *client.connection_state().borrow()
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("Stale")
        );
    }
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn explicit_sharing_prepares_owner_then_heartbeats_before_offer_and_takeover() {
    use super::super::helper::tests::{respond, synthetic};
    use crate::process_client::loopback_tests::Peer;
    use tokio::io::BufReader;
    let mut peer = Peer::open().await;
    let (helper, ipc) = synthetic();
    let mut ipc = BufReader::new(ipc);
    let root = tempfile::tempdir().unwrap();
    let initial = binding();
    let c = peer.client.clone();
    let h = helper.clone();
    let mut b = initial.clone();
    let path = root.path().to_owned();
    let socket = *c.connection_state().borrow();
    let (status, observed) = watch::channel(Status::default());
    let task = tokio::spawn(async move {
        let mut offered = false;
        let mut current = BrowserControl::Private;
        update_control(
            &c,
            &h,
            &json!({"mode":"agent","shared":true,"epoch":3,"capture_epoch":4}),
            &mut b,
            &mut offered,
            &mut current,
            &status,
            &path,
            socket,
        )
        .await
        .unwrap();
        assert!(offered);
        assert_eq!(current, BrowserControl::Shared);
        update_control(
            &c,
            &h,
            &json!({"mode":"human","epoch":4,"capture_epoch":5}),
            &mut b,
            &mut offered,
            &mut current,
            &status,
            &path,
            socket,
        )
        .await
        .unwrap();
        assert_eq!(current, BrowserControl::Human);
        b
    });
    let (id, command) = peer.command().await;
    assert!(
        matches!(command,VesselCommand::Voyage(v) if matches!(v.command,VoyageCommand::PrepareBrowser))
    );
    peer.voyage_reply(id, initial.session_id, initial.incarnation, json!({}))
        .await;
    let heartbeat = respond(
        &helper,
        &mut ipc,
        json!({"ok":true,"result":{"mode":"agent","shared":true,"epoch":3}}),
    )
    .await;
    assert_eq!(heartbeat["op"], "heartbeat");
    assert_eq!(heartbeat["binding"]["capture_epoch"], 4);
    let (id, command) = peer.command().await;
    let VesselCommand::Voyage(v) = command else {
        panic!("voyage")
    };
    let VoyageCommand::Browser {
        operation: BrowserOperation::Offer { binding, .. },
    } = v.command
    else {
        panic!("offer")
    };
    assert_eq!(binding.controller_epoch, 3);
    let reply = json!({"kind":"status","status":{"binding":binding,"control":"shared","available":true,"cleanup_pending":false}});
    peer.voyage_reply(id, initial.session_id, initial.incarnation, reply.clone())
        .await;
    let (id, command) = peer.command().await;
    assert!(
        matches!(command,VesselCommand::Voyage(v) if matches!(v.command,VoyageCommand::Browser {operation:BrowserOperation::Control {control:BrowserControl::Human,..}}))
    );
    peer.voyage_reply(id, initial.session_id, initial.incarnation, reply)
        .await;
    let final_binding = task.await.unwrap();
    assert_eq!(final_binding.capture_epoch, 5);
    assert!(observed.borrow().summary.contains("Human control"));
    let journal: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("binding.json")).unwrap()).unwrap();
    assert_eq!(
        journal["binding"]["session_id"],
        initial.session_id.to_string()
    );
}
