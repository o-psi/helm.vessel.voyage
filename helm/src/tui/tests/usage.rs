use super::routing::navigation_agent;
use super::*;

fn screen(app: &App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| draw(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
async fn receive(
    rx: &mut mpsc::UnboundedReceiver<UiEvent>,
    app: &mut App,
    store: &SessionStore,
    terminals: &FakeTerminals,
) {
    let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    handle_ui_event(event, app, store, terminals).await.unwrap();
}
#[tokio::test]
async fn history_real_ledger_group_drilldown_scope_and_paste_ownership() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    app.composer.text = "keep this draft λ".into();
    let mut ledger = crate::inference::Store::open(directory.path().join("inference")).unwrap();
    let project = ledger.project(directory.path()).unwrap();
    ledger.bind_session(project, app.session.id).unwrap();
    let run = Uuid::new_v4();
    let permit = ledger
        .admit(&crate::inference::Attribution {
            session: app.session.id,
            run,
            agent: None,
            provider: "fixture\u{061c}".into(),
            model: "model λ\u{202e}\u{200b}".into(),
            purpose: crate::inference::Purpose::Conversation,
        })
        .unwrap();
    ledger.report(permit.id, Some(0), None).unwrap();
    let accounting = crate::inference::runtime::Accounting::fixture(ledger, project);
    let agent = Arc::new(
        Arc::try_unwrap(navigation_agent(&directory))
            .ok()
            .unwrap()
            .with_inference_accounting(accounting),
    );
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let terminals = FakeTerminals::new();
    let supervisor = Arc::new(FakeSupervisor::new(vec![]));
    let todos = todo_store(&directory);
    handle_command(
        "/inference history --from 2000-01-01T00:00:00Z --until 2100-01-01T00:00:00Z",
        &mut app,
        &mut store,
        Some(&agent),
        Some(&tx),
    )
    .await
    .unwrap();
    receive(&mut rx, &mut app, &store, &terminals).await;
    let display = screen(&app, 120, 30);
    assert!(display.contains("model λ"));
    for ch in ['\u{061c}', '\u{202e}', '\u{200b}'] {
        assert!(!display.contains(ch));
        assert!(display.contains(&format!("\\u{:04x}", ch as u32)));
    }
    assert!(display.contains("0 reported (1 known / 0 missing)"));
    assert!(display.contains("unavailable reported (0 known / 1 missing)"));
    handle_input_event(
        Event::Paste("/run do not execute\ny".into()),
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
    assert_eq!(app.composer.text, "keep this draft λ");
    assert!(app.session.messages.is_empty());
    assert!(!app.is_running());
    handle_key(
        KeyEvent::from(KeyCode::Enter),
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
    receive(&mut rx, &mut app, &store, &terminals).await;
    assert!(screen(&app, 120, 60).contains(&run.to_string()));
    for (width, height) in [(8, 3), (24, 8), (80, 24)] {
        resize_conversation(&mut app, width, height);
        let _ = screen(&app, width as u16, height as u16);
    }
    handle_key(
        KeyEvent::from(KeyCode::Backspace),
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
    receive(&mut rx, &mut app, &store, &terminals).await;
    handle_key(
        KeyEvent::from(KeyCode::Char('s')),
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
    receive(&mut rx, &mut app, &store, &terminals).await;
    assert!(screen(&app, 100, 30).contains("Voyage history"));
    handle_key(
        KeyEvent::from(KeyCode::Esc),
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
    assert!(!app.usage_panel.open);
    assert_eq!(app.composer.text, "keep this draft λ");
}
#[tokio::test]
async fn history_wait_cancel_and_stale_results_do_not_mutate_new_voyage() {
    let directory = tempfile::tempdir().unwrap();
    let mut ledger = crate::inference::Store::open(directory.path().join("inference")).unwrap();
    let project = ledger.project(directory.path()).unwrap();
    let accounting = crate::inference::runtime::Accounting::fixture(ledger, project);
    let held = accounting.test_store();
    let agent = Arc::new(
        Arc::try_unwrap(navigation_agent(&directory))
            .ok()
            .unwrap()
            .with_inference_accounting(accounting),
    );
    let mut app = App::new(Session::new(directory.path().into(), "test".into()), vec![]);
    let mut store = SessionStore::new(directory.path().join("sessions"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let terminals = FakeTerminals::new();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = held.lock().unwrap();
        let _ = ready_tx.send(());
        let _ = release_rx.recv_timeout(Duration::from_secs(5));
    });
    ready_rx.await.unwrap();
    tokio::time::timeout(
        Duration::from_millis(200),
        handle_command(
            "/inference history",
            &mut app,
            &mut store,
            Some(&agent),
            Some(&tx),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    let _ = screen(&app, 24, 6);
    handle_key(
        KeyEvent::from(KeyCode::Esc),
        &mut app,
        &agent,
        &mut store,
        &tx,
        &terminals,
        Arc::new(FakeSupervisor::new(vec![])),
        todo_store(&directory),
    )
    .await
    .unwrap();
    assert!(!app.usage_panel.open);
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    handle_command(
        "/inference history",
        &mut app,
        &mut store,
        Some(&agent),
        Some(&tx),
    )
    .await
    .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    app.session = Session::new(directory.path().into(), "new".into());
    app.status = "new voyage".into();
    handle_ui_event(event, &mut app, &store, &terminals)
        .await
        .unwrap();
    assert_eq!(app.status, "new voyage");
    assert!(screen(&app, 100, 20).contains("Loading historical accounting"));
    app.usage_panel.close();
}

struct NoHistoryRequests;
#[async_trait]
impl crate::provider::Provider for NoHistoryRequests {
    async fn complete(
        &self,
        _: crate::model::ModelRequest,
    ) -> Result<crate::model::ModelResponse, crate::provider::ProviderError> {
        panic!("historical inspection must not dispatch inference")
    }
}
#[tokio::test]
async fn history_profile_revocation_after_worker_return_refuses_ui_disclosure() {
    use crate::policy_profile::{
        Builtin, Overrides,
        selection::{Selection, SelectionRequest},
        store::{Action, ProfileChange, ProfileStore},
    };
    let root = tempfile::tempdir().unwrap();
    let mut app = App::new(Session::new(root.path().into(), "test".into()), vec![]);
    app.composer.text = "preserve draft".into();
    let directory = root.path().join("profiles");
    let profiles = ProfileStore::open(&directory).unwrap();
    let snapshot = profiles
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "history-profile".into(),
            expected_revision: 0,
            action: Action::Create {
                rules: Builtin::Balanced.document().rules,
            },
        })
        .unwrap()
        .snapshot;
    let mut config = crate::Config::default();
    let request = SelectionRequest {
        directory,
        name: snapshot.name.clone(),
        revision: snapshot.revision,
        digest: snapshot.digest().unwrap(),
        explicit: Overrides::default(),
    };
    let preview = Selection::preview(&config, root.path(), &request).unwrap();
    config.policy_profile = Some(
        Selection::bind(
            &config,
            root.path(),
            request,
            Some(&preview.transition_digest),
        )
        .unwrap(),
    );
    let policy = Arc::new(crate::policy::Policy::new(&config, root.path().into()).unwrap());
    let mut ledger = crate::inference::Store::open(root.path().join("inference")).unwrap();
    let project = ledger.project(root.path()).unwrap();
    ledger.bind_session(project, app.session.id).unwrap();
    ledger
        .admit(&crate::inference::Attribution {
            session: app.session.id,
            run: Uuid::new_v4(),
            agent: None,
            provider: "fixture".into(),
            model: "private-model-observation".into(),
            purpose: crate::inference::Purpose::Conversation,
        })
        .unwrap();
    let agent = Arc::new(
        Agent::new(
            Box::new(NoHistoryRequests),
            crate::tools::ToolRegistry::default(),
            crate::tools::ToolContext {
                github: None,
                completion: None,
                policy,
                approver: Arc::new(crate::tools::UnattendedApprover { allow: false }),
                timeout: Duration::from_secs(2),
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
        .with_inference_accounting(crate::inference::runtime::Accounting::fixture(
            ledger, project,
        )),
    );
    let mut store = SessionStore::new(root.path().join("sessions"));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let terminals = FakeTerminals::new();
    handle_command(
        "/inference history",
        &mut app,
        &mut store,
        Some(&agent),
        Some(&tx),
    )
    .await
    .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let UiEvent::InferenceHistory { result, .. } = &event else {
        panic!("unexpected history event")
    };
    let data = result.as_ref().unwrap();
    assert_eq!(data.totals.retained_attempts, 1);
    assert!(
        serde_json::to_string(data)
            .unwrap()
            .contains("private-model-observation")
    );
    profiles
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: snapshot.name,
            expected_revision: 1,
            action: Action::Delete {},
        })
        .unwrap();
    assert!(agent.check_current_policy().is_err());
    handle_ui_event(event, &mut app, &store, &terminals)
        .await
        .unwrap();
    let visible = screen(&app, 120, 30);
    assert!(visible.contains("Current local authority changed"));
    assert!(!visible.contains("private-model-observation"));
    assert_eq!(app.composer.text, "preserve draft");
    assert!(app.session.messages.is_empty());
    app.usage_panel.close();
}
