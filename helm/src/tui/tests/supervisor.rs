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

#[tokio::test]
async fn history_warnings_survive_refresh_ring_eviction_and_narrow_resized_render() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().join("sessions"));
    let terminals = FakeTerminals::new();
    let mut app = App::new(Session::new(dir.path().into(), "test".into()), vec![]);
    let id = AgentId(Uuid::new_v4());
    let mut view = agent(id, None, "安全 🦀 é \u{202e}\u{1b}[31m task");
    view.status = AgentStatus::Cancelled;
    view.error = Some("cancelled".into());
    app.supervisor_panel.supervisor_mode = Some(SupervisorMode::Inspect(id));
    let events = (1..=600)
        .map(|sequence| SupervisionEvent {
            sequence,
            timestamp: Utc::now(),
            agent_id: id,
            kind: SupervisionEventKind::Progress {
                text: "進捗 🦀 \u{1b}]52;synthetic\u{7}".into(),
            },
        })
        .collect::<Vec<_>>();
    for event in &events {
        app.supervisor_panel.append_event(event.clone());
    }
    assert_eq!(app.supervisor_panel.inspected_events.len(), 500);
    app.supervisor_panel.record_lag(11);
    let inspection = crate::supervision::AgentInspection {
        agent: view.clone(),
        events,
        history: Some(crate::subagent::HistoryStatus {
            cursor: crate::subagent::HistoryCursor {
                epoch: Uuid::new_v4(),
                sequence: 600,
            },
            first_sequence: Some(1),
            evicted_through: 12,
            durable: false,
            cursor_gap: true,
            notices: [
                crate::subagent::HistoryNotice::Corrupt,
                crate::subagent::HistoryNotice::Restarted,
                crate::subagent::HistoryNotice::WriteFailed,
            ]
            .into_iter()
            .collect(),
        }),
        transport_skipped: 20,
    };
    for _ in 0..3 {
        handle_ui_event(
            UiEvent::SupervisorInspect(id, Ok(inspection.clone())),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
        handle_ui_event(
            UiEvent::SupervisorTree(Ok(vec![])),
            &mut app,
            &store,
            &terminals,
        )
        .await
        .unwrap();
    }
    assert_eq!(app.supervisor_panel.adapter_skipped, 20);
    assert_eq!(app.supervisor_panel.live_skipped, 11);
    assert_eq!(app.supervisor_panel.inspected_events.len(), 500);
    assert_eq!(
        app.supervisor_panel
            .inspected_agent
            .as_ref()
            .unwrap()
            .status,
        AgentStatus::Cancelled
    );
    for (width, height) in [(32, 10), (50, 14), (100, 30), (32, 10)] {
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            rendered.contains("History warning"),
            "{width}x{height}: {rendered}"
        );
        assert!(!rendered.contains('\u{1b}') && !rendered.contains('\u{202e}'));
    }
    app.supervisor_panel.supervisor_scroll = usize::MAX;
    let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    for expected in [
        "安",
        "全",
        "cancelled",
        "corrupt",
        "Restart boundary",
        "write failed",
        "skipped 11",
        "skipped 20",
    ] {
        assert!(
            rendered.contains(expected),
            "missing {expected}: {rendered}"
        );
    }
}

#[test]
fn stale_history_replies_do_not_change_selected_agent_or_rewind_cursor() {
    let first = AgentId(Uuid::new_v4());
    let second = AgentId(Uuid::new_v4());
    let mut panel = SupervisorPanel {
        supervisor_mode: Some(SupervisorMode::Inspect(second)),
        ..Default::default()
    };
    panel.apply_inspection(crate::supervision::AgentInspection {
        agent: agent(first, None, "wrong"),
        events: vec![],
        history: None,
        transport_skipped: 0,
    });
    assert!(panel.inspected_agent.is_none());
}

#[test]
fn history_replay_cannot_replace_a_newer_live_event_with_an_older_snapshot() {
    let id = AgentId(Uuid::new_v4());
    let epoch = Uuid::new_v4();
    let mut panel = SupervisorPanel {
        supervisor_mode: Some(SupervisorMode::Inspect(id)),
        ..Default::default()
    };
    let mut inspection = crate::supervision::AgentInspection {
        agent: agent(id, None, "task"),
        events: vec![],
        transport_skipped: 0,
        history: Some(crate::subagent::HistoryStatus {
            cursor: crate::subagent::HistoryCursor {
                epoch,
                sequence: 10,
            },
            first_sequence: None,
            evicted_through: 0,
            notices: Default::default(),
            durable: true,
            cursor_gap: false,
        }),
    };
    panel.apply_inspection(inspection.clone());
    panel.append_event(SupervisionEvent {
        sequence: 12,
        timestamp: Utc::now(),
        agent_id: id,
        kind: SupervisionEventKind::Cancelled,
    });
    inspection.history.as_mut().unwrap().cursor.sequence = 11;
    panel.apply_inspection(inspection);
    assert_eq!(panel.inspected_events.last().unwrap().sequence, 12);
}

#[test]
fn initial_history_replay_preserves_newer_live_events() {
    let id = AgentId(Uuid::new_v4());
    let epoch = Uuid::new_v4();
    let mut panel = SupervisorPanel {
        supervisor_mode: Some(SupervisorMode::Inspect(id)),
        ..Default::default()
    };
    let mut inspection = crate::supervision::AgentInspection {
        agent: agent(id, None, "task"),
        events: vec![],
        transport_skipped: 0,
        history: Some(crate::subagent::HistoryStatus {
            cursor: crate::subagent::HistoryCursor {
                epoch,
                sequence: 10,
            },
            first_sequence: None,
            evicted_through: 0,
            notices: Default::default(),
            durable: true,
            cursor_gap: false,
        }),
    };
    panel.append_event(SupervisionEvent {
        sequence: 12,
        timestamp: Utc::now(),
        agent_id: id,
        kind: SupervisionEventKind::Cancelled,
    });
    inspection.history.as_mut().unwrap().cursor.sequence = 11;
    panel.apply_inspection(inspection);
    assert_eq!(panel.inspected_events.last().unwrap().sequence, 12);
}
