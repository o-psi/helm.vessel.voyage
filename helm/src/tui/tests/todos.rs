use super::*;

#[tokio::test]
async fn todo_composer_actions_are_async_and_do_not_enter_chat() {
    let directory = tempfile::tempdir().unwrap();
    let store = todo_store(&directory);
    let mut app = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    app.todo_panel.todo_mode = Some(TodoMode::List);
    let (tx, mut rx) = mpsc::unbounded_channel();
    handle_todo_key(
        KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
        &mut app.todo_panel,
        &tx,
        store.clone(),
    )
    .await;
    app.todo_panel
        .todo_input
        .insert_str("Ship it\nRelease evidence");
    handle_todo_key(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        &mut app.todo_panel,
        &tx,
        store.clone(),
    )
    .await;
    assert!(matches!(rx.recv().await, Some(UiEvent::TodoAction(Ok(_)))));
    let Some(UiEvent::TodoSnapshot(Ok(snapshot))) = rx.recv().await else {
        panic!("todo action must trigger a live snapshot refresh")
    };
    let item = snapshot.ordered()[0];
    assert_eq!(item.title, "Ship it");
    assert_eq!(item.description, "Release evidence");
    assert!(app.session.messages.is_empty());
}

#[tokio::test]
async fn todo_view_renders_selected_item_and_complete_inspection() {
    let directory = tempfile::tempdir().unwrap();
    let store = todo_store(&directory);
    for index in 0..20 {
        store
            .create(NewTodo {
                title: format!("todo {index}"),
                description: format!("description {index}"),
                priority: Priority::Normal,
                order: None,
                assignees: BTreeSet::new(),
            })
            .await
            .unwrap();
    }
    let snapshot = store.snapshot().await.unwrap();
    let mut app = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    app.todo_panel.todo_mode = Some(TodoMode::List);
    app.todo_panel.todos = snapshot.ordered().into_iter().cloned().collect();
    app.todo_panel.selected_todo = 19;
    let backend = ratatui::backend::TestBackend::new(40, 10);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| draw(frame, &app)).unwrap();
    let rendered: String = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(rendered.contains("todo 19"));
    assert!(rendered.contains("Esc return"));

    app.todo_panel.todo_mode = Some(TodoMode::Inspect(app.todo_panel.todos[19].id));
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
    assert!(rendered.contains("description 19"));
    assert!(rendered.contains("Assignees"));
}

#[tokio::test]
async fn todo_ui_maps_edit_block_assign_evidence_reorder_transition_and_archive() {
    let directory = tempfile::tempdir().unwrap();
    let store = todo_store(&directory);
    let first = store
        .create(NewTodo {
            title: "first".into(),
            description: String::new(),
            priority: Priority::Normal,
            order: None,
            assignees: BTreeSet::new(),
        })
        .await
        .unwrap();
    let second = store
        .create(NewTodo {
            title: "second".into(),
            description: String::new(),
            priority: Priority::Normal,
            order: None,
            assignees: BTreeSet::new(),
        })
        .await
        .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();

    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Edit,
        "renamed\ndetails".into(),
    );
    receive_todo_action(&mut rx).await;
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Block,
        "waiting on access".into(),
    );
    receive_todo_action(&mut rx).await;
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Assign,
        "alice, bob".into(),
    );
    receive_todo_action(&mut rx).await;
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Evidence,
        "report.txt".into(),
    );
    receive_todo_action(&mut rx).await;
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Progress,
        "halfway".into(),
    );
    receive_todo_action(&mut rx).await;
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Note,
        "remember rollback".into(),
    );
    receive_todo_action(&mut rx).await;
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Dependencies,
        second.id.0.to_string(),
    );
    receive_todo_action(&mut rx).await;

    let snapshot = store.snapshot().await.unwrap();
    let item = snapshot.items.get(&first.id).unwrap();
    assert_eq!(item.title, "renamed");
    assert_eq!(item.description, "details");
    assert_eq!(item.status, TodoStatus::Blocked);
    assert_eq!(item.assignees.len(), 2);
    assert_eq!(item.evidence[0].text, "report.txt");
    assert_eq!(item.progress[0].text, "halfway");
    assert_eq!(item.notes[0].text, "remember rollback");
    assert!(item.dependencies.contains(&second.id));

    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Dependencies,
        "not-a-uuid".into(),
    );
    assert!(matches!(rx.recv().await, Some(UiEvent::TodoAction(Err(_)))));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::TodoSnapshot(Ok(_)))
    ));
    assert!(
        store.snapshot().await.unwrap().items[&first.id]
            .dependencies
            .contains(&second.id)
    );
    request_todo_input(
        &tx,
        store.clone(),
        Some(first.id),
        TodoInput::Dependencies,
        String::new(),
    );
    receive_todo_action(&mut rx).await;
    assert!(
        store.snapshot().await.unwrap().items[&first.id]
            .dependencies
            .is_empty()
    );

    let mut app = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    app.todo_panel.todos = snapshot.ordered().into_iter().cloned().collect();
    reorder_todo(&app.todo_panel, &tx, store.clone(), first.id, 1);
    receive_todo_action(&mut rx).await;
    let snapshot = store.snapshot().await.unwrap();
    assert!(snapshot.items[&first.id].order >= snapshot.items[&second.id].order);

    store
        .set_status(second.id, TodoStatus::Completed)
        .await
        .unwrap();
    store.set_blockers(first.id, Vec::new()).await.unwrap();
    let mut app = App::new(
        Session::new(directory.path().into(), "test-model".into()),
        Vec::new(),
    );
    app.todo_panel.todo_mode = Some(TodoMode::Inspect(first.id));
    app.todo_panel.todos = store
        .snapshot()
        .await
        .unwrap()
        .ordered()
        .into_iter()
        .cloned()
        .collect();
    handle_todo_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        &mut app.todo_panel,
        &tx,
        store.clone(),
    )
    .await;
    receive_todo_action(&mut rx).await;
    assert_eq!(
        store.snapshot().await.unwrap().items[&first.id].status,
        TodoStatus::InProgress
    );
    app.todo_panel.todos = store
        .snapshot()
        .await
        .unwrap()
        .ordered()
        .into_iter()
        .cloned()
        .collect();
    handle_todo_key(
        KeyEvent::new(KeyCode::Char('t'), KeyModifiers::NONE),
        &mut app.todo_panel,
        &tx,
        store.clone(),
    )
    .await;
    receive_todo_action(&mut rx).await;
    assert_eq!(
        store.snapshot().await.unwrap().items[&first.id].status,
        TodoStatus::Completed
    );
    app.todo_panel.todos = store
        .snapshot()
        .await
        .unwrap()
        .ordered()
        .into_iter()
        .cloned()
        .collect();
    handle_todo_key(
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        &mut app.todo_panel,
        &tx,
        store.clone(),
    )
    .await;
    receive_todo_action(&mut rx).await;
    assert!(store.snapshot().await.unwrap().items[&first.id].archived());
}
