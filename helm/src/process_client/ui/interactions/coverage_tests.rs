use super::super::{App, coverage_support, state::Target};
use super::*;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};
use serde_json::json;

fn decision(app: &mut App, target: Target, request: serde_json::Value, expires: u64) -> Uuid {
    let id = Uuid::new_v4();
    let view = app.views.get_mut(&target).unwrap();
    view.snapshot.as_mut().unwrap().decisions.push(serde_json::from_value(json!({"decision_id":id,"run_id":Uuid::new_v4(),"incarnation":view.process.incarnation,"expires_at_ms":expires,"request":request})).unwrap());
    app.sync_interactions();
    id
}
fn draw(app: &App, width: u16, height: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
    term.draw(|f| render::draw(f, app, Rect::new(0, 0, width, height)))
        .unwrap();
    term.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn key(app: &mut App, code: KeyCode) {
    assert!(
        app.interaction_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap()
    );
}

#[test]
fn question_selection_custom_editor_and_utf8_editing_preserve_composer() {
    let (_fixture, mut app, target) = coverage_support::app();
    let id = decision(
        &mut app,
        target,
        json!({"kind":"question","question":{"question":"Choose a direction","options":["North","South"]}}),
        u64::MAX,
    );
    let output = draw(&app, 100, 35);
    for expected in [
        "Your answer needed",
        "Choose a direction",
        "North",
        "South",
        "Write a custom answer",
        "[Skip]",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    assert!(app.question_editor().is_none());
    key(&mut app, KeyCode::Down);
    assert_eq!(
        app.interactions.borrow().answers[&(target, id)].option,
        Some(1)
    );
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.interactions.borrow().answers[&(target, id)].option,
        None
    );
    key(&mut app, KeyCode::Enter);
    assert!(draw(&app, 100, 35).contains("Your answer"));
    app.interaction_input(&Event::Paste("a界\n\tb".into()))
        .unwrap();
    assert_eq!(app.question_editor().unwrap().1, "a界    b");
    // Remove the four normalized whitespace cells, then exercise multibyte deletion.
    key(&mut app, KeyCode::Left);
    for _ in 0..4 {
        key(&mut app, KeyCode::Backspace);
    }
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(app.question_editor().unwrap().1, "ab");
    key(&mut app, KeyCode::Home);
    key(&mut app, KeyCode::Delete);
    key(&mut app, KeyCode::End);
    key(&mut app, KeyCode::Char('é'));
    key(&mut app, KeyCode::Left);
    key(&mut app, KeyCode::Right);
    assert_eq!(app.question_editor().unwrap().1, "bé");
    key(&mut app, KeyCode::Esc);
    assert!(app.question_editor().is_none());
    assert_eq!(app.views[&target].draft.text, "preserved draft");
    assert!(app.views[&target].pending.is_none());
}

#[test]
fn decisions_cycle_without_reusing_displayed_identity_and_overlays_suspend_focus() {
    let (_fixture, mut app, target) = coverage_support::app();
    let first = decision(
        &mut app,
        target,
        json!({"kind":"approval","approval":{"action":"Read synthetic file","description":"Inspect","resource":"fixture","reason":"Regression","source":{"name":"test tool"}}}),
        u64::MAX,
    );
    let second = decision(
        &mut app,
        target,
        json!({"kind":"question","question":{"question":"Explain","options":[]}}),
        u64::MAX,
    );
    let output = draw(&app, 100, 35);
    for expected in [
        "Permission needed",
        "test tool",
        "Read synthetic file",
        "Deny",
        "Allow once",
        "[Next]",
    ] {
        assert!(output.contains(expected), "{output}");
    }
    assert_eq!(
        app.interactions.borrow().answers[&(target, first)].option,
        Some(0)
    );
    key(&mut app, KeyCode::Right);
    assert_eq!(app.interactions.borrow().selected, Some((target, second)));
    assert!(app.interactions.borrow().displayed.is_none());
    draw(&app, 100, 35);
    assert_eq!(
        app.interactions.borrow().answers[&(target, second)].option,
        None
    );
    key(&mut app, KeyCode::Left);
    assert_eq!(app.interactions.borrow().selected, Some((target, first)));
    app.help = true;
    app.sync_interactions();
    assert!(!app.interactions.borrow().focused);
    assert!(app.interactions.borrow().selected.is_none());
    assert!(
        !app.interaction_input(&Event::Paste("not an answer".into()))
            .unwrap()
    );
    app.help = false;
    app.sync_interactions();
    assert_eq!(app.interactions.borrow().selected, Some((target, first)));
}

#[test]
fn expired_unknown_and_small_requests_never_publish_response_controls() {
    for request in [
        json!({"kind":"approval","approval":{}}),
        json!({"kind":"question","question":{"options":["One"]}}),
        json!({"kind":"future"}),
    ] {
        let (_fixture, mut app, target) = coverage_support::app();
        let id = decision(&mut app, target, request, 0);
        let output = draw(&app, 100, 35);
        assert!(output.contains("expired"), "{output}");
        assert!(
            app.interactions
                .borrow()
                .hits
                .iter()
                .all(|(_, c)| !matches!(
                    c,
                    Control::Confirm | Control::Cancel | Control::Choice(_)
                ))
        );
        key(&mut app, KeyCode::Enter);
        assert!(app.views[&target].pending.is_none());
        assert_eq!(app.interactions.borrow().selected, Some((target, id)));
        let output = draw(&app, 18, 8);
        assert!(output.contains("Enlarge"));
        assert!(app.interactions.borrow().displayed.is_none());
        assert!(app.interactions.borrow().hits.is_empty());
    }
}

#[test]
fn mouse_choices_clear_editor_and_resize_invalidate_geometry() {
    let (_fixture, mut app, target) = coverage_support::app();
    let id = decision(
        &mut app,
        target,
        json!({"kind":"question","question":{"question":"Choose","options":["One"]}}),
        u64::MAX,
    );
    draw(&app, 100, 35);
    let rect = app
        .interactions
        .borrow()
        .hits
        .iter()
        .find(|(_, c)| matches!(c, Control::Choice(None)))
        .unwrap()
        .0;
    let click = |rect: Rect| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })
    };
    app.interaction_input(&click(rect)).unwrap();
    assert!(app.interactions.borrow().answers[&(target, id)].editing);
    app.interaction_input(&Event::Paste("synthetic answer".into()))
        .unwrap();
    draw(&app, 100, 35);
    let clear = app
        .interactions
        .borrow()
        .hits
        .iter()
        .find(|(_, c)| matches!(c, Control::Clear))
        .unwrap()
        .0;
    app.interaction_input(&click(clear)).unwrap();
    assert_eq!(app.question_editor().unwrap().1, "");
    assert!(
        app.interaction_input(&Event::Paste("x".repeat(4097)))
            .is_err()
    );
    key(&mut app, KeyCode::PageDown);
    assert_eq!(app.interactions.borrow().scroll, 5);
    key(&mut app, KeyCode::PageUp);
    assert_eq!(app.interactions.borrow().scroll, 0);
    app.interaction_input(&Event::Resize(80, 20)).unwrap();
    assert!(app.interactions.borrow().hits.is_empty());
    assert!(app.interactions.borrow().displayed.is_none());
    assert!(app.question_editor().is_none());
}

#[test]
fn response_validation_rejects_mismatched_choices_controls_and_unknown_kinds() {
    let question = json!({"kind":"question","question":{"options":["One","Two"]}});
    for response in [
        json!({"status":"selected","index":0,"answer":"One"}),
        json!({"status":"custom","answer":"synthetic answer"}),
        json!({"status":"cancelled"}),
    ] {
        assert!(validate_response(&question, &response).is_ok());
    }
    for response in [
        json!({"status":"selected","index":0,"answer":"Two"}),
        json!({"status":"selected","index":9,"answer":"One"}),
        json!({"status":"selected","index":-1,"answer":"One"}),
        json!({"status":"custom","answer":"  "}),
        json!({"status":"custom","answer":"a\nb"}),
        json!({"status":"custom","answer":"x".repeat(4097)}),
        json!({"status":"invalid"}),
    ] {
        assert!(validate_response(&question, &response).is_err());
    }
    let approval = json!({"kind":"approval"});
    for response in [json!("approved"), json!("denied")] {
        assert!(validate_response(&approval, &response).is_ok());
    }
    assert!(validate_response(&approval, &json!({"status":"selected"})).is_err());
    assert!(validate_response(&json!({"kind":"future"}), &json!("approved")).is_err());
}

#[tokio::test]
async fn root_grant_review_shows_exact_scope_and_sends_owner_only_envelope() {
    let (_fixture, mut app, target) = coverage_support::app();
    let id = decision(
        &mut app,
        target,
        json!({"kind":"root_grant","root_grant":{"path":"/fixture/config","permission":"write","lifetime":"current_run"},"approval":{"action":"Grant Write filesystem access for CURRENT RUN ONLY (no children)","target":"/fixture/config","reason":"Configure requested integration"}}),
        u64::MAX,
    );
    let output = draw(&app, 110, 40);
    assert!(output.contains("CURRENT RUN ONLY"));
    assert!(output.contains("/fixture/config"));
    assert!(output.contains("Grant for current run"));
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    let command = app.views[&target]
        .pending
        .as_ref()
        .unwrap()
        .original
        .as_ref()
        .unwrap();
    match command.as_ref() {
        voyage_protocol::vessel::VoyageCommand::Respond {
            response,
            decision_id,
            ..
        } => {
            assert_eq!(*decision_id, id);
            assert_eq!(response, &json!({"root_grant":"approved"}));
        }
        _ => panic!("expected typed root consent"),
    }
}
