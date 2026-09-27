//! End-to-end presentation dispatch over the existing loopback Vessel fixture.
use super::socket_support_tests::{Server, voyage};
use super::*;
use serde_json::json;
use uuid::Uuid;
use voyage_protocol::vessel::{VesselCommand, VoyageCommand};

async fn next_update(receiver: &mut mpsc::Receiver<Update>) -> Update {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .expect("dispatch must finish")
        .expect("update channel open")
}

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

#[tokio::test]
async fn submit_receipts_preserve_exact_wire_identity_and_resolve_only_matching_results() {
    for status in ["accepted", "rejected", "not_admitted", "unknown"] {
        let mut server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else {
                panic!("voyage")
            };
            let VoyageCommand::Submit {
                command_id,
                expected_revision,
                prompt,
                ..
            } = &request.command
            else {
                panic!("submit")
            };
            assert_eq!(*expected_revision, 17);
            assert_eq!(prompt, "preserved draft");
            Ok(voyage(
                command,
                json!({"command_id":command_id,"status":status}),
            ))
        })
        .await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.command_for(target, "preserved draft".into(), false)
            .unwrap();
        let pending = app.views[&target].pending.as_ref().unwrap();
        let identity = pending.command_id;
        assert_eq!(pending.incarnation, server.incarnation);
        assert_eq!(pending.draft, "preserved draft");
        assert!(matches!(
            pending.original.as_deref(),
            Some(VoyageCommand::Submit { .. })
        ));
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].transcript.borrow().delivery.is_some());
        let update = next_update(&mut receiver).await;
        let sent = server.requests.recv().await.unwrap();
        let VesselCommand::Voyage(request) = sent else {
            unreachable!()
        };
        assert_eq!(request.session_id, target.session);
        assert_eq!(request.incarnation, None);
        app.update(update);
        let view = &app.views[&target];
        if status == "unknown" {
            assert_eq!(view.pending.as_ref().unwrap().command_id, identity);
            assert!(app.status.contains("Not confirmed"));
        } else {
            assert!(view.pending.is_none());
            assert_eq!(
                view.draft.text,
                if status == "accepted" {
                    ""
                } else {
                    "preserved draft"
                }
            );
        }
        for jobs in app.route_tasks.values_mut() {
            for job in jobs.drain(..) {
                job.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn malformed_and_foreign_receipts_never_acknowledge_the_pending_command() {
    for result in [
        json!({}),
        json!({"status":"accepted","command_id":Uuid::new_v4()}),
        json!({"status":"accepted"}),
    ] {
        let server = Server::new(move |command| Ok(voyage(command, result.clone()))).await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.command_for(target, "preserved draft".into(), true)
            .unwrap();
        let identity = app.views[&target].pending.as_ref().unwrap().command_id;
        app.update(next_update(&mut receiver).await);
        assert_eq!(
            app.views[&target].pending.as_ref().unwrap().command_id,
            identity
        );
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.status.contains("could not be matched"));
        for jobs in app.route_tasks.values_mut() {
            for job in jobs.drain(..) {
                job.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn pending_guard_and_inflight_deduplication_prevent_duplicate_wire_effects() {
    let mut server = Server::new(|command| {
        let VesselCommand::Voyage(request) = command else {
            unreachable!()
        };
        let VoyageCommand::Submit { command_id, .. } = &request.command else {
            unreachable!()
        };
        Ok(voyage(
            command,
            json!({"command_id":command_id,"status":"accepted"}),
        ))
    })
    .await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.command_for(target, "preserved draft".into(), true)
        .unwrap();
    let pending = app.views[&target].pending.as_ref().unwrap();
    let command_id = pending.command_id;
    let original = pending.original.as_deref().unwrap().clone();
    app.dispatch(target, command_id, original);
    let error = app
        .command_for(target, "second prompt".into(), false)
        .unwrap_err();
    assert!(error.to_string().contains("pending"), "{error}");
    app.update(next_update(&mut receiver).await);
    assert!(app.views[&target].pending.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    for jobs in app.route_tasks.values_mut() {
        for job in jobs.drain(..) {
            job.await.unwrap();
        }
    }
    server.requests.recv().await.unwrap();
    assert!(server.requests.try_recv().is_err());
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn administrative_commands_keep_revision_and_payload_through_public_dispatch() {
    for (text, kind) in [("/rename Updated voyage", "rename")] {
        let server = Server::new(move |command| {
            let VesselCommand::Voyage(request) = command else {
                unreachable!()
            };
            let (command_id, revision) = match &request.command {
                VoyageCommand::Rename {
                    command_id,
                    expected_revision,
                    name,
                    ..
                } => {
                    assert_eq!(kind, "rename");
                    assert_eq!(name, "Updated voyage");
                    (*command_id, *expected_revision)
                }
                VoyageCommand::Compact {
                    command_id,
                    expected_revision,
                    ..
                } => {
                    assert_eq!(kind, "compact");
                    (*command_id, *expected_revision)
                }
                VoyageCommand::SetModel {
                    command_id,
                    expected_revision,
                    model,
                    ..
                } => {
                    assert_eq!(kind, "model");
                    assert_eq!(model, "example-model");
                    (*command_id, *expected_revision)
                }
                _ => panic!("unexpected command"),
            };
            assert_eq!(revision, 17);
            Ok(voyage(
                command,
                json!({"command_id":command_id,"status":"completed"}),
            ))
        })
        .await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.command_for(target, text.into(), true).unwrap();
        app.update(next_update(&mut receiver).await);
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        for jobs in app.route_tasks.values_mut() {
            for job in jobs.drain(..) {
                job.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn local_command_dispatch_is_independent_of_snapshot_and_preserves_text() {
    let (_fixture, mut app, target) = coverage_support::app();
    app.views.get_mut(&target).unwrap().snapshot = None;
    for command in ["/help", "/terminals", "/archived", "/voyages"] {
        app.command_for(target, command.into(), false).unwrap();
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].pending.is_none());
    }
    assert!(app.help);
    assert!(app.views[&target].terminals.open);
    assert!(!app.archives);
    app.command_for(target, format!("/use {}", target.session), true)
        .unwrap();
    assert_eq!(app.selected, Some(target));
    for command in [
        "/use invalid",
        "/use 00000000-0000-0000-0000-000000000000",
        "/terminal invalid",
    ] {
        assert!(app.command_for(target, command.into(), true).is_err());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
    }
    assert!(
        app.command_for(target, "new prompt".into(), false)
            .unwrap_err()
            .to_string()
            .contains("snapshot")
    );
    app.command_for(target, "/quit".into(), false).unwrap();
    assert!(app.quit);
}
