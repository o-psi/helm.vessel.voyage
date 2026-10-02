//! Ordinary UI review journeys over the owned socket; no administrator launch.
use super::super::{coverage_support, socket_support_tests::Server};
use super::*;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

fn connect(app: &mut App, old: Target, server: &Server) -> Target {
    let route = app.clients.insert(server.client.clone());
    let target = Target {
        route,
        session: server.target.session,
    };
    let mut view = app.views.remove(&old).unwrap();
    view.process.session_id = target.session;
    view.process.incarnation = server.incarnation;
    view.snapshot.as_mut().unwrap().session_id = target.session;
    app.views.insert(target, view);
    app.selected = Some(target);
    target
}
async fn next(receiver: &mut mpsc::Receiver<Update>) -> Update {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn sent(server: &mut Server) -> ExecutionOperation {
    let command = tokio::time::timeout(Duration::from_secs(3), server.requests.recv())
        .await
        .unwrap()
        .unwrap();
    let voyage_protocol::vessel::VesselCommand::Execution { operation } = command else {
        panic!("only public execution review operations")
    };
    operation
}
fn saved(target: Target, incarnation: Uuid, ids: [Uuid; 3], state: &str, change: &str) -> Value {
    let vessel = Uuid::new_v4();
    let run = Uuid::new_v4();
    let facts = json!({"vessel_id":vessel,"session_id":ids[2],"run_id":run,"incarnation":incarnation,"requester_id":Uuid::new_v4(),"connection_id":target.route.id,"connection_revision":1,"administrative_owner_id":Uuid::new_v4(),"authority_revision":1,"expected_session_revision":17,"change":change,"previous_incarnation":null,"identity":{"id":Uuid::new_v4(),"revision":2},"account_context":{"id":Uuid::new_v4(),"revision":3},"account":{"account_id":Uuid::new_v4(),"connection_id":Uuid::new_v4(),"identity_generation":1,"connection_revision":1,"transport":"openai_responses"},"account_capability_revision":1,"workspace":"/synthetic-workspace","host_identity_digest":"a".repeat(64),"policy_digest":"b".repeat(64),"release_digest":"c".repeat(64),"pending_work_digest":"d".repeat(64)});
    let outcome = match state {
        "unconfirmed" => json!({"state":state,"cleanup_obligations":[Uuid::new_v4()]}),
        "revocation_requested" => json!({"state":state,"administrator_grant_id":Uuid::new_v4()}),
        _ => json!({"state":state}),
    };
    json!({"review":{"schema":1,"review_id":ids[0],"command_id":ids[1],"created_at_ms":1,"expires_at_ms":u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap()+60000,"facts":facts,"digest":"e".repeat(64)},"receipt":{"schema":1,"vessel_id":vessel,"session_id":ids[2],"run_id":run,"incarnation":incarnation,"command_id":ids[1],"review_id":ids[0],"review_digest":"e".repeat(64),"outcome":outcome},"administrator_grant_id":null})
}

#[tokio::test]
async fn prepare_durably_freezes_new_identity_and_restored_check_never_reprepares() {
    let mut server = Server::new(|_| Ok(json!({"preparation":"unconfirmed"}))).await;
    let (fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    let identity = Uuid::new_v4();
    app.execution_command(target, &format!("/execution prepare {identity} 2"))
        .unwrap();
    let ids = app.views[&target].execution_pending.unwrap();
    assert!(ids.iter().all(|id| !id.is_nil()));
    assert_ne!(ids[2], target.session);
    let ExecutionOperation::Prepare {
        review_id,
        command_id,
        session_id,
        workspace,
        identity: sent_identity,
    } = sent(&mut server).await
    else {
        panic!("prepare")
    };
    assert_eq!([review_id, command_id, session_id], ids);
    assert_eq!(workspace, app.views[&target].process.workspace);
    assert_eq!(sent_identity.id, identity);
    assert_eq!(sent_identity.revision.get(), 2);
    app.update(next(&mut receiver).await);
    assert!(
        app.execution_command(target, &format!("/execution prepare {identity} 2"))
            .is_err()
    );
    app.views.get_mut(&target).unwrap().execution_pending = None;
    app.execution_command(target, "/execution check").unwrap();
    assert_eq!(app.views[&target].execution_pending, Some(ids));
    assert!(
        matches!(sent(&mut server).await, ExecutionOperation::Review {review_id} if review_id == ids[0])
    );
    app.update(next(&mut receiver).await);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    let path = fixture
        .0
        .path()
        .join("helm-execution-reviews")
        .join(format!("{}-{}.json", target.route.id, target.session));
    assert_eq!(
        serde_json::from_slice::<[Uuid; 3]>(&std::fs::read(path).unwrap()).unwrap(),
        ids
    );
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn transition_freezes_current_owner_and_requires_explicit_stop_source() {
    let mut server = Server::new(|_| Ok(json!({"source_cleanup_observed":false}))).await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    let identity = Uuid::new_v4();
    assert!(
        app.execution_command(target, &format!("/execution transition {identity} 2"))
            .is_err()
    );
    assert!(app.views[&target].execution_pending.is_none());
    app.execution_command(
        target,
        &format!("/execution transition {identity} 2 stop-source"),
    )
    .unwrap();
    let ids = app.views[&target].execution_pending.unwrap();
    let ExecutionOperation::PrepareTransition {
        review_id,
        command_id,
        session_id,
        source_incarnation,
        identity: sent_identity,
        stop_source,
    } = sent(&mut server).await
    else {
        panic!("transition")
    };
    assert_eq!([review_id, command_id, session_id], ids);
    assert_eq!(session_id, target.session);
    assert_eq!(source_incarnation, server.incarnation);
    assert_eq!(sent_identity.id, identity);
    assert!(stop_source);
    app.update(next(&mut receiver).await);
    assert_eq!(app.views[&target].process.incarnation, server.incarnation);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn uncertain_approval_is_fenced_until_exact_check_without_automatic_resend() {
    let observation = Arc::new(Mutex::new(Value::Null));
    let captured = observation.clone();
    let mut server = Server::new(move |command| {
        let voyage_protocol::vessel::VesselCommand::Execution { operation } = command else {
            panic!("execution only")
        };
        if matches!(operation, ExecutionOperation::Approve { .. }) {
            Err("synthetic review transport refusal".into())
        } else {
            Ok(captured.lock().unwrap().clone())
        }
    })
    .await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let ids = [Uuid::new_v4(), Uuid::new_v4(), target.session];
    let review = saved(
        target,
        server.incarnation,
        ids,
        "awaiting_approval",
        "start",
    );
    *observation.lock().unwrap() = review.clone();
    app.execution_arrived(target, server.incarnation, Ok(review));
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.execution_command(target, "/execution approve").unwrap();
    assert!(app.views[&target].execution_uncertain);
    let ExecutionOperation::Approve { approval } = sent(&mut server).await else {
        panic!("approve")
    };
    assert_eq!((approval.review_id, approval.command_id), (ids[0], ids[1]));
    assert_eq!(approval.digest, "e".repeat(64));
    app.update(next(&mut receiver).await);
    assert!(app.execution_command(target, "/execution approve").is_err());
    assert_eq!(app.views[&target].execution_pending, Some(ids));
    app.execution_command(target, "/execution check").unwrap();
    assert!(
        matches!(sent(&mut server).await, ExecutionOperation::Review {review_id} if review_id == ids[0])
    );
    app.update(next(&mut receiver).await);
    assert!(!app.views[&target].execution_uncertain);
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn cancel_clears_retained_review_while_revoke_retains_authority_cleanup_observation() {
    for action in ["cancel", "revoke"] {
        let observation = Arc::new(Mutex::new(Value::Null));
        let captured = observation.clone();
        let mut server = Server::new(move |_| Ok(captured.lock().unwrap().clone())).await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let ids = [Uuid::new_v4(), Uuid::new_v4(), target.session];
        app.execution_arrived(
            target,
            server.incarnation,
            Ok(saved(
                target,
                server.incarnation,
                ids,
                "awaiting_approval",
                "start",
            )),
        );
        *observation.lock().unwrap() = saved(
            target,
            server.incarnation,
            ids,
            if action == "cancel" {
                "cancelled"
            } else {
                "revocation_requested"
            },
            "start",
        );
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.execution_command(target, &format!("/execution {action}"))
            .unwrap();
        let ExecutionOperation::Control { control } = sent(&mut server).await else {
            panic!("control")
        };
        assert_eq!(control.review_id, ids[0]);
        assert_ne!(control.command_id, ids[1]);
        assert_eq!(control.digest, "e".repeat(64));
        assert_eq!(
            control.action,
            if action == "cancel" {
                ExecutionReviewControlAction::Cancel
            } else {
                ExecutionReviewControlAction::Revoke
            }
        );
        app.update(next(&mut receiver).await);
        assert_eq!(
            restored(target).unwrap(),
            if action == "cancel" { None } else { Some(ids) }
        );
        assert_eq!(
            app.views[&target].execution_pending,
            if action == "cancel" { None } else { Some(ids) }
        );
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(server.requests.try_recv().is_err());
    }
}

#[tokio::test]
async fn reconcile_is_an_explicit_transition_observation_with_new_command_and_frozen_digest() {
    let mut server = Server::new(|_| Ok(json!({"source_cleanup_observed":false}))).await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let ids = [Uuid::new_v4(), Uuid::new_v4(), target.session];
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    for (state, change) in [
        ("awaiting_approval", "transition"),
        ("unconfirmed", "start"),
    ] {
        app.execution_arrived(
            target,
            server.incarnation,
            Ok(saved(target, server.incarnation, ids, state, change)),
        );
        assert!(
            app.execution_command(target, "/execution reconcile")
                .is_err()
        );
    }
    app.execution_arrived(
        target,
        server.incarnation,
        Ok(saved(
            target,
            server.incarnation,
            ids,
            "unconfirmed",
            "transition",
        )),
    );
    app.execution_command(target, "/execution reconcile")
        .unwrap();
    let ExecutionOperation::ReconcileTransition {
        review_id,
        command_id,
        digest,
    } = sent(&mut server).await
    else {
        panic!("reconcile")
    };
    assert_eq!(review_id, ids[0]);
    assert_ne!(command_id, ids[1]);
    assert_eq!(digest, "e".repeat(64));
    app.update(next(&mut receiver).await);
    assert_eq!(app.views[&target].execution_pending, Some(ids));
    assert_eq!(app.views[&target].process.incarnation, server.incarnation);
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn observational_execution_commands_preserve_review_and_parser_refusals_do_not_dispatch() {
    let mut server = Server::new(|_| Ok(json!({"configured_references":[]}))).await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    for text in [
        "/execution prepare nope 1",
        "/execution prepare 00000000-0000-0000-0000-000000000001 0",
        "/execution check",
        "/execution approve",
        "/execution cancel",
        "/execution revoke",
        "/execution nonsense",
    ] {
        assert!(app.execution_command(target, text).is_err(), "{text}");
    }
    for text in ["/execution", "/execution status", "/execution identities"] {
        app.execution_command(target, text).unwrap();
        let operation = sent(&mut server).await;
        if text.ends_with("identities") {
            assert!(matches!(operation, ExecutionOperation::Inventory));
        } else {
            assert!(
                matches!(operation, ExecutionOperation::Status {session_id} if session_id == target.session)
            );
        }
        app.update(next(&mut receiver).await);
    }
    let id = Uuid::new_v4();
    app.execution_command(target, &format!("/execution review {id}"))
        .unwrap();
    assert!(
        matches!(sent(&mut server).await, ExecutionOperation::Review {review_id} if review_id == id)
    );
    app.update(next(&mut receiver).await);
    assert!(app.views[&target].execution_pending.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(server.requests.try_recv().is_err());
}

#[test]
fn unsafe_or_malformed_execution_receipts_refuse_without_replacing_private_evidence() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for case in 0..5 {
        let (fixture, _app, target) = coverage_support::app();
        let root = fixture.0.path().join("helm-execution-reviews");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join(format!("{}-{}.json", target.route.id, target.session));
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        let bytes = match case {
            0 => b"invalid-json".to_vec(),
            1 => serde_json::to_vec(&[Uuid::nil(); 3]).unwrap(),
            2 => vec![b'x'; 1025],
            _ => serde_json::to_vec(&ids).unwrap(),
        };
        let original = root.join("original");
        std::fs::write(&original, &bytes).unwrap();
        std::fs::set_permissions(&original, std::fs::Permissions::from_mode(0o600)).unwrap();
        match case {
            3 => symlink(&original, &path).unwrap(),
            4 => std::fs::hard_link(&original, &path).unwrap(),
            _ => {
                std::fs::rename(&original, &path).unwrap();
            }
        }
        assert!(restored(target).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        if case >= 3 {
            assert!(retain(target, ids).is_err());
            assert_eq!(std::fs::read(original).unwrap(), bytes);
        }
    }
}

#[tokio::test]
async fn stale_approval_or_receipt_storage_failure_cannot_dispatch_or_replace_retained_evidence() {
    use std::os::unix::fs::PermissionsExt;
    let mut server = Server::new(|_| panic!("no execution request may escape preflight")).await;
    let (fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let ids = [Uuid::new_v4(), Uuid::new_v4(), target.session];
    for state in ["approved", "launching", "awaiting_approval"] {
        let mut value = saved(target, server.incarnation, ids, state, "start");
        if state == "awaiting_approval" {
            value["review"]["expires_at_ms"] = json!(1);
        }
        app.execution_arrived(target, server.incarnation, Ok(value));
        assert!(app.execution_command(target, "/execution approve").is_err());
        assert_eq!(app.views[&target].execution_pending, Some(ids));
    }
    let root = fixture.0.path().join("helm-execution-reviews");
    let path = root.join(format!("{}-{}.json", target.route.id, target.session));
    let bytes = std::fs::read(&path).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let replacement = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    app.execution_arrived(
        target,
        server.incarnation,
        Ok(saved(
            target,
            server.incarnation,
            replacement,
            "awaiting_approval",
            "start",
        )),
    );
    assert_eq!(app.views[&target].execution_pending, Some(ids));
    assert!(app.views[&target].execution_uncertain);
    assert!(
        app.views[&target]
            .error
            .as_ref()
            .unwrap()
            .contains("could not be retained")
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    app.clients.mark_unavailable(target.route);
    assert!(app.execution_command(target, "/execution check").is_err());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(server.requests.try_recv().is_err());
}
