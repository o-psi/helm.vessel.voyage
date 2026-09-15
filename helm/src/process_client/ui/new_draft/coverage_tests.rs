use super::super::coverage_support;
use super::*;

fn setup() -> (super::super::account_test_support::Fixture, App, Uuid) {
    let (fixture, mut app, target) = coverage_support::app();
    std::fs::create_dir_all(&app.clients[target.route].directory).unwrap();
    let mut config = crate::Config::default();
    config.workspace = Some(fixture.0.path().into());
    app.new_chat_config = Some(config);
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let id = app.active_draft.unwrap();
    (fixture, app, id)
}

fn key(code: KeyCode) -> Event {
    Event::Key(crossterm::event::KeyEvent::new(code, KeyModifiers::NONE))
}

fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| app.draw_new_draft(frame, frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[tokio::test]
async fn empty_draft_is_reused_without_starting_a_process() {
    let (fixture, mut app, id) = setup();
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    assert_eq!(app.active_draft, Some(id));
    assert_eq!(app.new_drafts.len(), 1);
    assert_eq!(app.new_drafts[&id].navigation_title(), "New voyage");
    assert_eq!(app.new_drafts[&id].navigation_workspace(), fixture.0.path());
    assert!(app.new_drafts[&id].saved.start.is_none());
}

#[tokio::test]
async fn authored_draft_is_not_reused_and_titles_use_first_line() {
    let (fixture, mut app, id) = setup();
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("first line\nsecond line");
    assert_eq!(app.new_drafts[&id].navigation_title(), "first line");
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    assert_ne!(app.active_draft, Some(id));
    assert_eq!(app.new_drafts.len(), 2);
    assert_eq!(app.new_drafts[&id].composer.text, "first line\nsecond line");
}

#[tokio::test]
async fn draft_keyboard_edits_and_saves_without_sending() {
    let (_fixture, mut app, id) = setup();
    for event in [
        key(KeyCode::Char('a')),
        key(KeyCode::Char('b')),
        key(KeyCode::Left),
        key(KeyCode::Char('c')),
    ] {
        assert!(app.new_draft_input(&event).unwrap());
    }
    assert_eq!(app.new_drafts[&id].composer.text, "acb");
    assert_eq!(app.new_drafts[&id].saved.text, "acb");
    app.new_draft_input(&key(KeyCode::Backspace)).unwrap();
    app.new_draft_input(&key(KeyCode::Delete)).unwrap();
    assert_eq!(app.new_drafts[&id].saved.text, "a");
    assert!(app.new_drafts[&id].saved.start.is_none());
}

#[tokio::test]
async fn help_and_release_keys_leave_draft_untouched() {
    let (_fixture, mut app, id) = setup();
    let mut released = crossterm::event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    released.kind = crossterm::event::KeyEventKind::Release;
    assert!(app.new_draft_input(&Event::Key(released)).unwrap());
    app.new_draft_input(&key(KeyCode::F(1))).unwrap();
    assert!(app.status.contains("Draft commands:"));
    assert!(app.new_drafts[&id].composer.text.is_empty());
    app.draft_command(id, "/help").unwrap();
    assert!(app.status.contains("/discard"));
}

#[tokio::test]
async fn escape_returns_to_existing_voyage_and_inactive_input_is_ignored() {
    let (_fixture, mut app, id) = setup();
    assert_eq!(app.active_draft, Some(id));
    assert!(app.selected.is_none());
    app.new_draft_input(&key(KeyCode::Esc)).unwrap();
    assert!(app.active_draft.is_none());
    assert!(app.selected.is_some());
    assert!(!app.new_draft_input(&key(KeyCode::Char('x'))).unwrap());
}

#[tokio::test]
async fn busy_draft_rejects_commands_and_absorbs_edits() {
    let (_fixture, mut app, id) = setup();
    app.new_drafts.get_mut(&id).unwrap().busy = true;
    assert!(app.new_draft_input(&key(KeyCode::Char('x'))).unwrap());
    assert!(app.new_drafts[&id].composer.text.is_empty());
    assert!(app.draft_command(id, "/discard").is_err());
    assert!(app.draft_command(id, "/access approval").is_err());
    assert!(app.new_drafts.contains_key(&id));
}

#[tokio::test]
async fn access_commands_validate_and_persist_modes_without_starting() {
    let (_fixture, mut app, id) = setup();
    for (command, mode) in [
        ("read-only", crate::config::AccessMode::ReadOnly),
        ("approval", crate::config::AccessMode::Approval),
        ("unrestricted", crate::config::AccessMode::Unrestricted),
    ] {
        app.draft_command(id, &format!("/access {command}"))
            .unwrap();
        assert_eq!(
            app.new_drafts[&id].saved.config.as_ref().unwrap().access,
            Some(mode)
        );
    }
    assert!(app.draft_command(id, "/access bogus").is_err());
    assert!(app.new_drafts[&id].saved.start.is_none());
}

#[tokio::test]
async fn unsupported_commands_preserve_text_and_discard_removes_only_draft() {
    let (_fixture, mut app, id) = setup();
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("keep me");
    assert!(app.draft_command(id, "/unknown").is_err());
    assert_eq!(app.new_drafts[&id].composer.text, "keep me");
    app.draft_command(id, "/discard").unwrap();
    assert!(!app.new_drafts.contains_key(&id));
    assert!(app.active_draft.is_none());
    assert_eq!(app.views.len(), 1);
    assert_eq!(app.status, "Local draft discarded.");
}

#[tokio::test]
async fn draft_render_handles_empty_authored_and_busy_states() {
    let (_fixture, mut app, id) = setup();
    let empty = screen(&app, 100, 35);
    assert!(empty.contains("New voyage"));
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("rendered synthetic message");
    assert!(screen(&app, 100, 35).contains("rendered synthetic message"));
    app.new_drafts.get_mut(&id).unwrap().busy = true;
    assert!(!screen(&app, 100, 35).is_empty());
    for (width, height) in [(1, 1), (12, 5), (40, 12)] {
        let _ = screen(&app, width, height);
    }
    app.active_draft = None;
    assert!(screen(&app, 40, 12).trim().is_empty());
}
