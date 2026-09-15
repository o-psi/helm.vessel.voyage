use super::super::super::coverage_support;
use super::*;

#[test]
fn only_complete_authored_assistant_answers_offer_actions() {
    let mut message = Message {
        role: "assistant".into(),
        content: "answer".into(),
        ..Default::default()
    };
    assert!(eligible(&message));
    message.role = "user".into();
    assert!(!eligible(&message));
    message.role = "assistant".into();
    message.interrupted_attempt = Some(uuid::Uuid::new_v4());
    assert!(!eligible(&message));
    message.interrupted_attempt = None;
    message.operator_name = Some("shell".into());
    assert!(!eligible(&message));
    message.operator_name = None;
    message.content.clear();
    assert!(!eligible(&message));
}

#[tokio::test]
async fn action_focus_without_visible_answers_reports_no_actions() {
    let (_fixture, mut app, target) = coverage_support::app();
    let event = Event::Key(crossterm::event::KeyEvent::new(
        KeyCode::Char('m'),
        KeyModifiers::ALT,
    ));
    assert!(app.answer_actions_input(&event));
    assert!(app.status.contains("No saved answer actions visible"));
    assert!(
        app.views[&target]
            .transcript
            .borrow()
            .answer_selected
            .is_none()
    );
    assert!(!app.answer_actions_input(&Event::Resize(20, 10)));
    app.selected = None;
    assert!(!app.answer_actions_input(&event));
}
