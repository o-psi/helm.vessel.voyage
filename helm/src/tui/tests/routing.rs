//! Cross-module routing regressions: feature extraction must not change precedence.
use super::*;

struct NoRequests;

#[tokio::test]
async fn tool_details_shortcut_expands_collapses_and_respects_approval_input() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let store = SessionStore::new(directory.path().join("sessions"));
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
        &store,
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
        &store,
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
        key, &mut app, &agent, &store, &tx, &terminals, supervisor, todos,
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

fn navigation_agent(directory: &tempfile::TempDir) -> Arc<Agent> {
    Arc::new(Agent::new(
        Box::new(NoRequests),
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
    ))
}

#[tokio::test]
async fn central_router_preserves_modal_and_panel_precedence() {
    let directory = tempfile::tempdir().unwrap();
    let agent = navigation_agent(&directory);
    let store = SessionStore::new(directory.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
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
        &store,
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
        &store,
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
        &store,
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
        &store,
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
        &store,
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
        &store,
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
        &store,
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
        &store,
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
