//! Frozen human lifecycle reviews use one exact envelope and retain unsent input.
use super::history_review::BranchReview;
use super::socket_support_tests::{Server, voyage};
use super::*;
use serde_json::json;
use uuid::Uuid;
use voyage_protocol::vessel::{VesselCommand, VoyageCommand};

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
fn review(app: &App, target: Target) -> BranchReview {
    BranchReview {
        target,
        incarnation: app.views[&target].process.incarnation,
        revision: 17,
        branch_id: Uuid::new_v4(),
        host: "fixture-host".into(),
        total: 10,
        points: vec![(3, "historical boundary".into())],
        selected: Some(0),
    }
}
async fn update(receiver: &mut mpsc::Receiver<observe::Update>) -> observe::Update {
    tokio::time::timeout(Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap()
}
fn archived(app: &mut App, target: Target) {
    let view = app.views.get_mut(&target).unwrap();
    view.process.state = voyage_protocol::process::ProcessState::Stopped;
    view.process.archive = Some(voyage_protocol::process::ArchivedVoyage {
        name: Some("Archived".into()),
        revision: 17,
        receipt: json!({"command_id":Uuid::new_v4(),"archived":true}),
    });
}

#[tokio::test]
async fn branch_frozen_review_refusal_matrix_has_no_receipt_or_wire_effect() {
    for variant in 0..7 {
        let (_fixture, mut app, target) = coverage_support::app();
        let mut frozen = review(&app, target);
        match variant {
            0 => app.clients.mark_unavailable(target.route),
            1 => {
                app.views.remove(&target);
            }
            2 => {
                app.views.get_mut(&target).unwrap().snapshot = None;
            }
            3 => frozen.incarnation = Uuid::new_v4(),
            4 => frozen.revision += 1,
            5 => frozen.target.session = Uuid::new_v4(),
            _ => {
                app.views.get_mut(&target).unwrap().pending = Some(state::Pending {
                    account_host: None,
                    command_id: Uuid::new_v4(),
                    incarnation: frozen.incarnation,
                    draft: "earlier pending".into(),
                    preserve_draft: true,
                    original: None,
                    receipt_only: true,
                })
            }
        }
        let prior = app.views.get(&target).map(|view| view.draft.text.clone());
        assert!(app.branch(target, None, true, frozen).is_err());
        assert!(app.route_tasks.is_empty());
        assert!(app.command_checks.is_empty());
        if let Some(prior) = prior {
            assert_eq!(app.views[&target].draft.text, prior);
        }
    }
}

#[tokio::test]
async fn branch_success_dispatches_exact_historical_boundary_and_preserves_new_composer() {
    for selected_elsewhere in [false, true] {
        let mut server=Server::new(|command| {
            let VesselCommand::Branch {branch_id,name,..}=command else {panic!("only reviewed branch allowed")};
            Ok(json!({"session_id":branch_id,"incarnation":Uuid::new_v4(),"workspace":"/synthetic-workspace","state":"starting","name":name}))
        }).await;
        let (fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let frozen = review(&app, target);
        let branch_id = frozen.branch_id;
        let original_owner = frozen.incarnation;
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.branch(target, Some("Historical branch".into()), true, frozen)
            .unwrap();
        let pending = app.views[&target].pending.as_ref().unwrap().clone();
        assert!(pending.receipt_only);
        assert!(pending.original.is_none());
        assert_eq!(pending.incarnation, original_owner);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        let command = tokio::time::timeout(Duration::from_secs(3), server.requests.recv())
            .await
            .unwrap()
            .unwrap();
        let VesselCommand::Branch {
            command_id,
            session_id,
            incarnation,
            expected_revision,
            branch_id: sent_branch,
            through_message,
            name,
            ..
        } = command
        else {
            panic!("branch")
        };
        assert_eq!(command_id, pending.command_id);
        assert_eq!(session_id, target.session);
        assert_eq!(incarnation, original_owner);
        assert_eq!(expected_revision, 17);
        assert_eq!(sent_branch, branch_id);
        assert_eq!(through_message, Some(3));
        assert_eq!(name.as_deref(), Some("Historical branch"));
        app.views
            .get_mut(&target)
            .unwrap()
            .draft
            .set_text("new unsent composer".into());
        let elsewhere = Target {
            route: target.route,
            session: Uuid::new_v4(),
        };
        if selected_elsewhere {
            app.selected = Some(elsewhere);
        }
        app.update(update(&mut receiver).await);
        app.update(update(&mut receiver).await);
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "new unsent composer");
        assert_eq!(
            app.selected,
            Some(if selected_elsewhere {
                elsewhere
            } else {
                Target {
                    route: target.route,
                    session: branch_id,
                }
            })
        );
        assert!(app.views.contains_key(&Target {
            route: target.route,
            session: branch_id
        }));
        assert!(server.requests.try_recv().is_err());
        for entry in std::fs::read_dir(fixture.0.path().join("helm-command-receipts")).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                assert!(
                    !std::fs::read_to_string(path)
                        .unwrap()
                        .contains("new unsent composer")
                );
            }
        }
    }
}

#[tokio::test]
async fn branch_explicit_refusal_and_malformed_creation_do_not_create_a_new_view() {
    for variant in 0..5 {
        let mut server = Server::new(move |command| {
            let VesselCommand::Branch {branch_id,..}=command else {panic!("branch")};
            match variant {
                0=>Err("explicit branch refusal".into()),
                1=>Ok(json!({"malformed":true})),
                _=>Ok(json!({"session_id":if variant==2 {Uuid::new_v4()}else {*branch_id},"incarnation":if variant==3 {Uuid::nil()}else {Uuid::new_v4()},"workspace":if variant==4 {"/different-workspace"}else {"/synthetic-workspace"},"state":"starting"})),
            }
        })
        .await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let frozen = review(&app, target);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.branch(target, None, true, frozen).unwrap();
        let id = app.views[&target].pending.as_ref().unwrap().command_id;
        server.requests.recv().await.unwrap();
        let first = update(&mut receiver).await;
        if let observe::Update::Command {
            command_id,
            refused,
            ..
        } = &first
        {
            assert_eq!(*command_id, id);
            assert_eq!(*refused, variant == 0);
        } else {
            panic!("exact command update")
        };
        app.update(first);
        assert!(receiver.try_recv().is_err());
        assert_eq!(app.views[&target].pending.is_some(), variant != 0);
        assert_eq!(app.views.len(), 1);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert_eq!(app.selected, Some(target));
        assert!(server.requests.try_recv().is_err());
    }
}

#[tokio::test]
async fn lifecycle_receipt_persistence_failure_refuses_before_any_dispatch() {
    for restoration in [false, true] {
        let (fixture, mut app, target) = coverage_support::app();
        let frozen = review(&app, target);
        if restoration {
            archived(&mut app, target);
        }
        std::fs::write(
            fixture.0.path().join("helm-command-receipts"),
            b"unrelated blocking file",
        )
        .unwrap();
        let result = if restoration {
            app.restore_archive(target, true)
        } else {
            app.branch(target, None, true, frozen)
        };
        assert!(result.is_err());
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.route_tasks.is_empty());
        assert!(app.command_checks.is_empty());
        assert_eq!(
            std::fs::read(fixture.0.path().join("helm-command-receipts")).unwrap(),
            b"unrelated blocking file"
        );
    }
}

#[tokio::test]
async fn archive_restore_refuses_unconfirmed_owner_missing_archive_and_pending_delivery() {
    for variant in 0..6 {
        let (_fixture, mut app, target) = coverage_support::app();
        archived(&mut app, target);
        match variant {
            0 => app.clients.mark_unavailable(target.route),
            1 => {
                app.views.remove(&target);
            }
            2 => app.views.get_mut(&target).unwrap().process.archive = None,
            3 => {
                app.views.get_mut(&target).unwrap().process.state =
                    voyage_protocol::process::ProcessState::Unavailable
            }
            4 => {
                app.views.get_mut(&target).unwrap().process.state =
                    voyage_protocol::process::ProcessState::CleanupUnconfirmed
            }
            _ => {
                app.views.get_mut(&target).unwrap().pending = Some(state::Pending {
                    account_host: None,
                    command_id: Uuid::new_v4(),
                    incarnation: Uuid::new_v4(),
                    draft: "old".into(),
                    preserve_draft: true,
                    original: None,
                    receipt_only: true,
                })
            }
        }
        assert!(app.restore_archive(target, true).is_err());
        assert!(app.route_tasks.is_empty());
        assert!(app.command_checks.is_empty());
    }
}

#[tokio::test]
async fn archive_restore_uses_new_owner_revision_and_one_exact_mutation_id() {
    let new_owner = Uuid::new_v4();
    let mut server=Server::new(move |command| match command {
        VesselCommand::Restart {session_id,..}=>Ok(json!({"session_id":session_id,"incarnation":new_owner,"workspace":"/synthetic-workspace","state":"live"})),
        VesselCommand::Voyage(request)=>match &request.command {
            VoyageCommand::Snapshot=>Ok(voyage(command,json!({"session_id":request.session_id,"revision":29,"model":"fixture","messages":[],"lifecycle":{"archived":true}}))),
            VoyageCommand::Archive {command_id,archived,..}=>Ok(voyage(command,json!({"command_id":command_id,"status":"applied","archived":archived}))),
            _=>panic!("no other voyage operation"),
        },_=>panic!("no other Vessel operation"),
    }).await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    archived(&mut app, target);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.archives = true;
    app.restore_archive(target, true).unwrap();
    let pending = app.views[&target].pending.as_ref().unwrap().clone();
    for step in 0..3 {
        let command = tokio::time::timeout(Duration::from_secs(3), server.requests.recv())
            .await
            .unwrap()
            .unwrap();
        match (step, command) {
            (
                0,
                VesselCommand::Restart {
                    command_id,
                    session_id,
                    incarnation,
                },
            ) => {
                assert_eq!(command_id, pending.command_id);
                assert_eq!(session_id, target.session);
                assert_eq!(incarnation, pending.incarnation);
            }
            (1, VesselCommand::Voyage(request)) => {
                assert_eq!(request.incarnation, Some(new_owner));
                assert!(matches!(request.command, VoyageCommand::Snapshot));
            }
            (2, VesselCommand::Voyage(request)) => {
                assert_eq!(request.incarnation, Some(new_owner));
                let VoyageCommand::Archive {
                    command_id,
                    expected_revision,
                    archived,
                    ..
                } = request.command
                else {
                    panic!("restore archive")
                };
                assert_eq!(command_id, pending.command_id);
                assert_eq!(expected_revision, 29);
                assert!(!archived);
            }
            _ => panic!("ordered exact restore sequence"),
        }
    }
    app.update(update(&mut receiver).await);
    app.update(update(&mut receiver).await);
    assert!(app.views[&target].pending.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(!app.archives);
    assert_eq!(app.views[&target].process.incarnation, new_owner);
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn archive_restore_unconfirmed_identity_and_stage_failures_never_replay_effects() {
    for mode in 0..8 {
        let new_owner = Uuid::new_v4();
        let mut server=Server::new(move |command|match command {
            VesselCommand::Restart {session_id,incarnation,..}=>match mode {
                0=>Err("explicit restart refusal".into()),
                _=>Ok(json!({"session_id":if mode==1 {Uuid::new_v4()}else {*session_id},"incarnation":if mode==2 {Uuid::nil()}else if mode==3 {*incarnation}else {new_owner},"workspace":if mode==4 {"/wrong"}else {"/synthetic-workspace"},"state":"live"})),
            },
            VesselCommand::Voyage(request)=>match &request.command {
                VoyageCommand::Snapshot=>if mode==5 {Err("snapshot refused".into())}else {Ok(voyage(command,json!({"session_id":request.session_id,"revision":if mode==6 {serde_json::Value::Null}else {json!(29)},"messages":[],"model":"fixture"})))},
                VoyageCommand::Archive {..}=>Err("archive refusal".into()),
                _=>panic!("no other operation"),
            },_=>panic!("no other lifecycle command"),
        }).await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        archived(&mut app, target);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.restore_archive(target, true).unwrap();
        let pending_id = app.views[&target].pending.as_ref().unwrap().command_id;
        let first = update(&mut receiver).await;
        let final_update = if matches!(first, observe::Update::Created { .. }) {
            app.update(first);
            update(&mut receiver).await
        } else {
            first
        };
        let observe::Update::Command {
            command_id,
            refused,
            result,
            ..
        } = &final_update
        else {
            panic!("bounded final command update")
        };
        assert_eq!(*command_id, pending_id);
        assert!(result.is_err());
        assert_eq!(*refused, matches!(mode, 0 | 5 | 7));
        app.update(final_update);
        assert_eq!(app.views.len(), 1);
        assert_eq!(app.selected, Some(target));
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert_eq!(
            app.views[&target].pending.is_some(),
            !matches!(mode, 0 | 5 | 7)
        );
        let expected = if mode <= 4 {
            1
        } else if mode <= 6 {
            2
        } else {
            3
        };
        let mut observed = 0;
        while let Ok(command) = server.requests.try_recv() {
            if let VesselCommand::Restart { command_id, .. } = command {
                assert_eq!(command_id, pending_id);
            }
            observed += 1;
        }
        assert_eq!(observed, expected);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn archived_navigation_changes_only_visible_selection_and_preserves_all_composers() {
    let (_fixture, mut app, target) = coverage_support::app();
    let mut second = app.views[&target].process.clone();
    second.session_id = Uuid::new_v4();
    let archive_target = Target {
        route: target.route,
        session: second.session_id,
    };
    app.views.insert(archive_target, state::View::new(second));
    archived(&mut app, archive_target);
    app.views
        .get_mut(&archive_target)
        .unwrap()
        .draft
        .set_text("archived unsent input".into());
    app.show_archives(true);
    assert!(app.archives);
    assert_eq!(app.selected, Some(archive_target));
    app.show_archives(false);
    assert!(!app.archives);
    assert_eq!(app.selected, Some(target));
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert_eq!(
        app.views[&archive_target].draft.text,
        "archived unsent input"
    );
    assert!(app.route_tasks.is_empty());
    assert!(app.command_checks.is_empty());
}

#[tokio::test]
async fn export_refuses_blank_disconnected_or_missing_destination_without_dispatch() {
    for variant in 0..3 {
        let (_fixture, mut app, target) = coverage_support::app();
        let path = if variant == 0 {
            "   "
        } else {
            "unused-destination"
        };
        if variant == 1 {
            app.clients.mark_unavailable(target.route);
        }
        if variant == 2 {
            app.views.remove(&target);
        }
        assert!(app.export(target, path).is_err());
        assert!(app.route_tasks.is_empty());
        assert!(app.command_checks.is_empty());
        if let Some(view) = app.views.get(&target) {
            assert_eq!(view.draft.text, "preserved draft");
            assert!(view.pending.is_none());
        }
    }
}

#[tokio::test]
async fn export_success_collision_and_revision_refusal_keep_draft_and_atomic_destination() {
    for mode in 0..3 {
        let incarnation = Uuid::new_v4();
        let mut server=Server::new(move |command|match command {
            VesselCommand::Inspect {session_id}=>Ok(json!({"session_id":session_id,"incarnation":incarnation,"workspace":"/synthetic-workspace","state":"suspended"})),
            VesselCommand::Voyage(request)=>match request.command {
                VoyageCommand::Snapshot=>Ok(voyage(command,json!({"session_id":request.session_id,"revision":17,"total_messages":0,"messages":[]}))),
                VoyageCommand::History {offset,limit,expected_revision}=>{
                    assert_eq!(offset,0);assert_eq!(limit,1);assert_eq!(expected_revision,Some(17));
                    if mode==2 {Err("revision changed".into())}else {Ok(voyage(command,json!({"revision":17,"messages":[]})))}
                },_=>panic!("export cannot mutate or wake"),
            },_=>panic!("export must use read-only commands"),
        }).await;
        let (fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let destination = fixture.0.path().join("conversation.md");
        if mode == 1 {
            std::fs::write(&destination, b"existing unrelated document").unwrap();
        }
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.export(target, destination.to_str().unwrap()).unwrap();
        let result = update(&mut receiver).await;
        let observe::Update::Control {
            target: returned,
            incarnation: owner,
            result: outcome,
        } = &result
        else {
            panic!("export returns a control observation")
        };
        assert_eq!(*returned, target);
        assert_eq!(*owner, server.incarnation);
        assert_eq!(outcome.is_ok(), mode == 0);
        app.update(result);
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].pending.is_none());
        match mode {
            0 => assert!(
                std::fs::read_to_string(&destination)
                    .unwrap()
                    .contains(&format!("# Voyage {}", target.session))
            ),
            1 => assert_eq!(
                std::fs::read(&destination).unwrap(),
                b"existing unrelated document"
            ),
            _ => assert!(!destination.exists()),
        }
        assert!(std::fs::read_dir(fixture.0.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tmp")
        }));
        let mut count = 0;
        while server.requests.try_recv().is_ok() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
