use super::*;

#[test]
fn encodes_terminal_keys_without_text_transformation() {
    assert_eq!(
        encode_terminal_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
        Some(vec![3])
    );
    assert_eq!(
        encode_terminal_key(KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE)),
        Some("界".as_bytes().to_vec())
    );
    assert_eq!(
        encode_terminal_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(
        encode_terminal_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
        Some(b"\x1bx".to_vec())
    );
}

#[test]
fn detach_accepts_both_crossterm_control_encodings() {
    assert!(is_terminal_detach_key(KeyEvent::new(
        KeyCode::Char('t'),
        KeyModifiers::CONTROL
    )));
    assert!(is_terminal_detach_key(KeyEvent::new(
        KeyCode::Char(']'),
        KeyModifiers::CONTROL
    )));
    assert!(is_terminal_detach_key(KeyEvent::new(
        KeyCode::Char('5'),
        KeyModifiers::CONTROL
    )));
    assert!(is_terminal_detach_key(KeyEvent::new(
        KeyCode::Char('\u{14}'),
        KeyModifiers::NONE
    )));
    assert!(is_terminal_detach_key(KeyEvent::new(
        KeyCode::Char('\u{1d}'),
        KeyModifiers::NONE
    )));
}

#[tokio::test]
async fn attached_input_is_isolated_and_detach_does_not_close_terminal() {
    let directory = tempfile::tempdir().unwrap();
    let session = Session::new(directory.path().into(), "test-model".into());
    let mut app = App::new(session, Vec::new());
    let terminals = FakeTerminals::new();
    app.terminal_panel.attached_terminal = Some(terminals.id);
    app.terminal_panel.terminal_snapshot = Some(terminals.snapshot(terminals.id).await.unwrap());
    let key = KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE);
    handle_attached_key(
        terminals.id,
        key,
        &mut app.terminal_panel,
        &mut app.status,
        &terminals,
    )
    .await;
    assert_eq!(
        terminals.writes.lock().unwrap().as_slice(),
        &[b"p".to_vec()]
    );
    assert!(app.composer.text.is_empty());
    assert!(app.session.messages.is_empty());
    handle_attached_key(
        terminals.id,
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL),
        &mut app.terminal_panel,
        &mut app.status,
        &terminals,
    )
    .await;
    assert!(app.terminal_panel.attached_terminal.is_none());
    assert_eq!(
        terminals.writes.lock().unwrap().len(),
        1,
        "detach chord must not reach the PTY"
    );
    assert_eq!(
        terminals.list().await.unwrap()[0].state,
        TerminalState::Running
    );
}

#[tokio::test]
async fn attached_keystrokes_can_never_answer_agent_approvals() {
    let directory = tempfile::tempdir().unwrap();
    let session = Session::new(directory.path().into(), "test-model".into());
    let mut app = App::new(session, Vec::new());
    app.terminal_panel.attached_terminal = Some(TerminalId(uuid::Uuid::new_v4()));
    let (response, receive) = oneshot::channel();
    let store = SessionStore::new(directory.path().join("sessions"));
    handle_ui_event(
        UiEvent::Approval(ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            action: "shell".into(),
            target: "dangerous".into(),
            reason: "test".into(),
            response,
        }),
        &mut app,
        &store,
        &crate::terminal::NoInteractiveTerminals::default(),
    )
    .await
    .unwrap();
    assert_eq!(receive.await.unwrap(), ApprovalOutcome::Unavailable);
    assert!(app.approval.is_none());
}

#[tokio::test]
async fn attached_privacy_and_detach_hint_remain_visible_in_narrow_and_resized_views() {
    let terminals = FakeTerminals::new();
    let panel = super::super::terminals::TerminalPanel {
        attached_terminal: Some(terminals.id),
        terminal_snapshot: Some(terminals.attach(terminals.id).await.unwrap()),
        ..Default::default()
    };
    for width in [20, 40, 80, 160] {
        let backend = ratatui::backend::TestBackend::new(width, 8);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                super::super::terminals::draw_attached_terminal(frame, frame.area(), &panel)
            })
            .unwrap();
        let screen = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            screen.contains("Ctrl+T") && screen.contains("private"),
            "{screen}"
        );
    }
}
