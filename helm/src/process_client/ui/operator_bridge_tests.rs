use super::super::coverage_support;
use super::*;
use crossterm::event::KeyEvent;
use serde_json::json;

fn loaded(app: &mut App, target: Target) -> Loaded {
    let observation = app.operator_observation(target).unwrap();
    let request = Uuid::new_v4();
    app.operator_loading = Some((
        request,
        std::time::Instant::now() + std::time::Duration::from_secs(30),
    ));
    Loaded {
        observation,
        request,
        filter: None,
        result: Ok([
            json!({"tools":[]}),
            json!({"tasks":[]}),
            json!({"agents":[]}),
        ]),
    }
}

#[test]
fn operator_loading_results_are_request_selection_and_revision_fenced() {
    for case in 0..5 {
        let (_fixture, mut app, target) = coverage_support::app();
        let mut result = loaded(&mut app, target);
        match case {
            0 => result.request = Uuid::new_v4(),
            1 => app.selected = None,
            2 => {
                app.views
                    .get_mut(&target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .revision += 1
            }
            3 => result.result = Err("synthetic unavailable".into()),
            _ => {}
        }
        app.operator_loaded(result);
        if case == 0 {
            assert!(app.operator_loading.is_some());
            assert!(app.operator.is_none());
        } else if case == 4 {
            assert!(app.operator_loading.is_none());
            assert!(app.operator.is_some());
        } else {
            assert!(app.operator_loading.is_none());
            assert!(app.operator.is_none());
            assert!(app.status.contains("Draft retained"));
        }
        assert_eq!(app.views[&target].draft.text, "preserved draft");
        assert!(app.views[&target].pending.is_none());
    }
}

#[test]
fn operator_loading_cancellation_timeout_and_modal_input_do_not_send_tools() {
    let (_fixture, mut app, target) = coverage_support::app();
    let result = loaded(&mut app, target);
    assert!(app.operator_input(&Event::Paste("not a tool request".into())));
    assert!(!app.operator_input(&Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL
    ))));
    assert!(app.operator_loading.is_some());
    let mut release = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert!(app.operator_input(&Event::Key(release)));
    assert!(app.operator_loading.is_some());
    assert!(app.operator_input(&Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))));
    assert!(app.operator_loading.is_none());
    app.operator_loaded(result);
    assert!(app.operator.is_none());
    assert!(app.status.contains("cancelled"));
    let _ = loaded(&mut app, target);
    app.operator_loading.as_mut().unwrap().1 =
        std::time::Instant::now() - std::time::Duration::from_secs(1);
    app.poll_operator();
    assert!(app.operator_loading.is_none());
    assert!(app.status.contains("no tool was sent"));
    assert!(!app.operator_input(&Event::FocusGained));
    let result = loaded(&mut app, target);
    app.operator_loaded(result);
    assert!(app.operator_input(&Event::Resize(100, 40)));
    assert!(app.operator_input(&Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))));
    assert!(app.operator.is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
}

#[test]
fn operator_observation_rejects_unavailable_missing_deleted_and_unsnapshotted_views() {
    let (_fixture, mut app, target) = coverage_support::app();
    let original = app.operator_observation(target).unwrap();
    assert_eq!(original.revision, 17);
    assert_eq!(original.run_id, None);
    app.clients.mark_unavailable(target.route);
    assert!(
        app.operator_observation(target)
            .unwrap_err()
            .to_string()
            .contains("Vessel unavailable")
    );
    app.clients.mark_available(target.route);
    app.views
        .get_mut(&target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .lifecycle = json!({"deleted":true});
    assert!(
        app.operator_observation(target)
            .unwrap_err()
            .to_string()
            .contains("Restore")
    );
    app.views.get_mut(&target).unwrap().snapshot = None;
    assert!(
        app.operator_observation(target)
            .unwrap_err()
            .to_string()
            .contains("Waiting")
    );
    app.views.remove(&target);
    assert!(
        app.operator_observation(target)
            .unwrap_err()
            .to_string()
            .contains("Voyage unavailable")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn open_operator_reads_bound_controls_and_optional_archive_then_delivers_panel() {
    use super::super::socket_support_tests::{Server, voyage};
    use voyage_protocol::vessel::VesselCommand;
    let (_fixture, mut app, old) = coverage_support::app();
    let mut server = Server::new(|command| {
        let VesselCommand::Voyage(request) = command else {
            unreachable!()
        };
        let VoyageCommand::Controls { section, run_id } = &request.command else {
            unreachable!()
        };
        assert_eq!(*run_id, None);
        Ok(voyage(
            command,
            match section.as_str() {
                "tools" => json!({"section":"tools","value":{"inventory":[]}}),
                "todos" => json!({"section":"todos","value":{"items":[]}}),
                "subagents" => json!({"section":"subagents","value":[]}),
                "subagents_archive" => {
                    json!({"section":"subagents_archive","value":{"agents":[],"next_after":null}})
                }
                _ => panic!("unexpected control section"),
            },
        ))
    })
    .await;
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
    let (sender, mut receiver) = tokio::sync::mpsc::channel(8);
    app.sender = sender;
    app.help = true;
    app.open_operator(target, None).unwrap();
    assert!(!app.help);
    assert!(app.operator_loading.is_some());
    let update = tokio::time::timeout(std::time::Duration::from_secs(3), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    match update {
        Update::Operator(loaded) => app.operator_loaded(*loaded),
        _ => panic!("operator result"),
    }
    assert!(app.operator.is_some(), "{}", app.status);
    assert!(app.operator_loading.is_none());
    assert!(app.views[&target].pending.is_none());
    for expected in ["tools", "todos", "subagents", "subagents_archive"] {
        let VesselCommand::Voyage(request) = server.requests.try_recv().unwrap() else {
            panic!("expected voyage controls");
        };
        assert_eq!(request.session_id, target.session);
        assert_eq!(request.incarnation, None);
        let VoyageCommand::Controls { section, run_id } = request.command else {
            panic!("expected controls");
        };
        assert_eq!(section, expected);
        assert_eq!(run_id, None);
    }
    assert!(server.requests.try_recv().is_err());
}
