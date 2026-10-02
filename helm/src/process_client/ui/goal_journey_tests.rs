//! Owner-reviewed goal changes use ordinary public dispatch and exact recovery.
use super::super::{
    coverage_support,
    socket_support_tests::{Server, voyage},
};
use super::*;
use crossterm::event::KeyEvent;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use voyage_protocol::goals::GoalUsage;

fn connect(app: &mut App, old: Target, server: &Server) -> Target {
    let target = Target {
        route: app.clients.insert(server.client.clone()),
        session: server.target.session,
    };
    let mut view = app.views.remove(&old).unwrap();
    view.process.session_id = target.session;
    view.process.incarnation = server.incarnation;
    view.snapshot.as_mut().unwrap().session_id = target.session;
    view.snapshot.as_mut().unwrap().goal = Some(GoalSnapshot {
        revision: 3,
        goal: Some(Goal {
            id: Uuid::new_v4(),
            session_id: target.session,
            objective: "Verify canonical results".into(),
            status: GoalStatus::Paused,
            continuation_authorized: false,
            limits: GoalLimits::default(),
            usage: GoalUsage::default(),
            created_at_ms: 1,
            updated_at_ms: 1,
            stop_reason: Some(GoalStopReason::UserPaused),
            assessment: None,
        }),
    });
    view.observed = Some(Instant::now());
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
async fn sent(server: &mut Server) -> VesselCommand {
    tokio::time::timeout(Duration::from_secs(3), server.requests.recv())
        .await
        .unwrap()
        .unwrap()
}
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn applied(command: &VoyageCommand) -> Value {
    let VoyageCommand::GoalUpdate {
        command_id, action, ..
    } = command
    else {
        panic!("GoalUpdate only")
    };
    let goal = match action {
        GoalAction::Set { .. } => Some(*command_id),
        GoalAction::Edit { goal_id, .. }
        | GoalAction::Pause { goal_id }
        | GoalAction::Resume { goal_id } => Some(*goal_id),
        GoalAction::Clear { .. } => None,
    };
    json!({"command_id":command_id,"status":"applied","goal_revision":4,"goal_id":goal})
}

#[tokio::test]
async fn every_reviewed_goal_action_dispatches_once_with_frozen_revision_and_preserves_new_draft() {
    for text in [
        "/goal set Replacement objective",
        "/goal edit Improved objective",
        "/goal limits 4 5000 60 2",
        "/goal pause",
        "/goal resume",
        "/goal clear",
    ] {
        let mut server = Server::new(|command| match command {
            VesselCommand::Capabilities => Ok(json!({"scope":"owner"})),
            VesselCommand::Voyage(request) => Ok(voyage(command, applied(&request.command))),
            _ => panic!("owner and exact goal command only"),
        })
        .await;
        let (fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let original_goal = app.views[&target]
            .snapshot
            .as_ref()
            .unwrap()
            .goal
            .clone()
            .unwrap();
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.review_goal(target, text, true).unwrap();
        assert!(app.views[&target].pending.is_none());
        assert!(matches!(
            sent(&mut server).await,
            VesselCommand::Capabilities
        ));
        app.update(next(&mut receiver).await);
        assert_eq!(app.goal_review.as_ref().unwrap().owner, Some(true));
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 50)).unwrap();
        terminal
            .draw(|frame| app.draw_goal(frame, frame.area()))
            .unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        let visible_intent = match text {
            "/goal set Replacement objective" => "CONFIRM REPLACEMENT",
            "/goal edit Improved objective" | "/goal limits 4 5000 60 2" => "Usage is retained",
            "/goal pause" => "Pause future continuation",
            "/goal resume" => "AUTHORIZE automatic continuation",
            "/goal clear" => "CONFIRM CLEAR",
            _ => unreachable!(),
        };
        assert!(
            screen.contains(visible_intent),
            "review must display the human intent"
        );
        app.goal_input(&key(KeyCode::Enter));
        let pending = app.views[&target].pending.as_ref().unwrap().clone();
        let VesselCommand::Voyage(request) = sent(&mut server).await else {
            panic!("voyage")
        };
        assert_eq!(request.session_id, target.session);
        assert_eq!(request.incarnation, Some(server.incarnation));
        let VoyageCommand::GoalUpdate {
            command_id,
            expected_revision,
            expires_at_ms,
            action,
        } = request.command
        else {
            panic!("goal")
        };
        assert_eq!(command_id, pending.command_id);
        assert_eq!(expected_revision, 17);
        assert!(expires_at_ms > u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap());
        let old_goal = original_goal.goal.as_ref().unwrap();
        match (text, action) {
            (
                "/goal set Replacement objective",
                GoalAction::Set {
                    objective,
                    limits,
                    replace_goal_id,
                    continue_automatically,
                },
            ) => {
                assert_eq!(objective, "Replacement objective");
                assert_eq!(limits, GoalLimits::default());
                assert_eq!(replace_goal_id, Some(old_goal.id));
                assert!(!continue_automatically);
            }
            (
                "/goal edit Improved objective",
                GoalAction::Edit {
                    goal_id,
                    objective,
                    limits,
                },
            ) => {
                assert_eq!(goal_id, old_goal.id);
                assert_eq!(objective, "Improved objective");
                assert_eq!(limits, old_goal.limits);
            }
            (
                "/goal limits 4 5000 60 2",
                GoalAction::Edit {
                    goal_id,
                    objective,
                    limits,
                },
            ) => {
                assert_eq!(goal_id, old_goal.id);
                assert_eq!(objective, "Verify canonical results");
                assert_eq!(
                    (
                        limits.runs,
                        limits.tokens,
                        limits.elapsed_ms,
                        limits.no_progress_runs
                    ),
                    (4, 5000, 60000, 2)
                );
            }
            ("/goal pause", GoalAction::Pause { goal_id })
            | ("/goal resume", GoalAction::Resume { goal_id })
            | ("/goal clear", GoalAction::Clear { goal_id }) => assert_eq!(goal_id, old_goal.id),
            _ => panic!("wire action differs from the human command"),
        }
        assert!(!app.goal_input(&key(KeyCode::Enter)));
        app.views
            .get_mut(&target)
            .unwrap()
            .draft
            .set_text("new unsent input".into());
        app.update(next(&mut receiver).await);
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "new unsent input");
        assert_eq!(
            app.views[&target].snapshot.as_ref().unwrap().goal,
            Some(original_goal)
        );
        assert!(server.requests.try_recv().is_err());
        for entry in std::fs::read_dir(fixture.0.path().join("helm-command-receipts")).unwrap() {
            assert!(
                !std::fs::read_to_string(entry.unwrap().path())
                    .unwrap()
                    .contains("new unsent input")
            );
        }
    }
}

#[tokio::test]
async fn restricted_owner_scope_refuses_confirm_without_inserting_input_or_dispatching_mutation() {
    let mut server = Server::new(|command| {
        assert!(matches!(command, VesselCommand::Capabilities));
        Ok(json!({"scope":"session"}))
    })
    .await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.review_goal(target, "/goal resume", true).unwrap();
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    app.goal_input(&Event::Paste("quarantined modal text".into()));
    app.goal_input(&key(KeyCode::Enter));
    assert!(
        app.goal_review
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("owner")
    );
    assert!(app.views[&target].pending.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    app.goal_input(&key(KeyCode::Esc));
    assert!(app.goal_review.is_none());
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn late_owner_answer_cannot_authorize_replaced_review() {
    let mut server = Server::new(|command| {
        assert!(matches!(command, VesselCommand::Capabilities));
        Ok(json!({"scope":"owner"}))
    })
    .await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.review_goal(target, "/goal resume", true).unwrap();
    let first = app.goal_review.as_ref().unwrap().id;
    sent(&mut server).await;
    let old_answer = next(&mut receiver).await;
    app.review_goal(target, "/goal clear", true).unwrap();
    let second = app.goal_review.as_ref().unwrap().id;
    assert_ne!(first, second);
    app.update(old_answer);
    assert_eq!(app.goal_review.as_ref().unwrap().owner, None);
    app.goal_input(&key(KeyCode::Enter));
    assert!(app.views[&target].pending.is_none());
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    assert_eq!(app.goal_review.as_ref().unwrap().owner, Some(true));
    app.goal_input(&key(KeyCode::Esc));
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn current_goal_review_refuses_stale_owner_snapshot_revision_recovery_and_cleanup() {
    for case in 0..9 {
        let mut server = Server::new(|command| {
            assert!(matches!(command, VesselCommand::Capabilities));
            Ok(json!({"scope":"owner"}))
        })
        .await;
        let (_fixture, mut app, old) = coverage_support::app();
        let target = connect(&mut app, old, &server);
        let (sender, mut receiver) = mpsc::channel(8);
        app.sender = sender;
        app.review_goal(target, "/goal resume", true).unwrap();
        sent(&mut server).await;
        app.update(next(&mut receiver).await);
        match case {
            0 => {
                app.goal_review.as_mut().unwrap().owner_at =
                    Some(Instant::now() - Duration::from_secs(31))
            }
            1 => {
                app.views.get_mut(&target).unwrap().observed =
                    Some(Instant::now() - Duration::from_secs(36))
            }
            2 => app.views.get_mut(&target).unwrap().process.incarnation = Uuid::new_v4(),
            3 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .goal
                    .as_mut()
                    .unwrap()
                    .revision += 1
            }
            4 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .recovery_pending = true
            }
            5 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .catalogue_only = true
            }
            6 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .pending_cleanup_run = Some(Uuid::new_v4())
            }
            7 => app.clients.mark_unavailable(target.route),
            _ => app.views.get_mut(&target).unwrap().connection_unavailable = true,
        }
        app.goal_input(&key(KeyCode::Enter));
        assert!(
            app.goal_review.as_ref().unwrap().error.is_some(),
            "case {case}"
        );
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(server.requests.try_recv().is_err());
        assert!(app.command_checks.is_empty());
        app.goal_input(&key(KeyCode::Esc));
    }
}

#[tokio::test]
async fn pause_during_running_cleanup_changes_only_future_continuation() {
    let mut server = Server::new(|command| match command {
        VesselCommand::Capabilities => Ok(json!({"scope":"owner"})),
        VesselCommand::Voyage(request) => {
            assert!(matches!(
                request.command,
                VoyageCommand::GoalUpdate {
                    action: GoalAction::Pause { .. },
                    ..
                }
            ));
            Ok(voyage(command, applied(&request.command)))
        }
        _ => panic!("no cancel or run allowed"),
    })
    .await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let run_id = Uuid::new_v4();
    let snapshot = app
        .views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap();
    snapshot.run =
        Some(serde_json::from_value(json!({"run_id":run_id,"state":"running"})).unwrap());
    snapshot.pending_cleanup_run = Some(run_id);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.review_goal(target, "/goal pause", true).unwrap();
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    app.goal_input(&key(KeyCode::Enter));
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    let snapshot = app.views[&target].snapshot.as_ref().unwrap();
    assert_eq!(snapshot.run.as_ref().unwrap().run_id, run_id);
    assert!(snapshot.run.as_ref().unwrap().active());
    assert_eq!(snapshot.pending_cleanup_run, Some(run_id));
    assert!(app.views[&target].pending.is_none());
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn mismatched_goal_result_retains_exact_envelope_and_recovers_with_resolve_only() {
    let mut server=Server::new(|command|match command {
        VesselCommand::Capabilities=>Ok(json!({"scope":"owner"})),
        VesselCommand::Voyage(request)=>match &request.command {
            VoyageCommand::GoalUpdate{command_id,..}=>Ok(voyage(command,json!({"command_id":command_id,"status":"applied","goal_revision":4,"goal_id":Uuid::new_v4()}))),
            VoyageCommand::Resolve{original:Some(original),..}=>Ok(voyage(command,applied(original))),
            _=>panic!("recovery must observe original, never repeat mutation"),
        },_=>panic!("goal only"),
    }).await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.review_goal(target, "/goal resume", true).unwrap();
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    app.goal_input(&key(KeyCode::Enter));
    let pending = app.views[&target].pending.as_ref().unwrap().clone();
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    assert_eq!(
        app.views[&target].pending.as_ref().unwrap().command_id,
        pending.command_id
    );
    let mut reopened = super::super::state::View::new(app.views[&target].process.clone());
    receipts::load(&app.clients[target.route], &mut reopened).unwrap();
    assert_eq!(
        reopened.pending.as_ref().unwrap().command_id,
        pending.command_id
    );
    assert_eq!(
        serde_json::to_value(&reopened.pending.as_ref().unwrap().original).unwrap(),
        serde_json::to_value(&pending.original).unwrap()
    );
    assert!(reopened.draft.text.is_empty());
    app.command_checks
        .insert((target, pending.command_id), Some(Instant::now()));
    app.reconcile_pending();
    app.reconcile_pending();
    let VesselCommand::Voyage(request) = sent(&mut server).await else {
        panic!("resolve")
    };
    let VoyageCommand::Resolve {
        command_id,
        original,
    } = request.command
    else {
        panic!("resolve only")
    };
    assert_eq!(command_id, pending.command_id);
    assert_eq!(
        serde_json::to_value(original).unwrap(),
        serde_json::to_value(pending.original).unwrap()
    );
    app.update(next(&mut receiver).await);
    assert!(app.views[&target].pending.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn unavailable_private_receipt_storage_refuses_goal_before_wire_effect() {
    let mut server = Server::new(|command| {
        assert!(matches!(command, VesselCommand::Capabilities));
        Ok(json!({"scope":"owner"}))
    })
    .await;
    let (fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.review_goal(target, "/goal resume", true).unwrap();
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    let root = fixture.0.path().join("helm-command-receipts");
    std::fs::write(&root, b"unrelated sentinel").unwrap();
    app.goal_input(&key(KeyCode::Enter));
    assert!(
        app.goal_review
            .as_ref()
            .unwrap()
            .error
            .as_ref()
            .unwrap()
            .contains("nothing sent")
    );
    assert!(app.views[&target].pending.is_none());
    assert!(app.command_checks.is_empty());
    assert_eq!(std::fs::read(root).unwrap(), b"unrelated sentinel");
    assert!(server.requests.try_recv().is_err());
}

#[tokio::test]
async fn status_modal_scroll_cancel_and_quit_keep_objective_usage_and_composer_out_of_dispatch() {
    let mut server = Server::new(|command| {
        assert!(matches!(command, VesselCommand::Capabilities));
        Ok(json!({"scope":"owner"}))
    })
    .await;
    let (_fixture, mut app, old) = coverage_support::app();
    let target = connect(&mut app, old, &server);
    let state = app
        .views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .goal
        .as_mut()
        .unwrap();
    state.goal.as_mut().unwrap().usage.unmeasured_runs = 1;
    state.goal.as_mut().unwrap().stop_reason = Some(GoalStopReason::UsageUnknown);
    let (sender, mut receiver) = mpsc::channel(8);
    app.sender = sender;
    app.review_goal(target, "/goal status", true).unwrap();
    sent(&mut server).await;
    app.update(next(&mut receiver).await);
    app.goal_input(&key(KeyCode::Down));
    app.goal_input(&key(KeyCode::Up));
    app.goal_input(&key(KeyCode::Enter));
    assert!(app.views[&target].pending.is_none());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 36)).unwrap();
    terminal
        .draw(|frame| app.draw_goal(frame, frame.area()))
        .unwrap();
    let screen: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(screen.contains("incomplete usage"));
    assert!(screen.contains("fresh allowance"));
    app.goal_input(&Event::Key(KeyEvent::new(
        KeyCode::Char('q'),
        KeyModifiers::CONTROL,
    )));
    assert!(app.quit);
    assert!(app.goal_review.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(server.requests.try_recv().is_err());
}
