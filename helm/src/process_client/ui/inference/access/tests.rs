use super::*;
#[tokio::test]
async fn access_picker_requires_visible_current_draft_and_preserves_authored_text() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let (fixture, mut app, target) = crate::process_client::ui::coverage_support::app();
    std::fs::create_dir_all(&app.clients[target.route].directory).unwrap();
    app.new_chat_config = Some(crate::Config {
        workspace: Some(fixture.0.path().into()),
        ..Default::default()
    });
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let id = app.active_draft.unwrap();
    app.new_draft_composer_mut(id)
        .unwrap()
        .insert_str("retained");
    app.inference.access.draft = Some((id, 0));
    app.composer_access_input(&Event::Key(KeyEvent::new(
        KeyCode::Down,
        KeyModifiers::NONE,
    )))
    .unwrap();
    app.composer_access_input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .unwrap();
    assert!(app.inference.access.draft.is_some());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 25)).unwrap();
    terminal.draw(|frame| app.draw_draft_access(frame)).unwrap();
    assert!(app.inference.access.visible.get());
    app.composer_access_input(&Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
    .unwrap();
    assert!(app.inference.access.draft.is_none());
    assert_eq!(app.new_draft_composer_mut(id).unwrap().text, "retained");
    app.inference.access.draft = Some((id, 0));
    app.composer_access_input(&Event::Resize(10, 5)).unwrap();
    assert!(!app.inference.access.visible.get());
    app.composer_access_input(&Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))
        .unwrap();
    assert!(app.inference.access.draft.is_none());
}
