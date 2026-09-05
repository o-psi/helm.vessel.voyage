use super::*;

#[test]
fn model_picker_filters_and_renders_manual_fallback() {
    let mut app = App::new(
        Session::new(PathBuf::from("/tmp"), "gpt-current".into()),
        Vec::new(),
    );
    app.model_panel.model_picker = true;
    app.model_panel.models = vec![
        ModelInfo::minimal("gpt-fast"),
        ModelInfo::minimal("gpt-careful"),
    ];
    app.model_panel.model_filter.insert_str("care");
    assert_eq!(filtered_models(&app.model_panel)[0].id, "gpt-careful");
    let backend = ratatui::backend::TestBackend::new(60, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("gpt-careful"));
    assert!(rendered.contains("current: gpt-current"));
}

#[tokio::test]
async fn tool_calls_remain_in_conversation_order_live_and_after_completion() {
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().join("sessions"));
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let terminals = FakeTerminals::new();
    for index in 0..4 {
        handle_ui_event(
            UiEvent::Agent(AgentEvent::AssistantText(format!("before-{index}"))),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        handle_ui_event(
            UiEvent::Agent(AgentEvent::ToolStarted {
                name: format!("tool-{index}"),
                arguments: serde_json::json!({"index": index}),
            }),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        let pending = transcript(&app, 120).to_string();
        assert!(
            pending.find(&format!("before-{index}")).unwrap()
                < pending.find(&format!("tool-{index}")).unwrap()
        );
        handle_ui_event(
            UiEvent::Agent(AgentEvent::ToolFinished {
                name: format!("tool-{index}"),
                result: format!("result-{index}"),
                success: index != 2,
            }),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
    }
    app.streaming_response = "final answer".into();
    let live = transcript(&app, 120).to_string();
    assert!(live.contains("tool-0"));
    for index in 1..4 {
        assert!(live.contains(&format!("tool-{index}")));
        assert!(live.contains(&format!("result-{index}")));
    }
    assert!(live.contains("✗ tool-2"));
    assert!(live.find("result-1").unwrap() < live.find("before-2").unwrap());
    assert!(live.find("result-3").unwrap() < live.find("final answer").unwrap());
    app.session.messages.append(&mut app.live_messages);
    app.session.messages.push(crate::Message::new(
        Role::Assistant,
        app.streaming_response.clone(),
    ));
    app.streaming_response.clear();
    store.save(&mut app.session).await.unwrap();
    let mut restored = App::new(store.list().await.unwrap().remove(0), vec![]);
    let saved = transcript(&restored, 120).to_string();
    assert!(saved.contains("tool-0"));
    assert!(saved.find("result-1").unwrap() < saved.find("before-2").unwrap());
    restored.show_activity = true;
    assert!(transcript(&restored, 120).to_string().contains("tool-0"));
}

#[test]
fn each_stretch_between_assistant_messages_keeps_its_last_three_calls() {
    let mut app = App::new(Session::new(PathBuf::from("/tmp"), "test".into()), vec![]);
    for group in 0..3 {
        let mut reply = crate::Message::new(Role::Assistant, format!("reply-{group}"));
        // Persisted responses can hold multiple calls in one message;
        // live events arrive as separate messages with empty content.
        for index in 0..5 {
            let call = crate::model::ToolCall {
                id: format!("{group}-{index}"),
                name: format!("call-{group}-{index}"),
                arguments: serde_json::json!({}),
            };
            if group == 1 {
                if index == 0 {
                    app.live_messages.push(reply.clone());
                }
                let mut live = crate::Message::new(Role::Assistant, "");
                live.tool_calls.push(call);
                app.live_messages.push(live);
            } else {
                reply.tool_calls.push(call);
            }
        }
        if group != 1 {
            app.live_messages.push(reply);
        }
    }
    let compact = transcript(&app, 120).to_string();
    for group in 0..3 {
        for index in 0..5 {
            assert_eq!(
                compact.contains(&format!("call-{group}-{index}")),
                index >= 2
            );
        }
    }
    assert!(compact.find("call-0-4").unwrap() < compact.find("reply-1").unwrap());
    assert!(compact.find("call-1-4").unwrap() < compact.find("reply-2").unwrap());
    app.show_activity = true;
    let expanded = transcript(&app, 120).to_string();
    for group in 0..3 {
        for index in 0..5 {
            assert!(expanded.contains(&format!("call-{group}-{index}")));
        }
    }
    app.show_activity = false;
    assert_eq!(compact, transcript(&app, 120).to_string());
}

#[test]
fn working_indicator_animates_and_rotates_every_four_seconds() {
    let first = working_message(Duration::ZERO);
    let next_frame = working_message(Duration::from_millis(100));
    assert_ne!(first.chars().next(), next_frame.chars().next());
    assert_eq!(
        first.split_once(' ').unwrap().1,
        next_frame.split_once(' ').unwrap().1
    );
    assert_eq!(first, working_message(Duration::from_secs(3)));
    assert_ne!(first, working_message(Duration::from_secs(4)));
}

#[tokio::test]
async fn shift_enter_adds_newlines_without_dispatching_messages_or_todos() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let id = AgentId(Uuid::new_v4());
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let (tx, mut rx) = mpsc::unbounded_channel();
    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Message {
        target: id,
        follow_up: false,
    });
    app.supervisor_panel.supervisor_input.insert_str("first");
    handle_supervisor_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        &mut app.supervisor_panel,
        &mut app.status,
        &tx,
        supervisor.clone(),
    )
    .await;
    assert_eq!(app.supervisor_panel.supervisor_input.text, "first\n");
    assert!(supervisor.actions.lock().unwrap().is_empty());
    app.todo_panel.todo_mode = Some(TodoMode::Input {
        target: None,
        action: TodoInput::Add,
    });
    app.todo_panel.todo_input.insert_str("first");
    handle_todo_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        &mut app.todo_panel,
        &tx,
        todo_store(&directory),
    )
    .await;
    assert_eq!(app.todo_panel.todo_input.text, "first\n");
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn active_run_composer_sends_durable_steering_and_stays_editable() {
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().join("sessions"));
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let agent = Arc::new(navigation_agent_for_conversation(&directory));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, _rx) = mpsc::unbounded_channel();
    let (steering, _steering_input) = crate::agent::steering_channel(4);
    let cancel = tokio_util::sync::CancellationToken::new();
    app.running = Some(Running {
        task: tokio::spawn(std::future::pending()),
        cancel,
        steering,
    });

    for key in [
        KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
    ] {
        handle_key(
            key,
            &mut app,
            &agent,
            &store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.composer.text, "go\n界");
    handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &agent,
        &store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();

    assert!(app.composer.text.is_empty());
    assert!(app.status.contains("Steering queued"));
    let saved = store.load(app.session.id).await.unwrap();
    assert_eq!(saved.messages.last().unwrap().content, "go\n界");

    let backend = ratatui::backend::TestBackend::new(60, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("Enter send"));
}

struct ConversationNoRequests;

#[async_trait]
impl crate::provider::Provider for ConversationNoRequests {
    async fn complete(
        &self,
        _: crate::model::ModelRequest,
    ) -> Result<crate::model::ModelResponse, crate::provider::ProviderError> {
        panic!("UI-only steering test must not execute the provider")
    }
}

fn navigation_agent_for_conversation(directory: &tempfile::TempDir) -> Agent {
    Agent::new(
        Box::new(ConversationNoRequests),
        crate::tools::ToolRegistry::default(),
        crate::tools::ToolContext {
            policy: Arc::new(
                crate::policy::Policy::new(
                    &crate::config::Config::default(),
                    directory.path().into(),
                )
                .unwrap(),
            ),
            approver: Arc::new(crate::tools::UnattendedApprover { allow: false }),
            timeout: Duration::from_secs(1),
            max_output_bytes: 4096,
            environment: Default::default(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Attended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        },
        Arc::new(crate::agent::SilentSink),
        "test".into(),
        "system".into(),
        100,
        None,
    )
}

#[test]
fn renders_small_terminal_without_panicking() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let app = App::new(session, Vec::new());
    let backend = ratatui::backend::TestBackend::new(60, 16);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered = terminal.backend().buffer().content();
    assert!(rendered.iter().any(|cell| cell.symbol() == "H"));
}

#[test]
fn transcript_renders_only_assistant_content_as_markdown() {
    let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "**user literal**"));
    session
        .messages
        .push(crate::Message::new(Role::Assistant, "# Heading\n\n`code`"));
    session
        .messages
        .push(crate::Message::new(Role::Tool, "# tool literal"));
    let app = App::new(session, Vec::new());
    let rendered = transcript(&app, 40);
    let symbols = rendered
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(symbols.contains("**user literal**"));
    assert!(symbols.contains("Heading"));
    assert!(!symbols.contains("# tool literal"));
    assert!(
        rendered
            .lines
            .iter()
            .flat_map(|line| &line.spans)
            .any(|span| span.style.add_modifier.contains(Modifier::BOLD))
    );
}

#[test]
fn submitted_prompt_is_visible_while_activity_is_opt_in() {
    let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "inspect this now"));
    let mut app = App::new(session, Vec::new());
    app.activity.push("▶ shell: noisy internal detail".into());

    let hidden = transcript(&app, 60).to_string();
    assert!(hidden.contains("inspect this now"));
    assert!(!hidden.contains("noisy internal detail"));

    app.show_activity = true;
    let visible = transcript(&app, 60).to_string();
    assert!(visible.contains("noisy internal detail"));
}

#[tokio::test]
async fn failed_run_does_not_remove_submitted_prompt_from_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut session = Session::new(directory.path().into(), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "keep this request"));
    let mut app = App::new(session, Vec::new());
    let store = SessionStore::new(directory.path().join("sessions"));
    store.save(&mut app.session).await.unwrap();

    handle_ui_event(
        UiEvent::Finished(Err("provider unavailable".into())),
        &mut app,
        &store,
        &crate::terminal::NoInteractiveTerminals::default(),
    )
    .await
    .unwrap();

    assert_eq!(app.session.messages.last().unwrap().role, Role::User);
    assert_eq!(
        app.session.messages.last().unwrap().content,
        "keep this request"
    );
}

#[test]
fn no_color_markdown_theme_has_no_foreground_or_background_colors() {
    let theme = markdown_theme_for(true);
    assert_eq!(theme.text, Color::Reset);
    assert_eq!(theme.heading, Color::Reset);
    assert_eq!(theme.link, Color::Reset);
    assert_eq!(theme.code, Color::Reset);
    assert_eq!(theme.code_background, Color::Reset);
    assert_eq!(theme.quote, Color::Reset);
    assert_eq!(theme.table_header, Color::Reset);
    assert_eq!(theme.warning, Color::Reset);
}

#[test]
fn markdown_full_screen_buffer_uses_borderless_chat_regions() {
    let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    session
        .messages
        .push(crate::Message::new(Role::User, "# literal user"));
    session.messages.push(crate::Message::new(
        Role::Assistant,
        "# Rendered\n\n| A | B |\n|---|---|\n| one | two |",
    ));
    let app = App::new(session, Vec::new());
    let backend = ratatui::backend::TestBackend::new(48, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let rows = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    let snapshot = rows.join("\n");
    assert!(snapshot.contains("# literal user"));
    assert!(snapshot.contains("Rendered"));
    assert!(snapshot.contains("one"));
    assert!(!snapshot.contains("Conversation"));
    assert!(!snapshot.contains("Prompt"));
    assert!(rows.iter().all(|row| row.chars().count() == 48));
}

#[test]
fn global_ctrl_shortcuts_are_hidden_until_f1_help_is_opened() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let mut app = App::new(session, Vec::new());
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let normal: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(normal.contains("F1 shortcuts"));
    assert!(!normal.contains("Ctrl+D"));
    assert!(!normal.contains("^D todos"));

    assert!(handle_shortcut_help_key(
        KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
        &mut app,
    ));
    assert!(app.shortcut_help);
    assert!(handle_shortcut_help_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &mut app,
    ));
    assert!(app.composer.text.is_empty());
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let help: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(help.contains("Conversation shortcuts"));
    assert!(help.contains("Ctrl+D: todos"));

    assert!(handle_shortcut_help_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut app,
    ));
    assert!(!app.shortcut_help);
}

#[test]
fn incomplete_markdown_stream_is_stable_and_canonical() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let mut app = App::new(session, Vec::new());
    let source = "## Live\n\n```rust\nfn main() {}\n```";
    for character in source.chars() {
        app.streaming_response.push(character);
        let rendered = transcript(&app, 24);
        assert!(rendered.lines.len() < 100);
    }
    assert_eq!(app.streaming_response, source);
    let rendered = transcript(&app, 24);
    let symbols = rendered
        .lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(symbols.contains("fn main() {}"));
}

#[test]
fn manual_scroll_anchor_survives_stream_growth_and_resize() {
    let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    session.messages.push(crate::Message::new(
        Role::Assistant,
        (0..40)
            .map(|line| format!("line {line}\n"))
            .collect::<String>(),
    ));
    let mut app = App::new(session, Vec::new());
    app.conversation_width = 30;
    app.conversation_height = 8;
    app.scroll = 7;
    let before_height = transcript_height(&app, app.conversation_width);
    let before_top = before_height
        .saturating_sub(app.conversation_height)
        .saturating_sub(app.scroll as usize);
    app.streaming_response
        .push_str("new streamed line\nsecond line");
    preserve_manual_anchor(&mut app, before_height);
    let grown_top = transcript_height(&app, app.conversation_width)
        .saturating_sub(app.conversation_height)
        .saturating_sub(app.scroll as usize);
    assert_eq!(grown_top, before_top);
    resize_conversation(&mut app, 18, 12);
    let resized_top = transcript_height(&app, app.conversation_width)
        .saturating_sub(app.conversation_height)
        .saturating_sub(app.scroll as usize);
    assert_eq!(resized_top, before_top);
}

#[test]
fn mouse_wheel_scrolls_conversation_within_transcript_bounds() {
    let mut session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    session.messages.push(crate::Message::new(
        Role::Assistant,
        (0..40)
            .map(|line| format!("line {line}\n"))
            .collect::<String>(),
    ));
    let mut app = App::new(session, Vec::new());
    app.conversation_width = 30;
    app.conversation_height = 8;
    let wheel_up = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    };
    let wheel_down = MouseEvent {
        kind: MouseEventKind::ScrollDown,
        ..wheel_up
    };

    handle_mouse(wheel_up, &mut app);
    assert_eq!(app.scroll, 3);
    for _ in 0..100 {
        handle_mouse(wheel_up, &mut app);
    }
    assert_eq!(app.scroll, max_conversation_scroll(&app));
    handle_mouse(wheel_down, &mut app);
    assert_eq!(app.scroll, max_conversation_scroll(&app) - 3);

    app.show_sessions = true;
    let previous = app.scroll;
    handle_mouse(wheel_down, &mut app);
    assert_eq!(app.scroll, previous);
}

#[test]
fn composer_has_a_separator_above_its_input_area() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let app = App::new(session, Vec::new());
    let backend = ratatui::backend::TestBackend::new(48, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let buffer = terminal.backend().buffer();
    let separator = (0..buffer.area.width)
        .map(|x| buffer[(x, 24)].symbol())
        .collect::<String>();
    assert!(separator.contains("Shift+Enter newline"));
    assert!(separator.ends_with('─'));
}

#[test]
fn renders_tiny_terminal_with_resize_guidance() {
    let session = Session::new(PathBuf::from("/tmp"), "test-model".into());
    let app = App::new(session, Vec::new());
    let backend = ratatui::backend::TestBackend::new(20, 5);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("Helm"));
}

#[tokio::test]
async fn title_results_preserve_manual_names_and_reject_stale_sessions_and_turns() {
    let directory = tempfile::tempdir().unwrap();
    let store = SessionStore::new(directory.path().into());
    let mut app = App::new(Session::new(directory.path().into(), "main".into()), vec![]);
    app.session.record_completed_turn();
    store.save(&mut app.session).await.unwrap();
    let id = app.session.id;
    let original = app.session.display_name();
    for (session_id, completed_runs) in [(Uuid::new_v4(), 1), (id, 0)] {
        handle_ui_event(
            UiEvent::TitleReady {
                session_id,
                completed_runs,
                result: Some(crate::titles::TitleResult {
                    title: Some("Stale".into()),
                    usage: Default::default(),
                }),
            },
            &mut app,
            &store,
            &crate::terminal::NoInteractiveTerminals::default(),
        )
        .await
        .unwrap();
        assert_eq!(app.session.display_name(), original);
    }
    handle_ui_event(
        UiEvent::TitleReady {
            session_id: id,
            completed_runs: 1,
            result: Some(crate::titles::TitleResult {
                title: Some("Generated title".into()),
                usage: crate::model::Usage {
                    input_tokens: 3,
                    output_tokens: 1,
                },
            }),
        },
        &mut app,
        &store,
        &crate::terminal::NoInteractiveTerminals::default(),
    )
    .await
    .unwrap();
    assert_eq!(
        store.load(id).await.unwrap().display_name(),
        "Generated title"
    );
    assert_eq!(app.session.model, "main");
    assert!(app.session.messages.is_empty());
    app.session.set_name("My name".into());
    handle_ui_event(
        UiEvent::TitleReady {
            session_id: id,
            completed_runs: 1,
            result: Some(crate::titles::TitleResult {
                title: Some("Late".into()),
                usage: Default::default(),
            }),
        },
        &mut app,
        &store,
        &crate::terminal::NoInteractiveTerminals::default(),
    )
    .await
    .unwrap();
    assert_eq!(app.session.display_name(), "My name");
}

#[tokio::test]
async fn unavailable_title_model_does_not_send_a_repaint_event() {
    let directory = tempfile::tempdir().unwrap();
    let agent = Arc::new(navigation_agent_for_conversation(&directory));
    let mut app = App::new(Session::new(directory.path().into(), "main".into()), vec![]);
    app.session.record_completed_turn();
    let (tx, mut rx) = mpsc::unbounded_channel();
    start_title_job(&mut app, &agent, &tx);
    tokio::time::timeout(
        Duration::from_secs(1),
        &mut app.title_job.as_mut().unwrap().task,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(rx.try_recv().is_err());
    assert!(!app.is_running());
}
