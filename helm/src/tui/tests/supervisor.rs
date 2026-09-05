use super::*;

#[test]
fn agent_tree_is_parent_first_and_cycle_safe() {
    let root = AgentId(Uuid::new_v4());
    let child = AgentId(Uuid::new_v4());
    let orphan = AgentId(Uuid::new_v4());
    let cycle_a = AgentId(Uuid::new_v4());
    let cycle_b = AgentId(Uuid::new_v4());
    let flattened = flatten_agent_tree(vec![
        agent(child, Some(root), "child"),
        agent(cycle_a, Some(cycle_b), "cycle a"),
        agent(root, None, "root"),
        agent(orphan, Some(AgentId(Uuid::new_v4())), "orphan"),
        agent(cycle_b, Some(cycle_a), "cycle b"),
    ]);
    assert_eq!(flattened.len(), 5);
    assert!(
        flattened.iter().position(|item| item.id == root)
            < flattened.iter().position(|item| item.id == child)
    );
    assert_eq!(
        flattened.iter().filter(|item| item.id == cycle_a).count(),
        1
    );
    assert_eq!(
        flattened.iter().filter(|item| item.id == cycle_b).count(),
        1
    );
}

#[test]
fn interrupted_agents_are_terminal_and_render_distinctly() {
    assert!(AgentStatus::Interrupted.is_terminal());
    assert_eq!(status_label(&AgentStatus::Interrupted), "interrupted");
    assert_eq!(
        supervision_event_label(&SupervisionEventKind::Interrupted {
            reason: "operator stopped work".into(),
        }),
        "interrupted: operator stopped work"
    );
}

#[test]
fn supervisor_summary_separates_active_agents_from_retained_history() {
    let running = agent(AgentId(Uuid::new_v4()), None, "running");
    let mut completed = agent(AgentId(Uuid::new_v4()), None, "completed");
    completed.status = AgentStatus::Completed;
    let mut timed_out = agent(AgentId(Uuid::new_v4()), None, "timed out");
    timed_out.status = AgentStatus::TimedOut;
    let agents = vec![running, completed, timed_out];

    assert_eq!(supervisor_counts(&agents), (1, 2));
    assert_eq!(
        supervisor_summary(&agents),
        "Supervising 1 active agent · 2 retained"
    );
}

#[tokio::test]
async fn supervisor_actions_are_dispatched_without_entering_chat_transcript() {
    let directory = tempfile::tempdir().unwrap();
    let id = AgentId(Uuid::new_v4());
    let supervisor = Arc::new(FakeSupervisor::new(vec![agent(id, None, "research")]));
    let mut app = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    app.supervisor_panel.agents = supervisor.agents.clone();
    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Tree);
    let (tx, mut rx) = mpsc::unbounded_channel();

    handle_supervisor_key(
        KeyEvent::new(KeyCode::Char('m'), KeyModifiers::NONE),
        &mut app.supervisor_panel,
        &mut app.status,
        &tx,
        supervisor.clone(),
    )
    .await;
    app.supervisor_panel
        .supervisor_input
        .insert_str("status please");
    handle_supervisor_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app.supervisor_panel,
        &mut app.status,
        &tx,
        supervisor.clone(),
    )
    .await;
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::SupervisorAction(Ok(_)))
    ));
    assert!(app.session.messages.is_empty());
    assert_eq!(
        supervisor.actions.lock().unwrap().as_slice(),
        &[format!("message:{id}:status please")]
    );

    handle_supervisor_key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        &mut app.supervisor_panel,
        &mut app.status,
        &tx,
        supervisor.clone(),
    )
    .await;
    assert!(supervisor.actions.lock().unwrap().len() == 1);
    handle_supervisor_key(
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        &mut app.supervisor_panel,
        &mut app.status,
        &tx,
        supervisor.clone(),
    )
    .await;
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::SupervisorAction(Ok(_)))
    ));
    assert_eq!(supervisor.actions.lock().unwrap().len(), 2);
}

#[test]
fn agent_view_renders_at_minimum_supported_size_and_keeps_selection_visible() {
    let mut app = App::new(
        Session::new(PathBuf::from("/tmp"), "test-model".into()),
        Vec::new(),
    );
    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Tree);
    app.supervisor_panel.agents = (0..20)
        .map(|index| agent(AgentId(Uuid::new_v4()), None, &format!("task {index}")))
        .collect();
    app.supervisor_panel.selected_agent = 19;
    let selected_id = short_id(app.supervisor_panel.agents[19].id);
    let backend = ratatui::backend::TestBackend::new(32, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains(&selected_id));
    assert!(rendered.contains("Esc return"));
}
