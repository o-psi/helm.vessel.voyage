//! Focused App journeys. Synthetic watch/one-shot inputs exercise the real picker,
//! persistence and input paths; no provider, socket server or browser is contacted.
use super::super::{
    account_test_support as support,
    state::{Target, View},
};
use super::*;
use crate::process_client::{duplex::ConnectionState, transport::Client};
use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use std::collections::BTreeMap;

fn app(dir: &std::path::Path) -> App {
    let clients = super::super::routes::Routes::new(vec![Client::local(dir.join("no-vessel"))]);
    let (sender, _receiver) = tokio::sync::mpsc::channel(32);
    let app = App {
        browsers: BTreeMap::new(),
        browser_opened: Default::default(),
        browser_retired: Vec::new(),
        working: super::super::effects::Working::new(false).unwrap(),
        settle_after_secs: 0,
        presentation_now: chrono::Utc::now(),
        previews: super::super::previews::State::new(false, false).unwrap(),
        vessels: None,
        vessel_button: Default::default(),
        vessel_sidebar_button: Default::default(),
        vessel_filter: None,
        clipboard_pending: None,
        clipboard_blocked: false,
        clients,
        observers: BTreeMap::new(),
        retired_observers: Vec::new(),
        coordination_request: None,
        route_tasks: BTreeMap::new(),
        pending_activations: BTreeMap::new(),
        pending_disconnects: Default::default(),
        new_chat_config: None,
        views: BTreeMap::new(),
        selected: None,
        new_drafts: BTreeMap::new(),
        active_draft: None,
        workspace_picker: None,
        draft_hits: Default::default(),
        sender,
        command_checks: BTreeMap::new(),
        first_send_checks: BTreeMap::new(),
        status: String::new(),
        quit: false,
        help: false,
        archives: false,
        help_scroll: 0,
        explore: None,
        workflows: Default::default(),
        operator: None,
        operator_loading: None,
        voyage_picker: None,
        interactions: Default::default(),
        terminal_request: None,
        completion: Default::default(),
        inference: Default::default(),
        accounts: Default::default(),
        sidebar: Default::default(),
    };
    app
}
fn settings() -> Settings {
    Settings {
        account: Some(super::tests::binding()),
        model: "synthetic-model".into(),
        provider: "openai-responses".into(),
        reasoning_effort: Some("high".into()),
        service_tier: Some("default".into()),
        ..Default::default()
    }
}
fn live(app: &mut App) -> Target {
    let t = Target {
        route: app.clients.first_route().unwrap(),
        session: Uuid::new_v4(),
    };
    let mut v = View::new(serde_json::from_value(serde_json::json!({
        "session_id": t.session, "incarnation": Uuid::new_v4(), "workspace": "/synthetic-workspace", "state": "live", "name": "fixture"
    })).unwrap());
    v.snapshot = Some(serde_json::from_value(serde_json::json!({
        "session_id": t.session, "revision": 17, "model": "synthetic-model", "messages": [], "inference": settings(), "inference_current": settings(),
        "run": {"run_id": Uuid::new_v4(), "state": "running"}
    })).unwrap());
    v.draft.text = "preserve my composer".into();
    app.views.insert(t, v);
    app.selected = Some(t);
    t
}
fn key(app: &mut App, code: KeyCode) {
    app.input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}
fn draw(app: &App, width: u16, height: u16) -> String {
    let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
    term.draw(|f| app.draw_accounts(f)).unwrap();
    term.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
fn picker(app: &mut App, t: Target) -> tokio::sync::watch::Sender<ConnectionState> {
    let socket = ConnectionState {
        socket_id: Some(Uuid::new_v4()),
        loss_generation: 4,
    };
    let (tx, rx) = tokio::sync::watch::channel(socket);
    let original = app.inference_settings(Destination::Live(t)).unwrap();
    let b = super::tests::binding();
    app.accounts.picker = Some(Picker {
        id: Uuid::new_v4(),
        destination: Destination::Live(t),
        route: t.route,
        workspace: "/synthetic-workspace".into(),
        host: Some(Uuid::new_v4()),
        _lock: None,
        catalogue: Catalogue {
            accounts: vec![AccountDescriptor {
                id: b.account_id,
                connection_id: b.connection_id,
                alias: "work".into(),
                label: "Work".into(),
                metadata_revision: 1,
                identity_generation: b.identity_generation,
                credential_revision: 1,
                capability_revision: 1,
                availability: CredentialAvailability::Available,
                state: AccountState::Ready,
            }],
            connections: vec![ConnectionDescriptor {
                id: b.connection_id,
                revision: b.connection_revision,
                label: "API".into(),
                endpoint: "https://api.openai.com/v1".into(),
                transports: vec![b.transport],
            }],
        },
        original,
        mode: Mode::List,
        query: String::new(),
        selected: 0,
        edit: 0,
        remember: false,
        enroll: true,
        notice: String::new(),
        busy: false,
        intent: None,
        private: None,
        poll: Instant::now(),
        models: vec![],
        connection: rx,
        loss_generation: socket.loss_generation,
        disconnected: false,
    });
    draw(app, 110, 32);
    tx
}
fn material(app: &mut App) -> PrivateEnrollmentStatus {
    let p = app.accounts.picker.as_mut().unwrap();
    let enrollment_id = Uuid::new_v4();
    p.intent = Some(storage::Intent {
        host: p.host.unwrap(),
        workspace: p.workspace.clone(),
        cancel: None,
        command: VesselCommand::EnrollAccount {
            command_id: Uuid::new_v4(),
            enrollment_id,
            workspace: p.workspace.clone(),
            connection_id: Uuid::new_v4(),
            alias: "work".into(),
            label: "Work".into(),
        },
    });
    p.mode = Mode::Enrollment;
    let v = PrivateEnrollmentStatus {
        status: EnrollmentStatus {
            enrollment_id,
            state: EnrollmentState::Pending,
            account_id: None,
            expires_at: now() + 60,
            effects_may_have_occurred: false,
        },
        user_code: Some("SYNTHETIC-1234".into()),
        verification_uri: Some("https://auth.openai.com/codex/device".into()),
    };
    p.private = Some(v.clone());
    v
}
fn assert_clean(app: &App, t: Target, dir: &std::path::Path) {
    assert_eq!(app.views[&t].draft.text, "preserve my composer");
    assert!(app.views[&t].snapshot.as_ref().unwrap().messages.is_empty());
    assert!(app.views[&t].history.entries.is_empty());
    assert!(!app.views[&t].history.draft.text.contains("SYNTHETIC-1234"));
    assert!(!app.status.contains("SYNTHETIC-1234"));
    if let Some(p) = &app.accounts.picker {
        assert!(!p.notice.contains("SYNTHETIC-1234"));
    }
    fn scan(dir: &std::path::Path) {
        for e in std::fs::read_dir(dir).unwrap() {
            let path = e.unwrap().path();
            if path.is_dir() {
                scan(&path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                let text = String::from_utf8_lossy(&bytes);
                assert!(!text.contains("SYNTHETIC-1234"));
                assert!(!text.contains("verification_uri"));
                assert!(!text.contains("user_code"));
            }
        }
    }
    scan(dir);
}
#[tokio::test]
async fn keyboard_and_mouse_picker_stage_exact_pending_without_touching_current_run_or_composer() {
    for mouse in [false, true] {
        let fixture = support::Fixture::new();
        let mut app = app(fixture.0.path());
        let t = live(&mut app);
        let _watch = picker(&mut app, t);
        let current = app.views[&t]
            .snapshot
            .as_ref()
            .unwrap()
            .inference_current
            .clone();
        if mouse {
            let rect = app.accounts.hits.borrow()[0].0;
            app.input(Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            }))
            .unwrap();
        } else {
            key(&mut app, KeyCode::Enter);
        }
        let p = app.accounts.picker.as_ref().unwrap();
        let Mode::Confirm(s) = &p.mode else {
            panic!("selection must require confirmation")
        };
        assert_eq!(s.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(s.service_tier.as_deref(), Some("default"));
        let selected = s.account.clone().unwrap();
        let id = p.id;
        app.accounts.reply = None; // replace the host effect with a bounded synthetic reply
        app.account_reply(id, Ok(Reply::Models(selected.clone(), vec![])))
            .unwrap();
        key(&mut app, KeyCode::Enter);
        assert!(app.accounts.picker.is_none());
        let view = &app.views[&t];
        assert!(view.snapshot.as_ref().unwrap().inference_current == current);
        let pending = view.pending.as_ref().unwrap();
        assert!(pending.preserve_draft);
        let VoyageCommand::Resolve {
            original: Some(original),
            ..
        } = pending.resolution()
        else {
            panic!("exact recovery required")
        };
        let VoyageCommand::SetAccountInference {
            account,
            expected_revision,
            model,
            reasoning_effort,
            service_tier,
            ..
        } = *original
        else {
            panic!("atomic inference envelope required")
        };
        assert_eq!(account, selected);
        assert_eq!(expected_revision, 17);
        assert_eq!(model, "synthetic-model");
        assert_eq!(reasoning_effort.as_deref(), Some("high"));
        assert_eq!(service_tier.as_deref(), Some("default"));
        assert_clean(&app, t, fixture.0.path());
    }
}
#[tokio::test]
async fn private_view_intercepts_paste_and_only_explicit_o_opens_verified_browser() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    material(&mut app);
    assert!(draw(&app, 110, 32).contains("SYNTHETIC-1234"));
    app.input(Event::Paste("SYNTHETIC-1234".into())).unwrap();
    key(&mut app, KeyCode::Char('x'));
    assert!(support::browsers().is_empty());
    key(&mut app, KeyCode::Char('o'));
    assert_eq!(
        support::browsers(),
        vec!["https://auth.openai.com/codex/device"]
    );
    let p = app.accounts.picker.as_ref().unwrap();
    storage::save(
        p.host.unwrap(),
        &p.workspace,
        &storage::Preferences {
            choices: vec![],
            enrollment: p.intent.clone(),
        },
    )
    .unwrap();
    drafts::save(&app.clients[t.route], &app.views[&t]).unwrap();
    assert_clean(&app, t, fixture.0.path());
    key(&mut app, KeyCode::Esc);
    assert!(app.accounts.picker.is_none());
    assert!(app.accounts.reply.is_none());
    assert_clean(&app, t, fixture.0.path());
}
#[tokio::test]
async fn loss_reconnect_between_ticks_discards_ready_and_late_private_replies() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let watch = picker(&mut app, t);
    let v = material(&mut app);
    let old_id = app.accounts.picker.as_ref().unwrap().id;
    let intent = serde_json::to_value(
        app.accounts
            .picker
            .as_ref()
            .unwrap()
            .intent
            .as_ref()
            .unwrap(),
    )
    .unwrap();
    let (tx, rx) = oneshot::channel();
    app.accounts.reply = Some((old_id, rx));
    assert!(tx.send(Ok(Reply::Private(v.clone()))).is_ok());
    watch
        .send(ConnectionState {
            socket_id: None,
            loss_generation: 5,
        })
        .unwrap();
    watch
        .send(ConnectionState {
            socket_id: Some(Uuid::new_v4()),
            loss_generation: 5,
        })
        .unwrap();
    // Drawing is safe even before the next regular UI tick processes loss.
    assert!(!draw(&app, 110, 32).contains("SYNTHETIC-1234"));
    app.account_tick();
    assert!(app.accounts.reply.is_none());
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.disconnected && p.private.is_none() && !p.busy);
    assert_eq!(
        serde_json::to_value(p.intent.as_ref().unwrap()).unwrap(),
        intent
    );
    app.account_reply(old_id, Ok(Reply::Private(v.clone())))
        .unwrap();
    key(&mut app, KeyCode::Char('o'));
    assert!(support::browsers().is_empty());
    assert!(app.accounts.picker.as_ref().unwrap().private.is_none());
    assert_clean(&app, t, fixture.0.path());
    key(&mut app, KeyCode::Char('r')); // explicit authenticated read, not Enroll replay
    let id = app.accounts.picker.as_ref().unwrap().id;
    app.accounts.reply = None;
    app.account_reply(id, Ok(Reply::Private(v))).unwrap();
    assert!(draw(&app, 110, 32).contains("SYNTHETIC-1234"));
    assert_eq!(
        serde_json::to_value(
            app.accounts
                .picker
                .as_ref()
                .unwrap()
                .intent
                .as_ref()
                .unwrap()
        )
        .unwrap(),
        intent
    );
}
#[tokio::test]
async fn expiry_denial_cancel_and_small_layout_clear_or_hide_material() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    material(&mut app);
    assert!(!draw(&app, 20, 8).contains("SYNTHETIC-1234"));
    key(&mut app, KeyCode::Char('o'));
    assert!(support::browsers().is_empty());
    draw(&app, 110, 32);
    app.accounts
        .picker
        .as_mut()
        .unwrap()
        .private
        .as_mut()
        .unwrap()
        .status
        .expires_at = now();
    app.account_tick();
    assert!(app.accounts.picker.as_ref().unwrap().private.is_none());
    let mut v = material(&mut app);
    v.user_code = Some(String::new());
    assert!(!active_material(&v));
    let id = app.accounts.picker.as_ref().unwrap().id;
    assert!(
        app.account_reply(id, Err(anyhow::anyhow!("access denied")))
            .is_err()
    );
    assert!(app.accounts.picker.as_ref().unwrap().private.is_none());
    material(&mut app);
    key(&mut app, KeyCode::Char('c'));
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none());
    assert!(p.intent.as_ref().unwrap().cancel.is_some());
    let cancel = p.intent.as_ref().unwrap().cancel;
    app.accounts.reply = None;
    app.accounts.picker.as_mut().unwrap().busy = false;
    app.cancel_enrollment().unwrap();
    assert_eq!(
        app.accounts
            .picker
            .as_ref()
            .unwrap()
            .intent
            .as_ref()
            .unwrap()
            .cancel,
        cancel
    );
    // Publication already won: report success rather than pretending cancellation undid it.
    let p = app.accounts.picker.as_ref().unwrap();
    let result = PrivateEnrollmentStatus {
        status: EnrollmentStatus {
            enrollment_id: p.intent.as_ref().and_then(enrollment_id).unwrap(),
            state: EnrollmentState::Succeeded,
            account_id: Some(Uuid::new_v4()),
            expires_at: now() + 30,
            effects_may_have_occurred: true,
        },
        user_code: Some("SYNTHETIC-1234".into()),
        verification_uri: None,
    };
    let id = p.id;
    app.accounts.reply = None;
    app.account_reply(id, Ok(Reply::Private(result))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none() && p.intent.is_none());
    assert!(p.notice.contains("Succeeded"));
    assert!(app.views[&t].pending.is_none());
    assert_clean(&app, t, fixture.0.path());
}
#[tokio::test]
async fn enrollment_only_connection_mouse_choice_and_escape_preserve_composer() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.catalogue.accounts.clear();
    p.catalogue.connections[0].transports = vec![Transport::ChatgptOauth];
    p.catalogue.connections[0].endpoint = "https://chatgpt.com/backend-api/codex".into();
    key(&mut app, KeyCode::Enter); // Add / Sign in
    assert!(matches!(
        app.accounts.picker.as_ref().unwrap().mode,
        Mode::Connections
    ));
    draw(&app, 110, 32);
    let rect = app.accounts.hits.borrow()[0].0;
    app.input(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    }))
    .unwrap();
    assert!(matches!(
        app.accounts.picker.as_ref().unwrap().mode,
        Mode::Alias(_)
    ));
    key(&mut app, KeyCode::Char('w'));
    key(&mut app, KeyCode::Esc);
    assert!(app.accounts.picker.is_none());
    assert!(support::browsers().is_empty());
    assert_clean(&app, t, fixture.0.path());
}

#[tokio::test]
async fn configured_first_send_persists_policy_and_resolves_the_exact_original_path() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let route = app.clients.first_route().unwrap();
    std::fs::create_dir(app.clients[route].directory.clone()).unwrap();
    let mut config = crate::Config::default();
    config.workspace = Some(fixture.0.path().into());
    config.system_prompt = "synthetic policy-preserving launch".into();
    config.command_timeout_secs = 37;
    config.terminal_max_count = 3;
    config.subagent_max_concurrency = 2;
    config.policy_explicit.github_enabled = Some(false);
    app.new_chat_config = Some(config.clone());
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let id = app.active_draft.unwrap();
    app.set_draft_account(id, Uuid::new_v4(), settings())
        .unwrap();
    app.new_draft_composer_mut(id).unwrap().text = "first synthetic turn".into();
    app.send_draft(id).unwrap();
    let saved = &app.new_drafts[&id].saved;
    let VesselCommand::StartAccount {
        config_path: Some(path),
        command_id,
        account,
        ..
    } = saved.start.as_ref().unwrap()
    else {
        panic!("configured draft must keep owner-local policy path")
    };
    let captured: voyage_runtime::launch_config::LaunchConfig =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let resolved = captured.resolve(fixture.0.path()).unwrap();
    assert_eq!(resolved.system_prompt, config.system_prompt);
    assert_eq!(resolved.command_timeout_secs, 37);
    assert_eq!(resolved.terminal_max_count, 3);
    assert_eq!(resolved.subagent_max_concurrency, 2);
    assert_eq!(resolved.policy_explicit.github_enabled, Some(false));
    let restored: super::super::new_draft::Saved =
        serde_json::from_slice(&serde_json::to_vec(saved).unwrap()).unwrap();
    let (recovered, envelope) = restored.start_resolution().unwrap();
    assert_eq!(*command_id, recovered);
    let VesselCommand::ResolveStartAccount {
        config_path,
        account: recovered_account,
        ..
    } = envelope
    else {
        panic!("observation-only resolution required")
    };
    assert_eq!(config_path.as_ref(), Some(path));
    assert_eq!(&recovered_account, account);
    assert_eq!(
        app.copy_new_draft_images(id).unwrap().0.text,
        "first synthetic turn"
    );
    // A repeated send must not allocate a new command/path while creation is pending.
    assert!(app.send_draft(id).is_err());
    assert_eq!(
        std::fs::read_dir(app.clients[route].directory.join("launch"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn draft_choice_remembering_is_separate_and_unavailable_preference_never_falls_back() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let draft = app.active_draft.unwrap();
    let original = settings();
    let host = Uuid::new_v4();
    app.set_draft_account(draft, host, original.clone())
        .unwrap();
    app.new_draft_composer_mut(draft).unwrap().text = "draft survives account selection".into();
    let _watch = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.destination = Destination::Draft(draft);
    p.workspace = fixture.0.path().into();
    p.host = Some(host);
    p.original = original;
    key(&mut app, KeyCode::Enter);
    let p = app.accounts.picker.as_ref().unwrap();
    let Mode::Confirm(s) = &p.mode else {
        panic!("confirmation required")
    };
    let binding = s.account.clone().unwrap();
    let id = p.id;
    app.accounts.reply = None;
    app.account_reply(id, Ok(Reply::Models(binding.clone(), vec![])))
        .unwrap();
    key(&mut app, KeyCode::F(2)); // explicit override reset only
    key(&mut app, KeyCode::F(4)); // local new-voyage preference only
    key(&mut app, KeyCode::Enter);
    let saved = &app.new_drafts[&draft].saved;
    assert_eq!(
        saved.account_settings.as_ref().unwrap().account,
        Some(binding.clone())
    );
    assert!(
        saved
            .account_settings
            .as_ref()
            .unwrap()
            .reasoning_effort
            .is_none()
    );
    assert!(
        saved
            .account_settings
            .as_ref()
            .unwrap()
            .service_tier
            .is_none()
    );
    assert_eq!(
        app.copy_new_draft_images(draft).unwrap().0.text,
        "draft survives account selection"
    );
    let prefs = storage::load(host, fixture.0.path()).unwrap();
    assert_eq!(
        prefs.choices,
        vec![(binding.connection_id, binding.clone())]
    );
    assert!(app.views[&t].pending.is_none());
    // New draft receives remembered identity even when absent from the catalogue.
    // It must remain visibly unresolved/unavailable, not silently use host fallback.
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let next = app.active_draft.unwrap();
    assert_ne!(next, draft);
    app.open_accounts(Destination::Draft(next), "").unwrap();
    let id = app.accounts.picker.as_ref().unwrap().id;
    app.accounts.reply = None;
    let mut fallback = settings();
    fallback.account.as_mut().unwrap().connection_id = binding.connection_id;
    app.account_reply(
        id,
        Ok(Reply::Loaded(Loaded {
            host,
            catalogue: Catalogue {
                accounts: vec![],
                connections: vec![],
            },
            defaults: fallback,
            enroll: true,
        })),
    )
    .unwrap();
    assert_eq!(
        app.new_drafts[&next]
            .saved
            .account_settings
            .as_ref()
            .unwrap()
            .account,
        Some(binding)
    );
    assert!(
        app.accounts
            .picker
            .as_ref()
            .unwrap()
            .choices()
            .iter()
            .all(|(_, b)| b.is_none())
    );
    assert_eq!(
        app.copy_new_draft_images(draft).unwrap().0.text,
        "draft survives account selection"
    );
}
