//! Cross-module routing regressions: feature extraction must not change precedence.
use super::*;

struct NoRequests;

#[tokio::test]
async fn tool_details_shortcut_expands_collapses_and_respects_approval_input() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("keep draft");
    let mut message = crate::Message::new(Role::Assistant, "Inspect output");
    message.tool_calls.push(crate::model::ToolCall {
        id: "output".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command":"printf output"}),
    });
    app.session.messages.push(message);
    let output = format!(
        "exit: 0\nstdout:\n{}\nTAIL_SENTINEL\nstderr:\n",
        "line\n".repeat(30)
    );
    app.session
        .messages
        .push(crate::Message::tool("output", output));
    let before = serde_json::to_string(&app.session.messages).unwrap();
    assert!(!transcript(&app, 80).to_string().contains("TAIL_SENTINEL"));
    let key = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    handle_key(
        key,
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(app.tool_details);
    assert!(transcript(&app, 80).to_string().contains("TAIL_SENTINEL"));
    resize_conversation(&mut app, 24, 8);
    scroll_conversation(&mut app, 8);
    assert!(app.scroll > 0);
    handle_key(
        key,
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(!app.tool_details);
    assert!(app.scroll <= max_conversation_scroll(&app));
    assert_eq!(app.composer.text, "keep draft");
    assert_eq!(
        before,
        serde_json::to_string(&app.session.messages).unwrap()
    );
    let (response, _answer) = oneshot::channel();
    app.approval = Some(ApprovalRequest {
        id: Uuid::new_v4(),
        action: "shell".into(),
        target: "approval".into(),
        reason: "test".into(),
        response,
    });
    handle_key(
        key, &mut app, &agent, &mut store, &tx, &terminals, supervisor, todos,
    )
    .await
    .unwrap();
    assert!(!app.tool_details);
    assert!(app.approval.is_some());
    assert!(rx.try_recv().is_err());
    store.save(&mut app.session).await.unwrap();
    let mut restored = App::new(store.list().await.unwrap().remove(0), vec![]);
    restored.tool_details = true;
    assert!(
        transcript(&restored, 80)
            .to_string()
            .contains("TAIL_SENTINEL")
    );
}
#[async_trait]
impl crate::provider::Provider for NoRequests {
    async fn complete(
        &self,
        _: crate::model::ModelRequest,
    ) -> Result<crate::model::ModelResponse, crate::provider::ProviderError> {
        panic!("UI-only navigation must not request model execution")
    }
}

pub(super) fn navigation_agent(directory: &tempfile::TempDir) -> Arc<Agent> {
    Arc::new(Agent::new(
        Box::new(NoRequests),
        crate::tools::ToolRegistry::default(),
        crate::tools::ToolContext {
            github: None,
            completion: None,
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
    ))
}

#[tokio::test]
async fn central_router_preserves_modal_and_panel_precedence() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let (steering, _receiver) = crate::agent::steering_channel(4);
    app.running = Some(Running {
        task: tokio::spawn(std::future::pending()),
        cancel: tokio_util::sync::CancellationToken::new(),
        steering,
    });
    app.composer.insert_str("unsent λ");
    app.shortcut_help = true;
    app.model_panel.model_picker = true;
    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Message {
        target: AgentId(Uuid::new_v4()),
        follow_up: false,
    });
    open_todo_input(&mut app.todo_panel, None, TodoInput::Add, "");
    let (response, answer) = oneshot::channel();
    app.question = Some(QuestionDialog {
        request: QuestionRequest {
            question: crate::tools::Question {
                question: "Format?".into(),
                options: vec!["JSON".into(), "Text".into()],
            },
            response,
        },
        selected: 0,
        custom: Composer::default(),
        scroll: None,
    });
    handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(
        answer.await.unwrap(),
        crate::tools::QuestionAnswer::Selected { index: 0, .. }
    ));
    assert!(app.shortcut_help && app.model_panel.model_picker);
    assert!(app.model_panel.model_filter.text.is_empty());

    let (response, answer) = oneshot::channel();
    app.approval = Some(ApprovalRequest {
        id: Uuid::new_v4(),
        action: "write".into(),
        target: "fixture".into(),
        reason: "test".into(),
        response,
    });
    handle_key(
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(answer.await.unwrap(), ApprovalOutcome::Approved);
    assert!(app.shortcut_help);

    app.terminal_panel.attached_terminal = Some(terminals.id);
    handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(*terminals.writes.lock().unwrap(), vec![b"x".to_vec()]);
    handle_key(
        KeyEvent::new(KeyCode::Char(']'), KeyModifiers::CONTROL),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(app.terminal_panel.attached_terminal.is_none());
    assert!(app.shortcut_help);
    handle_key(
        KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(!app.shortcut_help);

    handle_key(
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(app.model_panel.model_filter.text, "m");
    assert!(app.supervisor_panel.supervisor_input.text.is_empty());
    app.model_panel.model_picker = false;
    handle_key(
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(app.supervisor_panel.supervisor_input.text, "s");
    assert!(app.todo_panel.todo_input.text.is_empty());
    app.supervisor_panel.supervisor_mode = None;
    handle_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert_eq!(app.todo_panel.todo_input.text, "t");
    assert_eq!(app.composer.text, "unsent λ");
    assert!(app.session.messages.is_empty());
    assert!(rx.try_recv().is_err(), "navigation must not dispatch work");
}

#[test]
fn palette_completion_requires_only_read_only_inputs() {
    let workspace = PathBuf::from(".");
    let models = vec![ModelInfo::minimal("fixture-model")];
    let context = PaletteContext {
        input: "/model fixture",
        running: false,
        dismissed: false,
        selected: 0,
        models: &models,
        sessions: &[],
        workspace: &workspace,
    };
    let items = slash_palette_items(&context);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].completion, "/model fixture-model");
    assert_eq!(models[0].id, "fixture-model");
    // Command-name filtering remains suppressed during execution or dismissal.
    assert!(
        slash_palette_matches(&PaletteContext {
            input: "/",
            running: true,
            ..context
        })
        .is_empty()
    );
    assert!(
        slash_palette_matches(&PaletteContext {
            input: "/",
            dismissed: true,
            ..context
        })
        .is_empty()
    );
}

#[tokio::test]
async fn todo_panel_can_edit_without_application_or_conversation_state() {
    let directory = tempfile::tempdir().unwrap();
    let store = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut first = TodoPanel::default();
    let second = TodoPanel::default();
    open_todo_input(&mut first, None, TodoInput::Add, "first");
    handle_todo_key(
        KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE),
        &mut first,
        &tx,
        store,
    )
    .await;
    assert_eq!(first.todo_input.text, "first界");
    assert!(second.todo_input.text.is_empty());
    assert!(second.todo_mode.is_none());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn new_and_branch_shortcuts_rebind_execution_owner() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let unbound = SessionStore::new(directory.path().join("sessions"));
    let mut session = Session::new(directory.path().into(), "test".into());
    let mut store = unbound.with_execution(session.id).await.unwrap();
    store.save(&mut session).await.unwrap();
    let original_id = session.id;
    let mut app = App::new(session, vec![]);
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, _rx) = mpsc::unbounded_channel();
    handle_key(
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    let next_id = app.session.id;
    assert_ne!(next_id, original_id);
    assert_eq!(store.owned_session_id(), Some(next_id));
    unbound.with_execution(original_id).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), unbound.with_execution(next_id))
            .await
            .is_err()
    );
    handle_key(
        KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    let branch_id = app.session.id;
    assert_eq!(app.session.parent_id, Some(next_id));
    assert_eq!(store.owned_session_id(), Some(branch_id));
    unbound.with_execution(next_id).await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), unbound.with_execution(branch_id))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn workflow_form_keeps_draft_and_yields_to_question_approval_and_pty() {
    let directory = tempfile::tempdir().unwrap();
    let repo = directory.path().join(".helm/workflows");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("review.toml"), "schema_version=1\nid='review'\nversion='1'\ndescription='Review'\nprompt='Review {{topic}}'\n[parameters.topic]\ntype='string'\nrequired=true\n").unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("retained draft 雪");
    handle_command(
        "/workflow review --scope repository",
        &mut app,
        &mut store,
        Some(&agent),
        Some(&tx),
    )
    .await
    .unwrap();
    let session_id = app.session.id;
    // Loading owns ordinary shortcuts. A late discovery cannot find a different
    // session or a PTY/session picker opened through those same key events.
    for key in ['t', 's', 'n', 'm'] {
        handle_key(
            KeyEvent::new(KeyCode::Char(key), KeyModifiers::CONTROL),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
    }
    assert_eq!(app.session.id, session_id);
    assert!(!app.terminal_panel.terminal_picker && app.terminal_panel.attached_terminal.is_none());
    assert!(!app.show_sessions && !app.model_panel.model_picker);
    let event = tokio::time::timeout(Duration::from_secs(6), rx.recv())
        .await
        .unwrap()
        .unwrap();
    handle_ui_event(event, &mut app, &store, &terminals)
        .await
        .unwrap();
    assert!(app.workflow_panel.is_open());
    app.workflow_panel.paste("workflow value");
    let (response, answer) = oneshot::channel();
    app.question = Some(QuestionDialog {
        request: QuestionRequest {
            question: crate::tools::Question {
                question: "Question?".into(),
                options: vec!["answer".into()],
            },
            response,
        },
        selected: 0,
        custom: Composer::default(),
        scroll: None,
    });
    handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(matches!(
        answer.await.unwrap(),
        crate::tools::QuestionAnswer::Selected { .. }
    ));
    let (response, answer) = oneshot::channel();
    app.approval = Some(ApprovalRequest {
        id: Uuid::new_v4(),
        action: "test".into(),
        target: "test".into(),
        reason: "test".into(),
        response,
    });
    handle_key(
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(answer.await.unwrap(), ApprovalOutcome::Approved);
    app.terminal_panel.attached_terminal = Some(terminals.id);
    handle_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(*terminals.writes.lock().unwrap(), vec![b"x".to_vec()]);
    app.terminal_panel.attached_terminal = None;
    handle_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    let backend = ratatui::backend::TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(rendered.contains("workflow value"));
    assert!(!rendered.contains("workflow valueyx"));
    handle_mouse(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 2,
            row: 2,
            modifiers: KeyModifiers::NONE,
        },
        &mut app,
    );
    assert_eq!(app.scroll, 0);
    for _ in 0..2 {
        handle_key(
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
    }
    assert!(!app.workflow_panel.is_open());
    assert_eq!(app.composer.text, "retained draft 雪");
    assert!(app.session.messages.is_empty() && app.session.workflow_runs.is_empty());
    assert!(!app.is_running());
}

#[tokio::test]
async fn workflow_start_rejects_active_run_and_full_history_before_canonical_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let invocation = crate::workflow::Invocation {
        id: "review".into(),
        version: "1".into(),
        digest: "a".repeat(64),
        scope: crate::workflow::Scope::User,
        inputs: Default::default(),
    };
    app.session.workflow_runs = vec![invocation.clone(); 128];
    let before = serde_json::to_value(&app.session).unwrap();
    assert!(
        !start_run(
            &mut app,
            &agent,
            &mut store,
            &tx,
            "data".into(),
            Some(invocation.clone())
        )
        .await
        .unwrap()
    );
    assert_eq!(before, serde_json::to_value(&app.session).unwrap());
    let (steering, _receiver) = crate::agent::steering_channel(4);
    app.running = Some(Running {
        task: tokio::spawn(std::future::pending()),
        cancel: tokio_util::sync::CancellationToken::new(),
        steering,
    });
    assert!(
        !start_run(
            &mut app,
            &agent,
            &mut store,
            &tx,
            "data".into(),
            Some(invocation)
        )
        .await
        .unwrap()
    );
    assert_eq!(before, serde_json::to_value(&app.session).unwrap());
    app.running.take().unwrap().task.abort();
}

#[tokio::test]
async fn workflow_canonical_save_failure_never_dispatches() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory); // panics if any provider request is made
    let blocked = directory.path().join("blocked");
    std::fs::write(&blocked, b"not a directory").unwrap();
    let mut store = SessionStore::new(blocked.join("sessions"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let invocation = crate::workflow::Invocation {
        id: "review".into(),
        version: "1".into(),
        digest: "a".repeat(64),
        scope: crate::workflow::Scope::User,
        inputs: Default::default(),
    };
    assert!(
        start_run(
            &mut app,
            &agent,
            &mut store,
            &tx,
            "data".into(),
            Some(invocation)
        )
        .await
        .is_err()
    );
    assert!(!app.is_running());
    assert!(app.checkpoint.is_none());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn paste_is_consumed_by_help_and_nonediting_todo_panels() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    for view in 0..3 {
        let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
        app.composer.insert_str("original draft");
        if view == 0 {
            app.shortcut_help = true;
        } else {
            app.todo_panel.todo_mode = Some(if view == 1 {
                TodoMode::List
            } else {
                TodoMode::Inspect(crate::todo::TodoId(Uuid::new_v4()))
            });
        }
        handle_input_event(
            Event::Paste("PASTED 雪\r\nλ".into()),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
        assert_eq!(app.composer.text, "original draft", "view {view}");
        assert!(app.session.messages.is_empty());
        assert!(rx.try_recv().is_err());
    }
}

#[tokio::test]
async fn paste_owner_matrix_is_exclusive_during_idle_and_active_steering() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    for active in [false, true] {
        for view in [
            "help",
            "todo-list",
            "todo-inspect",
            "todo-input",
            "policy",
            "model",
            "supervisor-tree",
            "supervisor-inspect",
            "supervisor-message",
            "terminals",
            "sessions",
            "composer",
        ] {
            let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
            app.composer.insert_str("draft");
            app.todo_panel.todo_input.insert_str("todo");
            app.supervisor_panel
                .supervisor_input
                .insert_str("supervisor");
            app.model_panel.model_filter.insert_str("model");
            let id = AgentId(Uuid::new_v4());
            match view {
                "help" => {
                    app.shortcut_help = true;
                    open_todo_input(&mut app.todo_panel, None, TodoInput::Add, "todo");
                }
                "todo-list" => app.todo_panel.todo_mode = Some(TodoMode::List),
                "todo-inspect" => {
                    app.todo_panel.todo_mode =
                        Some(TodoMode::Inspect(crate::todo::TodoId(Uuid::new_v4())))
                }
                "todo-input" => open_todo_input(&mut app.todo_panel, None, TodoInput::Add, "todo"),
                "policy" => app.policy_panel.open = true,
                "model" => app.model_panel.model_picker = true,
                "supervisor-tree" => {
                    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Tree)
                }
                "supervisor-inspect" => {
                    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Inspect(id))
                }
                "supervisor-message" => {
                    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Message {
                        target: id,
                        follow_up: false,
                    })
                }
                "terminals" => app.terminal_panel.terminal_picker = true,
                "sessions" => app.show_sessions = true,
                _ => {}
            }
            let (steering, _receiver) = crate::agent::steering_channel(1);
            if active {
                app.running = Some(Running {
                    task: tokio::spawn(std::future::pending()),
                    cancel: tokio_util::sync::CancellationToken::new(),
                    steering,
                });
            }
            handle_input_event(
                Event::Paste(" 雪\r\nλ\r🦀".into()),
                &mut app,
                &agent,
                &mut store,
                &tx,
                &terminals,
                supervisor.clone(),
                todos.clone(),
            )
            .await
            .unwrap();
            let expected = |original: &str, edits: bool| {
                if edits {
                    format!("{original} 雪\nλ\n🦀")
                } else {
                    original.to_owned()
                }
            };
            assert_eq!(
                app.composer.text,
                expected("draft", view == "composer"),
                "{view}/{active}"
            );
            assert_eq!(
                app.todo_panel.todo_input.text,
                expected("todo", view == "todo-input"),
                "{view}/{active}"
            );
            assert_eq!(
                app.supervisor_panel.supervisor_input.text,
                expected("supervisor", view == "supervisor-message"),
                "{view}/{active}"
            );
            assert_eq!(
                app.model_panel.model_filter.text,
                expected("model", view == "model"),
                "{view}/{active}"
            );
            for (width, height) in [(32, 10), (80, 24)] {
                resize_conversation(&mut app, width, height);
                let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(
                    width as u16,
                    height as u16,
                ))
                .unwrap();
                terminal.draw(|frame| draw(frame, &app)).unwrap();
            }
            assert!(app.session.messages.is_empty());
            assert!(rx.try_recv().is_err());
            assert!(terminals.writes.lock().unwrap().is_empty());
            if let Some(run) = app.running.take() {
                run.task.abort();
            }
        }
    }
}

#[tokio::test]
async fn paste_questions_approvals_and_attached_terminal_keep_priority() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("draft");
    app.show_sessions = true;
    app.shortcut_help = true;
    app.model_panel.model_picker = true;
    app.terminal_panel.attached_terminal = Some(terminals.id);
    let (response, mut approval_answer) = oneshot::channel();
    app.approval = Some(ApprovalRequest {
        id: Uuid::new_v4(),
        action: "shell".into(),
        target: "fixture".into(),
        reason: "test".into(),
        response,
    });
    let (response, mut question_answer) = oneshot::channel();
    app.question = Some(QuestionDialog {
        request: QuestionRequest {
            question: crate::tools::Question {
                question: "Question?".into(),
                options: vec!["choice".into()],
            },
            response,
        },
        selected: 1,
        custom: Composer::default(),
        scroll: None,
    });
    let paste = "雪\r\nλ\x1b";
    handle_input_event(
        Event::Paste(paste.into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(app.question.as_ref().unwrap().custom.text, "雪λ");
    assert!(question_answer.try_recv().is_err());
    assert!(approval_answer.try_recv().is_err());
    assert!(terminals.writes.lock().unwrap().is_empty());
    app.question = None;
    handle_input_event(
        Event::Paste("y\n".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(approval_answer.try_recv().is_err());
    assert!(terminals.writes.lock().unwrap().is_empty());
    app.approval = None;
    handle_input_event(
        Event::Paste(paste.into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert_eq!(*terminals.writes.lock().unwrap(), [paste.as_bytes()]);
    assert_eq!(app.composer.text, "draft");
    assert!(app.model_panel.model_filter.text.is_empty());
    assert!(app.session.messages.is_empty());
    assert!(rx.try_recv().is_err());
}

fn paste_test_screen(app: &App) -> String {
    let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

#[tokio::test]
async fn paste_workflow_and_voyage_fields_never_fall_through_or_bypass_help() {
    let directory = tempfile::tempdir().unwrap();
    let repo = directory.path().join(".helm/workflows");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("review.toml"), "schema_version=1\nid='review'\nversion='1'\ndescription='Review'\nprompt='Review {{topic}}'\n[parameters.topic]\ntype='string'\nrequired=true\n").unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("draft");
    app.workflow_panel
        .open("", directory.path().into(), &tx)
        .unwrap();
    for loaded in [false, true] {
        if loaded {
            let event = tokio::time::timeout(Duration::from_secs(6), rx.recv())
                .await
                .unwrap()
                .unwrap();
            handle_ui_event(event, &mut app, &store, &terminals)
                .await
                .unwrap();
        }
        handle_input_event(
            Event::Paste("ignored-picker-canary".into()),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
        assert_eq!(app.composer.text, "draft");
        assert!(!paste_test_screen(&app).contains("ignored-picker-canary"));
    }
    handle_input_event(
        Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    handle_input_event(
        Event::Paste("workflow-雪".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(paste_test_screen(&app).contains("workflow-雪"));
    app.shortcut_help = true;
    handle_input_event(
        Event::Paste("hidden-help-canary".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    app.shortcut_help = false;
    assert!(!paste_test_screen(&app).contains("hidden-help-canary"));
    app.workflow_panel.close();
    app.voyage_panel = voyage_setup::Hub::new(directory.path().join("voyage-drafts"));
    app.voyage_panel.open();
    handle_input_event(
        Event::Paste("ignored-voyage-canary".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(!paste_test_screen(&app).contains("ignored-voyage-canary"));
    handle_input_event(
        Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    handle_input_event(
        Event::Paste("voyage-雪".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(app.voyage_panel.form.draft().name, "voyage-雪");
    app.shortcut_help = true;
    handle_input_event(
        Event::Paste("hidden-help-canary".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert_eq!(app.voyage_panel.form.draft().name, "voyage-雪");
    assert_eq!(app.composer.text, "draft");
    assert!(app.session.messages.is_empty());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn branch_keys_preserve_latest_source_draft_and_restart_identity() {
    for slash in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let agent = navigation_agent(&directory);
        let unbound = SessionStore::new(directory.path().join("sessions"));
        let mut source = Session::new(directory.path().into(), "test".into());
        source.draft = "previously saved draft".into();
        let source_id = source.id;
        let mut store = unbound.with_execution(source_id).await.unwrap();
        store.save(&mut source).await.unwrap();
        let mut app = App::new(source, vec![]);
        app.composer = Composer::default();
        let expected = if slash { "" } else { "edited 雪\nsecond λ" };
        app.composer
            .insert_str(if slash { "/branch chosen" } else { expected });
        let (tx, mut rx) = mpsc::unbounded_channel();
        handle_input_event(
            Event::Key(if slash {
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
            } else {
                KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
            }),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &FakeTerminals::new(),
            Arc::new(FakeSupervisor::new(vec![])),
            todo_store(&directory),
        )
        .await
        .unwrap();
        assert_ne!(app.session.id, source_id);
        assert_eq!(app.session.parent_id, Some(source_id));
        assert_eq!(app.composer.text, expected);
        assert_eq!(unbound.load(source_id).await.unwrap().draft, expected);
        assert_eq!(unbound.load(app.session.id).await.unwrap().draft, expected);
        assert!(app.session.messages.is_empty());
        assert!(rx.try_recv().is_err());
        let child_id = app.session.id;
        assert_eq!(store.owned_session_id(), Some(child_id));
        unbound.with_execution(source_id).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), unbound.with_execution(child_id))
                .await
                .is_err()
        );
        drop(store);
        for id in [source_id, child_id] {
            let (owner, session) = unbound.load_owned(&id.to_string()).await.unwrap();
            assert_eq!(session.id, id);
            assert_eq!(App::new(session, vec![]).composer.text, expected);
            drop(owner);
        }
    }
}

#[tokio::test]
async fn branch_keys_keep_source_and_retry_input_on_save_failure() {
    for slash in [false, true] {
        for failure in ["malformed", "stale", "owned"] {
            let directory = tempfile::tempdir().unwrap();
            let agent = navigation_agent(&directory);
            let unbound = SessionStore::new(directory.path().join("sessions"));
            let mut source = Session::new(directory.path().into(), "test".into());
            source.draft = "persisted old draft".into();
            let id = source.id;
            let foreign = unbound.with_execution(id).await.unwrap();
            foreign.save(&mut source).await.unwrap();
            let mut store = if failure == "owned" {
                unbound.clone()
            } else {
                foreign.clone()
            };
            let path = directory.path().join("sessions").join(format!("{id}.json"));
            let original = std::fs::read(&path).unwrap();
            let mut app = App::new(source, vec![]);
            app.composer = Composer::default();
            let input = if slash {
                "/branch retry"
            } else {
                "current 雪 input"
            };
            app.composer.insert_str(input);
            if failure == "malformed" {
                std::fs::write(&path, b"{invalid").unwrap();
            }
            if failure == "stale" {
                app.session.revision -= 1;
            }
            let (tx, mut rx) = mpsc::unbounded_channel();
            handle_input_event(
                Event::Key(if slash {
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)
                } else {
                    KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL)
                }),
                &mut app,
                &agent,
                &mut store,
                &tx,
                &FakeTerminals::new(),
                Arc::new(FakeSupervisor::new(vec![])),
                todo_store(&directory),
            )
            .await
            .unwrap();
            assert_eq!(app.session.id, id);
            assert_eq!(app.session.draft, "persisted old draft");
            assert_eq!(app.composer.text, input);
            assert!(app.status.starts_with("Cannot branch:"), "{}", app.status);
            assert!(!app.quit);
            assert!(app.exit.is_none());
            assert!(rx.try_recv().is_err());
            assert_eq!(
                std::fs::read(&path).unwrap(),
                if failure == "malformed" {
                    b"{invalid".to_vec()
                } else {
                    original
                }
            );
            assert_eq!(
                std::fs::read_dir(path.parent().unwrap())
                    .unwrap()
                    .filter_map(Result::ok)
                    .filter(|entry| entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "json"))
                    .count(),
                1
            );
            if failure != "owned" {
                assert_eq!(store.owned_session_id(), Some(id));
            }
        }
    }
}

#[tokio::test]
async fn policy_panel_owns_paste_and_preserves_composer_without_provider_dispatch() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, _) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("preserved draft 工作");
    let config = crate::Config::default();
    let policy = crate::policy::Policy::new(&config, directory.path().into()).unwrap();
    app.policy_panel
        .configure(Some(crate::policy_profile::switching::SwitchContext::new(
            config,
            policy.effective().clone(),
        )));
    let (steering, _) = crate::agent::steering_channel(1);
    app.running = Some(Running {
        task: tokio::spawn(std::future::pending()),
        cancel: tokio_util::sync::CancellationToken::new(),
        steering,
    });
    handle_input_event(
        Event::Key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert!(!app.policy_panel.open);
    assert!(app.status.contains("Finish or cancel active work"));
    app.running.take().unwrap().task.abort();
    app.policy_panel
        .open(Some(directory.path().join("profiles")), &tx)
        .unwrap();
    assert!(input_owner(&app) == InputOwner::Policy);
    handle_input_event(
        Event::Paste("y\n/shell touch injected".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(app.composer.text, "preserved draft 工作");
    assert!(app.exit.is_none() && !app.is_running());
    handle_input_event(
        Event::Key(KeyEvent::from(KeyCode::Esc)),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert!(input_owner(&app) == InputOwner::Composer);
    assert_eq!(app.composer.text, "preserved draft 工作");
}

#[tokio::test]
async fn policy_confirmation_save_failure_retains_review_draft_and_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let blocked = directory.path().join("blocked");
    std::fs::write(&blocked, "occupied").unwrap();
    let mut store = SessionStore::new(blocked.join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.insert_str("retained draft 工作");
    let config = crate::Config::default();
    let policy = crate::policy::Policy::new(&config, directory.path().into()).unwrap();
    app.policy_panel
        .configure(Some(crate::policy_profile::switching::SwitchContext::new(
            config,
            policy.effective().clone(),
        )));
    app.policy_panel
        .open(Some(directory.path().join("profiles")), &tx)
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    handle_ui_event(event, &mut app, &store, &terminals)
        .await
        .unwrap();
    for code in [KeyCode::Down, KeyCode::Enter] {
        handle_input_event(
            Event::Key(KeyEvent::from(code)),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
    }
    let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    handle_ui_event(event, &mut app, &store, &terminals)
        .await
        .unwrap();
    // Defaults may already grant the chosen profile; either confirmation key must retain state.
    for code in [KeyCode::Char('y'), KeyCode::Enter] {
        handle_input_event(
            Event::Key(KeyEvent::from(code)),
            &mut app,
            &agent,
            &mut store,
            &tx,
            &terminals,
            supervisor.clone(),
            todos.clone(),
        )
        .await
        .unwrap();
    }
    assert!(app.policy_panel.open && app.exit.is_none() && !app.quit && !app.is_running());
    assert_eq!(app.composer.text, "retained draft 工作");
    assert_eq!(app.session.draft, "retained draft 工作");
    assert!(app.session.messages.is_empty());
    assert!(paste_test_screen(&app).contains("Cannot save voyage"));
}

#[tokio::test]
async fn inference_inspection_keeps_input_resize_and_cancel_responsive_and_fences_stale_results() {
    let directory = tempfile::tempdir().unwrap();
    let mut ledger = crate::inference::Store::open(directory.path().join("inference")).unwrap();
    let project = ledger.project(directory.path()).unwrap();
    let accounting = crate::inference::runtime::Accounting::fixture(ledger, project);
    let held_store = accounting.test_store();
    let agent = Arc::new(
        Arc::try_unwrap(navigation_agent(&directory))
            .ok()
            .unwrap()
            .with_inference_accounting(accounting),
    );
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (held_tx, held_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = held_store.lock().unwrap();
        let _ = held_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });
    held_rx.await.unwrap();
    tokio::time::timeout(
        Duration::from_millis(200),
        handle_command("/inference", &mut app, &mut store, Some(&agent), Some(&tx)),
    )
    .await
    .expect("inspection must not block the event loop")
    .unwrap();
    tokio::task::yield_now().await;
    assert!(rx.try_recv().is_err(), "the actual store is held");
    handle_input_event(
        Event::Paste("draft λ".into()),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor.clone(),
        todos.clone(),
    )
    .await
    .unwrap();
    assert_eq!(app.composer.text, "draft λ");
    for (width, height) in [(32, 10), (100, 30)] {
        resize_conversation(&mut app, width, height);
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(
            width as u16,
            height as u16,
        ))
        .unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    let (steering, _receiver) = crate::agent::steering_channel(4);
    app.running = Some(Running {
        task: tokio::spawn(std::future::pending()),
        cancel: cancel.clone(),
        steering,
    });
    handle_key(
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        supervisor,
        todos,
    )
    .await
    .unwrap();
    assert!(cancel.is_cancelled());
    app.running.take().unwrap().task.abort();
    let old_session = app.session.id;
    app.session = Session::new(directory.path().into(), "test".into());
    app.status = "new session remains active".into();
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&event,UiEvent::InferenceStatus{session,..} if *session==old_session));
    handle_ui_event(event, &mut app, &store, &terminals)
        .await
        .unwrap();
    assert_eq!(app.status, "new session remains active");
    assert_eq!(app.composer.text, "draft λ");
}
