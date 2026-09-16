//! Real chooser input journeys; no host, environment, or browser effects.
use super::super::super::account_test_support::Fixture;
use super::*;
use crossterm::event::{KeyEvent, KeyEventKind};

fn setup(f: &Fixture) -> (App, Target) {
    let mut app = super::super::super::accounts::app_tests::app(f.0.path());
    let target = super::tests::app_fixture(&mut app);
    super::tests::picker(&mut app, target);
    app.inference.visible.set(true);
    (app, target)
}

fn key(app: &mut App, code: KeyCode) {
    assert!(
        app.model_chooser_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap()
    );
}

fn activate(app: &mut App, control: Control) {
    app.inference.picker.as_mut().unwrap().chooser.focus = control;
    key(app, KeyCode::Enter);
}

fn unchanged(app: &App, t: Target) {
    assert_eq!(app.views[&t].draft.text, "keep my draft");
    assert!(app.views[&t].pending.is_none());
    assert!(app.views[&t].snapshot.as_ref().unwrap().messages.is_empty());
    assert_eq!(
        app.inference_settings(Destination::Live(t)).unwrap().model,
        "current"
    );
    assert!(!app.accounts.open());
}

#[tokio::test]
async fn search_enter_stages_model_then_escape_leaves_live_settings_untouched() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    key(&mut app, KeyCode::Char('o'));
    key(&mut app, KeyCode::Char('t'));
    assert_eq!(app.inference.picker.as_ref().unwrap().query, "ot");
    key(&mut app, KeyCode::Enter);
    let p = app.inference.picker.as_ref().unwrap();
    assert_eq!(p.chooser.model, "ot");
    assert_eq!(p.chooser.focus, Control::Apply);
    assert_eq!(p.original.model, "current");
    unchanged(&app, t);
    key(&mut app, KeyCode::Esc);
    assert!(app.inference.picker.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn advanced_service_cycle_warns_without_applying() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    activate(&mut app, Control::Advanced);
    assert!(app.inference.picker.as_ref().unwrap().chooser.advanced);
    activate(&mut app, Control::Service);
    let p = app.inference.picker.as_ref().unwrap();
    assert_eq!(p.chooser.service.as_deref(), Some("standard"));
    assert!(p.notice.contains("cost more"));
    activate(&mut app, Control::Service);
    assert_eq!(
        app.inference
            .picker
            .as_ref()
            .unwrap()
            .chooser
            .service
            .as_deref(),
        Some("priority")
    );
    activate(&mut app, Control::Service);
    assert!(
        app.inference
            .picker
            .as_ref()
            .unwrap()
            .chooser
            .service
            .is_none()
    );
    unchanged(&app, t);
}

#[tokio::test]
async fn changing_model_requires_explicit_reset_of_advanced_preferences() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    activate(&mut app, Control::Thinking);
    assert_eq!(
        app.inference
            .picker
            .as_ref()
            .unwrap()
            .chooser
            .thinking
            .as_deref(),
        Some("low")
    );
    key(&mut app, KeyCode::Down);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().chooser.model,
        "other"
    );
    activate(&mut app, Control::Apply);
    let p = app.inference.picker.as_ref().unwrap();
    assert!(p.chooser.review);
    assert_eq!(p.chooser.focus, Control::Reset);
    assert!(p.notice.contains("Reset them or keep them"));
    unchanged(&app, t);
    activate(&mut app, Control::Reset);
    let p = app.inference.picker.as_ref().unwrap();
    assert!(!p.chooser.review);
    assert!(!p.chooser.keep);
    assert!(p.chooser.thinking.is_none());
    assert!(p.chooser.service.is_none());
    assert_eq!(p.chooser.model, "other");
    unchanged(&app, t);
}

#[tokio::test]
async fn keeping_preferences_is_invalidated_by_a_different_model_selection() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    activate(&mut app, Control::Thinking);
    key(&mut app, KeyCode::Down);
    activate(&mut app, Control::Apply);
    activate(&mut app, Control::Keep);
    let p = app.inference.picker.as_ref().unwrap();
    assert!(p.chooser.keep);
    assert!(!p.chooser.review);
    assert_eq!(p.chooser.thinking.as_deref(), Some("low"));
    key(&mut app, KeyCode::Up);
    let p = app.inference.picker.as_ref().unwrap();
    assert_eq!(p.chooser.model, "current");
    assert!(!p.chooser.keep);
    assert!(!p.chooser.review);
    unchanged(&app, t);
}

#[tokio::test]
async fn editing_thinking_after_keep_requires_new_review() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    activate(&mut app, Control::Thinking);
    key(&mut app, KeyCode::Down);
    activate(&mut app, Control::Apply);
    activate(&mut app, Control::Keep);
    activate(&mut app, Control::Thinking);
    let p = app.inference.picker.as_ref().unwrap();
    assert!(!p.chooser.keep);
    assert!(!p.chooser.review);
    assert_eq!(p.chooser.thinking.as_deref(), Some("high"));
    activate(&mut app, Control::Apply);
    assert!(app.inference.picker.as_ref().unwrap().chooser.review);
    unchanged(&app, t);
}

#[tokio::test]
async fn keyboard_focus_round_trip_includes_advanced_and_review_controls() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    activate(&mut app, Control::Advanced);
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().chooser.focus,
        Control::Thinking
    );
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().chooser.focus,
        Control::Service
    );
    key(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().chooser.focus,
        Control::Thinking
    );
    app.inference.picker.as_mut().unwrap().chooser.review = true;
    app.inference.picker.as_mut().unwrap().chooser.focus = Control::Service;
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().chooser.focus,
        Control::Reset
    );
    key(&mut app, KeyCode::Tab);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().chooser.focus,
        Control::Keep
    );
    unchanged(&app, t);
}

#[tokio::test]
async fn account_list_escape_returns_to_model_then_second_escape_closes() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    app.inference.picker.as_mut().unwrap().chooser.accounts_open = true;
    key(&mut app, KeyCode::Down);
    let p = app.inference.picker.as_ref().unwrap();
    assert_eq!(p.chooser.account_row, 0);
    assert_eq!(p.chooser.focus, Control::AccountRow(0));
    key(&mut app, KeyCode::Esc);
    assert!(!app.inference.picker.as_ref().unwrap().chooser.accounts_open);
    assert!(app.inference.account_load.is_none());
    unchanged(&app, t);
    key(&mut app, KeyCode::Esc);
    assert!(app.inference.picker.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn resize_invalidates_mouse_targets_and_blocks_unpainted_typing() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    app.inference
        .choices
        .borrow_mut()
        .push((Rect::new(1, 1, 8, 1), 1));
    app.inference
        .chooser_hits
        .borrow_mut()
        .push((Rect::new(1, 2, 8, 1), Control::Apply));
    assert!(!app.model_chooser_input(&Event::Resize(40, 12)).unwrap());
    assert!(!app.inference.visible.get());
    assert!(app.inference.choices.borrow().is_empty());
    assert!(app.inference.chooser_hits.borrow().is_empty());
    key(&mut app, KeyCode::Char('x'));
    assert!(app.inference.picker.as_ref().unwrap().query.is_empty());
    key(&mut app, KeyCode::Esc);
    assert!(app.inference.picker.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn key_release_does_not_change_selection_or_search() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    let mut event = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
    event.kind = KeyEventKind::Release;
    assert!(app.model_chooser_input(&Event::Key(event)).unwrap());
    let p = app.inference.picker.as_ref().unwrap();
    assert_eq!(p.selected, 0);
    assert_eq!(p.chooser.model, "current");
    assert!(p.query.is_empty());
    unchanged(&app, t);
}

#[tokio::test]
async fn utf8_search_limit_is_measured_in_bytes_and_backspace_removes_one_character() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    app.inference.picker.as_mut().unwrap().query = "a".repeat(254);
    key(&mut app, KeyCode::Char('é'));
    assert_eq!(app.inference.picker.as_ref().unwrap().query.len(), 256);
    key(&mut app, KeyCode::Char('b'));
    assert_eq!(app.inference.picker.as_ref().unwrap().query.len(), 256);
    key(&mut app, KeyCode::Backspace);
    assert_eq!(
        app.inference.picker.as_ref().unwrap().query,
        "a".repeat(254)
    );
    assert!(
        app.model_chooser_input(&Event::Paste("xyz".into()))
            .unwrap()
    );
    assert_eq!(app.inference.picker.as_ref().unwrap().query.len(), 254);
    unchanged(&app, t);
}

#[tokio::test]
async fn incarnation_change_closes_chooser_before_processing_input() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    app.views.get_mut(&t).unwrap().process.incarnation = Uuid::new_v4();
    key(&mut app, KeyCode::Down);
    assert!(app.inference.picker.is_none());
    assert!(app.status.contains("Conversation changed"));
    unchanged(&app, t);
}

#[tokio::test]
async fn initializing_picker_rejects_apply_without_creating_pending_command() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    let p = app.inference.picker.as_mut().unwrap();
    p.chooser.initializing = true;
    p.chooser.focus = Control::Apply;
    app.inference.visible.set(true);
    assert!(
        app.model_chooser_input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )))
        .is_ok()
    );
    assert!(
        app.inference
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("Select an account")
    );
    unchanged(&app, t);
    assert!(app.inference.catalog_job.is_none());
}

#[tokio::test]
async fn invalid_model_id_is_rejected_before_any_command_is_sent() {
    let f = Fixture::new();
    let (mut app, t) = setup(&f);
    let p = app.inference.picker.as_mut().unwrap();
    p.chooser.model = "two model ids".into();
    p.chooser.focus = Control::Apply;
    app.inference.visible.set(true);
    let _ = app
        .model_chooser_input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap();
    assert!(
        app.inference
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("Select one model ID")
    );
    unchanged(&app, t);
    assert!(app.inference.catalog_job.is_none());
}
