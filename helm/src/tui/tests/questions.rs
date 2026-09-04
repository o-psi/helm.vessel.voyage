use super::*;

fn question_dialog() -> (
    QuestionDialog,
    oneshot::Receiver<crate::tools::QuestionAnswer>,
) {
    let (response, receive) = oneshot::channel();
    (
        QuestionDialog {
            request: QuestionRequest {
                question: crate::tools::Question {
                    question: "Which format?".into(),
                    options: vec!["JSON".into(), "Markdown".into()],
                },
                response,
            },
            selected: 0,
            custom: Composer::default(),
            scroll: None,
        },
        receive,
    )
}

#[test]
fn questions_keyboard_selection_custom_editing_paste_and_limits() {
    use crate::tools::QuestionAnswer;
    let (mut dialog, _receive) = question_dialog();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    dialog.insert("not on custom");
    assert!(dialog.custom.text.is_empty());
    dialog.key(key(KeyCode::Down));
    assert_eq!(
        dialog.key(key(KeyCode::Enter)),
        Some(QuestionAnswer::Selected {
            index: 1,
            answer: "Markdown".into()
        })
    );
    dialog.key(key(KeyCode::Tab));
    assert!(dialog.key(key(KeyCode::Enter)).is_none());
    dialog.insert("日本語\n\u{001b}🛶");
    assert_eq!(dialog.custom.text, "日本語🛶");
    dialog.key(key(KeyCode::Left));
    dialog.key(key(KeyCode::Backspace));
    dialog.key(key(KeyCode::Char('文')));
    assert_eq!(dialog.custom.text, "日本文🛶");
    assert_eq!(
        dialog.key(key(KeyCode::Enter)),
        Some(QuestionAnswer::Custom {
            answer: "日本文🛶".into()
        })
    );
    dialog.key(key(KeyCode::Up));
    dialog.key(key(KeyCode::Down));
    assert_eq!(dialog.custom.text, "日本文🛶");
    dialog.insert(&"x".repeat(5000));
    assert!(dialog.custom.text.len() <= crate::tools::MAX_ANSWER_BYTES);
    assert_eq!(
        dialog.key(key(KeyCode::Esc)),
        Some(QuestionAnswer::Cancelled)
    );
}

#[tokio::test]
async fn questions_modal_routes_keys_without_touching_draft_or_other_views() {
    use crate::tools::QuestionAnswer;
    let mut app = App::new(Session::new(PathBuf::from("/tmp"), "test".into()), vec![]);
    app.composer.insert_str("unsent draft");
    app.shortcut_help = true;
    app.model_panel.model_picker = true;
    let (dialog, receive) = question_dialog();
    app.question = Some(dialog);
    for code in [
        KeyCode::F(1),
        KeyCode::Char('x'),
        KeyCode::Down,
        KeyCode::Enter,
    ] {
        assert!(handle_question_key(
            KeyEvent::new(code, KeyModifiers::NONE),
            &mut app
        ));
    }
    assert_eq!(
        receive.await.unwrap(),
        QuestionAnswer::Selected {
            index: 1,
            answer: "Markdown".into()
        }
    );
    assert_eq!(app.composer.text, "unsent draft");
    assert!(app.shortcut_help && app.model_panel.model_picker);
    assert!(app.question.is_none());
    let (dialog, receive) = question_dialog();
    app.question = Some(dialog);
    app.cancel();
    assert_eq!(receive.await.unwrap(), QuestionAnswer::Cancelled);
    let (dialog, receive) = question_dialog();
    drop(receive);
    app.question = Some(dialog);
    assert!(handle_question_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app
    ));
    assert!(app.question.is_none());
    assert_eq!(app.composer.text, "unsent draft");
}

#[tokio::test]
async fn questions_reject_concurrent_requests_and_attached_terminal_input() {
    use crate::tools::QuestionAnswer;
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.terminal_panel.attached_terminal = Some(terminals.id);
    let (dialog, receive) = question_dialog();
    handle_ui_event(
        UiEvent::Question(dialog.request),
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    assert_eq!(receive.await.unwrap(), QuestionAnswer::Unavailable);
    assert!(app.question.is_none());
    app.terminal_panel.attached_terminal = None;
    let (dialog, first) = question_dialog();
    handle_ui_event(
        UiEvent::Question(dialog.request),
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    let (dialog, second) = question_dialog();
    handle_ui_event(
        UiEvent::Question(dialog.request),
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    assert_eq!(second.await.unwrap(), QuestionAnswer::Unavailable);
    app.cancel();
    assert_eq!(first.await.unwrap(), QuestionAnswer::Cancelled);
    let (dialog, stale) = question_dialog();
    drop(stale);
    handle_ui_event(
        UiEvent::Question(dialog.request),
        &mut app,
        &store,
        &terminals,
    )
    .await
    .unwrap();
    assert!(app.question.is_none());
    assert!(terminals.writes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn questions_bridge_delivers_answers_and_handles_closed_frontend() {
    let (bridge, mut events) = bridge();
    let (dialog, _) = question_dialog();
    let task_bridge = bridge.clone();
    let question = dialog.request.question;
    let task = tokio::spawn(async move { task_bridge.ask_question(&question).await });
    let Some(UiEvent::Question(request)) = events.recv().await else {
        panic!("missing question event")
    };
    assert_eq!(request.question.question, "Which format?");
    request
        .response
        .send(crate::tools::QuestionAnswer::Custom {
            answer: "CSV".into(),
        })
        .unwrap();
    assert_eq!(
        task.await.unwrap(),
        crate::tools::QuestionAnswer::Custom {
            answer: "CSV".into()
        }
    );
    drop(events);
    let (dialog, _) = question_dialog();
    assert_eq!(
        bridge.ask_question(&dialog.request.question).await,
        crate::tools::QuestionAnswer::Unavailable
    );
}

#[test]
fn questions_render_safely_and_remain_visible_over_other_views_at_all_sizes() {
    use ratatui::backend::TestBackend;
    let mut app = App::new(Session::new(PathBuf::from("/tmp"), "test".into()), vec![]);
    let (mut dialog, _receive) = question_dialog();
    dialog.request.question.question = "Untrusted \u{001b}[2J 日本語 é ".repeat(30);
    dialog.selected = 2;
    dialog.insert("custom-text");
    app.question = Some(dialog);
    app.model_panel.model_picker = true;
    app.shortcut_help = true;
    for (width, height) in [(1, 1), (20, 6), (32, 10), (80, 24), (120, 40)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!rendered.contains('\u{001b}'));
        if width >= 32 {
            assert!(rendered.contains("custom-text"));
        }
    }
}
