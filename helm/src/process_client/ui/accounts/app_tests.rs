//! Focused App journeys. Synthetic watch/one-shot inputs exercise the real picker,
//! persistence and input paths; no provider, socket server or browser is contacted.
use super::super::{
    account_test_support as support,
    state::{Target, View},
};
use super::*;
use crate::process_client::{duplex::ConnectionState, transport::Client};
use crossterm::event::KeyEvent;
use ratatui::{Terminal, backend::TestBackend};
use std::collections::BTreeMap;

pub(in crate::process_client::ui) fn app(dir: &std::path::Path) -> App {
    let clients = super::super::routes::Routes::new(vec![Client::local(dir.join("no-vessel"))]);
    let (sender, _receiver) = tokio::sync::mpsc::channel(32);
    App {
        workspace: Default::default(),
        stop_review: None,
        viewport: Default::default(),
        observation_target: tokio::sync::watch::channel(None).0,
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
        discovery: Default::default(),
        workflows: Default::default(),
        operator: None,
        operator_loading: None,
        inspection: Default::default(),
        voyage_picker: None,
        interactions: Default::default(),
        terminal_request: None,
        completion: Default::default(),
        inference: Default::default(),
        accounts: Default::default(),
        sidebar: Default::default(),
    }
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
        auto_initialize: false,
        id: Uuid::new_v4(),
        destination: Destination::Live(t),
        route: t.route,
        workspace: "/synthetic-workspace".into(),
        host: Some(Uuid::new_v4()),
        _lock: None,
        catalogue: Catalogue {
            default_account: None,
            default_revision: 0,
            can_set_default: true,
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
        usage: Default::default(),
        usage_requested: Default::default(),
        default_change_pending: false,
        enroll: true,
        notice: String::new(),
        busy: false,
        intent: None,
        private: None,
        outcome: None,
        retry: None,
        restart: false,
        poll: Instant::now(),
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
        failure: None,
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
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let binding = app.accounts.picker.as_ref().unwrap().choices()[0]
        .1
        .clone()
        .unwrap();
    app.choose_account(binding).unwrap();
    assert!(!app.accounts.open());
    assert!(app.inference_picker_open());
    assert!(app.views[&t].pending.is_none());
    assert_eq!(app.views[&t].draft.text, "preserve my composer");
    app.cancel_model_catalog();
    for job in app.retired_observers {
        let _ = job.await;
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
    receipts::save(&app.clients[t.route], &app.views[&t]).unwrap();
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
        failure: None,
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
    assert!(p.notice.contains("Signed in"));
    assert!(app.views[&t].pending.is_none());
    assert_clean(&app, t, fixture.0.path());
}
#[tokio::test]
async fn single_connection_signin_skips_metadata_and_back_preserves_composer() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.catalogue.accounts.clear();
    p.catalogue.connections[0].transports = vec![Transport::ChatgptOauth];
    p.catalogue.connections[0].endpoint = "https://chatgpt.com/backend-api/codex".into();
    key(&mut app, KeyCode::Enter); // Add / Sign in
    // One supported connection skips internal connection selection.
    assert!(matches!(
        app.accounts.picker.as_ref().unwrap().mode,
        Mode::Alias(_)
    ));
    key(&mut app, KeyCode::Char('w'));
    key(&mut app, KeyCode::Esc);
    assert!(matches!(
        app.accounts.picker.as_ref().unwrap().mode,
        Mode::List
    ));
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
    let mut config = crate::Config {
        workspace: Some(fixture.0.path().into()),
        system_prompt: "synthetic policy-preserving launch".into(),
        command_timeout_secs: 37,
        terminal_max_count: 3,
        subagent_max_concurrency: 2,
        ..Default::default()
    };
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
async fn draft_override_does_not_change_new_voyage_default() {
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
    let _watch = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.destination = Destination::Draft(draft);
    p.original = original.clone();
    p.host = Some(host);
    let binding = p.choices()[0].1.clone().unwrap();
    app.choose_account(binding).unwrap();
    assert!(!app.accounts.open());
    assert!(app.draft_inference_settings(draft).unwrap() == original);
    app.cancel_model_catalog();
    for job in app.retired_observers {
        let _ = job.await;
    }
}

#[tokio::test]
async fn enrollment_refresh_keeps_code_and_restart_waits_for_confirmed_close() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let mut value = material(&mut app);
    let original = value.status.enrollment_id;
    app.poll_account_enrollment().unwrap();
    assert!(draw(&app, 80, 24).contains("SYNTHETIC-1234"));
    app.accounts.reply = None;
    let id = app.accounts.picker.as_ref().unwrap().id;
    value.status.state = EnrollmentState::Exchanging;
    app.account_reply(id, Ok(Reply::Private(value.clone())))
        .unwrap();
    assert!(draw(&app, 80, 24).contains("SYNTHETIC-1234"));
    value.status.state = EnrollmentState::Uncertain;
    value.failure = Some(EnrollmentFailure {
        phase: EnrollmentPhase::Poll,
        kind: EnrollmentFailureKind::InvalidResponse,
        http_status: Some(502),
    });
    app.account_reply(id, Ok(Reply::Private(value.clone())))
        .unwrap();
    let screen = draw(&app, 80, 24);
    assert!(!screen.contains("SYNTHETIC-1234"));
    assert!(screen.contains("N new sign-in"));
    assert!(screen.contains("HTTP 502"));
    key(&mut app, KeyCode::Char('n'));
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.restart && p.busy);
    assert_eq!(p.intent.as_ref().and_then(enrollment_id), Some(original));
    assert!(p.intent.as_ref().unwrap().cancel.is_some());
    assert!(matches!(p.mode, Mode::Enrollment));
    app.accounts.reply = None;
    value.status.state = EnrollmentState::Cancelled;
    app.account_reply(id, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::Alias(_)));
    assert_eq!(p.query, "work");
    assert!(p.intent.is_none());
    assert!(
        storage::load(p.host.unwrap(), &p.workspace)
            .unwrap()
            .enrollment
            .is_none()
    );
    key(&mut app, KeyCode::Enter);
    let p = app.accounts.picker.as_ref().unwrap();
    assert_ne!(p.intent.as_ref().and_then(enrollment_id), Some(original));
    assert!(matches!(p.mode, Mode::Enrollment));
    assert!(p.busy);
    assert_clean(&app, t, fixture.0.path());
}

#[tokio::test]
async fn late_success_during_restart_refreshes_choices_without_switching_account() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let original = app.inference_settings(Destination::Live(t)).unwrap();
    let _watch = picker(&mut app, t);
    let mut value = material(&mut app);
    draw(&app, 80, 24);
    key(&mut app, KeyCode::Char('n'));
    app.accounts.reply = None;
    let p = app.accounts.picker.as_ref().unwrap();
    let id = p.id;
    let account = p.catalogue.accounts[0].id;
    value.status.state = EnrollmentState::Succeeded;
    value.status.account_id = Some(account);
    app.account_reply(id, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none() && p.intent.is_none() && p.retry.is_none());
    assert!(!p.restart && p.busy);
    let catalogue = Catalogue {
        default_account: None,
        default_revision: 0,
        can_set_default: true,
        accounts: p.catalogue.accounts.clone(),
        connections: p.catalogue.connections.clone(),
    };
    app.accounts.reply = None;
    app.account_reply(id, Ok(Reply::Catalogue(catalogue, Some(account))))
        .unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::List));
    assert_eq!(p.selected, 0);
    assert!(p.notice.contains("Signed in"));
    assert!(app.views[&t].pending.is_none());
    assert!(app.inference_settings(Destination::Live(t)).unwrap() == original);
    assert_clean(&app, t, fixture.0.path());
}

#[tokio::test]
async fn compact_account_settings_distinguish_current_default_and_real_identity_changes() {
    assert!(!include_str!("render.rs").contains("Settings for next run"));
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let binding = app.accounts.picker.as_ref().unwrap().choices()[0]
        .1
        .clone()
        .unwrap();
    app.choose_account(binding).unwrap();
    assert!(!app.accounts.open());
    assert!(app.inference_picker_open());
    assert!(app.views[&t].pending.is_none());
    app.cancel_model_catalog();
    for job in app.retired_observers {
        let _ = job.await;
    }
}

#[tokio::test]
async fn usage_replies_require_current_identity_and_capability_revision() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _socket = picker(&mut app, t);
    let p = app.accounts.picker.as_ref().unwrap();
    let id = p.id;
    let mut observation = AccountUsageObservation {
        account: p.choices()[0].1.clone().unwrap(),
        capability_revision: 2,
        snapshot: None,
        refresh_status: AccountUsageRefreshStatus::NeverObserved,
        attempted_at: None,
    };
    assert!(
        app.account_reply(id, Ok(Reply::Usage(observation.clone())))
            .is_err()
    );
    assert!(app.accounts.picker.as_ref().unwrap().usage.is_empty());
    observation.capability_revision = 1;
    app.account_reply(id, Ok(Reply::Usage(observation)))
        .unwrap();
    assert_eq!(app.accounts.picker.as_ref().unwrap().usage.len(), 1);
    assert!(app.views[&t].pending.is_none());
}

#[tokio::test]
async fn disconnected_enter_opens_vessels_and_preserves_composer() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    app.vessels = Some(std::cell::RefCell::new(
        super::super::vessels::Manager::open(fixture.0.path().join("connections")).unwrap(),
    ));
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    material(&mut app);
    app.clients.mark_unavailable(t.route);
    app.account_tick();
    let id = app.accounts.picker.as_ref().unwrap().id;
    app.account_tick();
    assert_eq!(app.accounts.picker.as_ref().unwrap().id, id);
    let screen = draw(&app, 110, 32);
    assert!(screen.contains("Open Vessels to reconnect"));
    assert!(!screen.contains("SYNTHETIC-1234"));
    key(&mut app, KeyCode::Enter);
    assert!(app.accounts.picker.is_none());
    assert!(app.accounts.reply.is_none());
    assert!(app.vessels_open());
    assert_eq!(app.views[&t].draft.text, "preserve my composer");
}

#[tokio::test]
async fn account_ctrl_g_reaches_vessels_even_while_busy() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    app.vessels = Some(std::cell::RefCell::new(
        super::super::vessels::Manager::open(fixture.0.path().join("connections")).unwrap(),
    ));
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    material(&mut app);
    app.accounts.picker.as_mut().unwrap().busy = true;
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Char('g'),
        KeyModifiers::CONTROL,
    )))
    .unwrap();
    assert!(app.accounts.picker.is_none());
    assert!(app.vessels_open());
    assert!(support::browsers().is_empty());
}

#[tokio::test]
async fn enter_failure_is_visible_inside_account_modal() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.enroll = false;
    p.selected = p.choices().len() - 2;
    key(&mut app, KeyCode::Enter);
    assert!(draw(&app, 110, 32).contains("Device sign-in is not authorized/supported here"));
    app.account_tick();
    assert!(draw(&app, 110, 32).contains("Device sign-in is not authorized/supported here"));
    assert_eq!(app.views[&t].draft.text, "preserve my composer");
}

#[tokio::test]
async fn reconnected_enter_reloads_accounts_without_replaying_enrollment() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let watch = picker(&mut app, t);
    let old_id = app.accounts.picker.as_ref().unwrap().id;
    watch
        .send(ConnectionState {
            socket_id: Some(Uuid::new_v4()),
            loss_generation: 5,
        })
        .unwrap();
    app.account_tick();
    assert!(draw(&app, 110, 32).contains("Reload accounts"));
    key(&mut app, KeyCode::Enter);
    let p = app.accounts.picker.as_ref().unwrap();
    assert_ne!(p.id, old_id);
    assert!(!p.disconnected);
    assert!(p.busy && p.private.is_none() && p.intent.is_none());
    assert!(app.accounts.reply.is_some());
    assert_eq!(app.views[&t].draft.text, "preserve my composer");
}

#[tokio::test]
async fn host_default_requires_explicit_consent_and_escape_has_no_effect() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _socket = picker(&mut app, t);
    draw(&app, 100, 30);
    key(&mut app, KeyCode::F(6));
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::DefaultConsent(_)));
    assert!(!p.busy && !p.default_change_pending);
    assert!(app.accounts.reply.is_none());
    let screen = draw(&app, 100, 30);
    assert!(screen.contains("persistent default") && screen.contains("billing"));
    key(&mut app, KeyCode::Esc);
    assert!(matches!(
        app.accounts.picker.as_ref().unwrap().mode,
        Mode::List
    ));
    assert!(app.accounts.reply.is_none());
    assert!(app.views[&t].pending.is_none());
}

#[tokio::test]
async fn api_setup_has_friendly_private_commands_without_identity_detour() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _socket = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.selected = p.choices().len() - 1;
    draw(&app, 120, 40);
    key(&mut app, KeyCode::Enter);
    let screen = draw(&app, 120, 40);
    assert!(screen.contains("--provider openai") && screen.contains("--provider anthropic"));
    assert!(screen.contains("never paste") && screen.contains("separately"));
    assert!(!screen.contains("--connection") && !screen.contains("UUID"));
    assert!(app.accounts.reply.is_none());
}

#[tokio::test]
async fn missing_default_is_part_of_draft_apply_not_an_f6_prerequisite() {
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
    app.new_draft_composer_mut(draft).unwrap().text = "retained first task".into();
    let _socket = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.destination = Destination::Draft(draft);
    p.original = original.clone();
    p.mode = Mode::List;
    p.host = Some(host);
    app.apply_account(original).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::DefaultConsent(_)));
    assert!(!p.busy && !p.default_change_pending);
    assert!(app.new_drafts[&draft].saved.start.is_none());
    assert_eq!(
        app.copy_new_draft_images(draft).unwrap().0.text,
        "retained first task"
    );
}

#[tokio::test]
async fn default_save_is_not_readiness_until_catalogue_confirms_exact_account() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _socket = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    let expected = p.choices()[0].1.clone().unwrap();
    let mut settings = p.original.clone();
    settings.account = Some(expected.clone());
    p.mode = Mode::DefaultConsent(settings);
    p.default_change_pending = true;
    p.busy = true;
    let id = p.id;
    let catalogue = Catalogue {
        default_account: None,
        default_revision: 1,
        can_set_default: true,
        accounts: p.catalogue.accounts.clone(),
        connections: p.catalogue.connections.clone(),
    };
    assert!(
        app.account_reply(id, Ok(Reply::DefaultAccount(catalogue)))
            .is_err()
    );
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.default_change_pending && matches!(p.mode, Mode::DefaultConsent(_)));
    assert!(p.notice.contains("not confirmed"));
    assert!(app.set_default_account().is_err());
    assert!(app.views[&t].pending.is_none());
}

#[tokio::test]
async fn one_chatgpt_destination_goes_directly_to_account_name_without_auth_effect() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _socket = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.catalogue.accounts.clear();
    p.catalogue.connections[0].transports = vec![Transport::ChatgptOauth];
    p.catalogue.connections[0].endpoint = "https://chatgpt.com/backend-api/codex".into();
    draw(&app, 120, 40);
    key(&mut app, KeyCode::Enter);
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::Alias(_)));
    assert!(p.intent.is_none() && !p.busy);
    assert!(app.accounts.reply.is_none());
    assert!(draw(&app, 120, 40).contains("Name this account"));
}

#[tokio::test]
async fn unresolved_first_send_opens_setup_and_keeps_original_text() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let draft = app.active_draft.unwrap();
    app.new_draft_composer_mut(draft).unwrap().text = "explain this project".into();
    app.send_draft(draft).unwrap();
    assert!(!app.accounts.open());
    assert!(app.chooser_initializing());
    assert!(app.new_drafts[&draft].saved.start.is_none());
    assert!(!app.new_drafts[&draft].busy);
    assert_eq!(
        app.copy_new_draft_images(draft).unwrap().0.text,
        "explain this project"
    );
}

#[tokio::test]
async fn new_provider_advertised_default_is_reviewed_not_silently_applied() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _watch = picker(&mut app, t);
    let original = app.inference_settings(Destination::Live(t)).unwrap();
    let binding = app.accounts.picker.as_ref().unwrap().choices()[0]
        .1
        .clone()
        .unwrap();
    app.choose_account(binding).unwrap();
    assert!(app.inference_settings(Destination::Live(t)).unwrap() == original);
    assert!(app.views[&t].pending.is_none());
    assert!(!app.accounts.open());
    app.cancel_model_catalog();
    for job in app.retired_observers {
        let _ = job.await;
    }
}

#[tokio::test]
async fn confirmed_default_returns_to_review_without_sending_draft() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    app.create(Some(fixture.0.path().to_str().unwrap()))
        .unwrap();
    let d = app.active_draft.unwrap();
    let settings = settings();
    let host = Uuid::new_v4();
    app.set_draft_account(d, host, settings.clone()).unwrap();
    let _watch = picker(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.destination = Destination::Draft(d);
    p.original = settings.clone();
    p.host = Some(host);
    p.mode = Mode::DefaultConsent(settings.clone());
    p.busy = true;
    p.default_change_pending = true;
    let id = p.id;
    let catalogue = Catalogue {
        default_account: settings.account.clone(),
        default_revision: 1,
        can_set_default: true,
        accounts: p.catalogue.accounts.clone(),
        connections: p.catalogue.connections.clone(),
    };
    app.account_reply(id, Ok(Reply::DefaultAccount(catalogue)))
        .unwrap();
    assert!(!app.accounts.open());
    assert!(app.new_drafts[&d].saved.start.is_none());
    assert!(app.draft_inference_settings(d).unwrap() == settings);
}

#[tokio::test]
async fn discovery_app_retains_composer_images_and_refuses_unavailable() {
    use base64::Engine as _;
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let bytes = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==").unwrap();
    let image = super::super::attachments::Image::from_bytes("pixel.png".into(), &bytes).unwrap();
    let id = image.metadata().id;
    let view = app.views.get_mut(&t).unwrap();
    view.draft.cursor = view.draft.text.len();
    view.draft.insert_image(id);
    view.images.push(image);
    let before = serde_json::to_value(
        super::super::attachments::content(&view.draft, &view.images).unwrap(),
    )
    .unwrap();
    key(&mut app, KeyCode::F(8));
    app.input(Event::Paste("not composer text".into())).unwrap();
    for c in "settings".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    key(&mut app, KeyCode::Enter);
    assert!(app.help && app.discovery.settings);
    key(&mut app, KeyCode::Esc);
    assert!(!app.help);
    app.selected = None;
    assert!(
        app.discovery_reason("tools")
            .unwrap()
            .contains("Select a voyage")
    );
    app.discovery_open("tools").unwrap();
    assert!(app.operator.is_none());
    assert!(app.status.contains("Select a voyage"));
    let view = &app.views[&t];
    assert_eq!(
        before,
        serde_json::to_value(
            super::super::attachments::content(&view.draft, &view.images).unwrap()
        )
        .unwrap()
    );
    assert_eq!(view.images[0].metadata().id, id);
}

#[tokio::test]
async fn discovery_private_f1_overlays_without_dispatch_or_disclosure() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let _connection = picker(&mut app, t);
    material(&mut app);
    let intent = app
        .accounts
        .picker
        .as_ref()
        .unwrap()
        .intent
        .as_ref()
        .unwrap()
        .command
        .clone();
    key(&mut app, KeyCode::F(1));
    assert!(app.help);
    assert!(app.discovery_help().contains("Accounts: private"));
    assert!(!app.discovery_help().contains("SYNTHETIC-1234"));
    for (width, height) in [(80, 24), (40, 18)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| super::super::render::draw(frame, &app))
            .unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(rendered.contains("Accounts: private"));
        assert!(!rendered.contains("SYNTHETIC-1234"));
    }
    app.input(Event::Paste("do not dispatch".into())).unwrap();
    key(&mut app, KeyCode::Char('o'));
    key(&mut app, KeyCode::Enter);
    key(&mut app, KeyCode::Esc);
    assert!(!app.help);
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_some() && !p.busy);
    assert_eq!(
        serde_json::to_value(&p.intent.as_ref().unwrap().command).unwrap(),
        serde_json::to_value(intent).unwrap()
    );
    assert_clean(&app, t, fixture.0.path());
}

#[tokio::test]
async fn discovery_draft_focus_and_detach_never_edit_or_send_hidden_composer() {
    let fixture = support::Fixture::new();
    let mut app = app(fixture.0.path());
    let t = live(&mut app);
    let before = app.views[&t].draft.text.clone();
    // A draft identity need not have a live voyage; F8 must work before first send.
    app.active_draft = Some(Uuid::new_v4());
    key(&mut app, KeyCode::F(8));
    assert!(app.explore.is_some());
    for c in "settings".chars() {
        key(&mut app, KeyCode::Char(c));
    }
    app.input(Event::Paste("do not send".into())).unwrap();
    assert_eq!(app.discovery.query, "settings");
    assert_eq!(app.views[&t].draft.text, before);
    app.input(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    )))
    .unwrap();
    assert!(app.quit);
    assert!(app.command_checks.is_empty());
}

fn two_account_catalogue(
    app: &mut App,
    target: Target,
) -> (
    tokio::sync::watch::Sender<ConnectionState>,
    AccountBinding,
    AccountBinding,
) {
    let watch = picker(app, target);
    let p = app.accounts.picker.as_mut().unwrap();
    let first = p.choices()[0].1.clone().unwrap();
    let mut descriptor = p.catalogue.accounts[0].clone();
    descriptor.id = Uuid::new_v4();
    descriptor.alias = "second".into();
    descriptor.label = "Second default".into();
    let mut second = first.clone();
    second.account_id = descriptor.id;
    p.catalogue.accounts.push(descriptor);
    p.catalogue.default_account = Some(second.clone());
    (watch, first, second)
}
#[tokio::test]
async fn default_second_is_preselected_not_first_and_explicit_account_wins() {
    let f = support::Fixture::new();
    let mut app = app(f.0.path());
    let t = live(&mut app);
    let (_watch, first, second) = two_account_catalogue(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    p.original.account = None;
    p.preselect();
    assert_eq!(p.selected, 1);
    assert_eq!(p.choices()[p.selected].1, Some(second.clone()));
    p.original.account = Some(first.clone());
    p.preselect();
    assert_eq!(p.selected, 0);
    p.original.account = Some(second);
    p.preselect();
    assert_eq!(p.selected, 1);
    assert!(app.views[&t].pending.is_none());
}
#[tokio::test]
async fn saved_unbound_draft_resolves_second_default_before_chooser_without_confirmation_loop() {
    let f = support::Fixture::new();
    let mut app = app(f.0.path());
    let t = live(&mut app);
    app.create(Some(f.0.path().to_str().unwrap())).unwrap();
    let d = app.active_draft.unwrap();
    let (_watch, _first, second) = two_account_catalogue(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    let host = p.host.unwrap();
    p.destination = Destination::Draft(d);
    p.workspace = f.0.path().into();
    p.auto_initialize = true;
    let mut seed = settings();
    seed.account = None;
    let mut defaults = seed.clone();
    defaults.account = Some(second.clone());
    p.original = seed.clone();
    let id = p.id;
    let catalogue = Catalogue {
        accounts: p.catalogue.accounts.clone(),
        connections: p.catalogue.connections.clone(),
        default_account: p.catalogue.default_account.clone(),
        default_revision: p.catalogue.default_revision,
        can_set_default: p.catalogue.can_set_default,
    };
    app.set_draft_account(d, host, seed).unwrap();
    app.new_draft_composer_mut(d).unwrap().text = "retained draft".into();
    app.account_reply(
        id,
        Ok(Reply::Loaded(Loaded {
            host,
            catalogue,
            defaults,
            enroll: true,
        })),
    )
    .unwrap();
    assert!(!app.accounts.open());
    assert_eq!(
        app.draft_inference_settings(d).unwrap().account,
        Some(second)
    );
    assert_eq!(
        app.copy_new_draft_images(d).unwrap().0.text,
        "retained draft"
    );
    assert!(app.new_drafts[&d].saved.start.is_none());
}
#[tokio::test]
async fn unavailable_default_does_not_select_first_available_account_or_close_setup() {
    let f = support::Fixture::new();
    let mut app = app(f.0.path());
    let t = live(&mut app);
    app.create(Some(f.0.path().to_str().unwrap())).unwrap();
    let d = app.active_draft.unwrap();
    let (_watch, first, second) = two_account_catalogue(&mut app, t);
    let p = app.accounts.picker.as_mut().unwrap();
    let host = p.host.unwrap();
    p.destination = Destination::Draft(d);
    p.auto_initialize = true;
    p.catalogue.accounts[1].state = AccountState::SignInRequired;
    let catalogue = Catalogue {
        accounts: p.catalogue.accounts.clone(),
        connections: p.catalogue.connections.clone(),
        default_account: p.catalogue.default_account.clone(),
        default_revision: p.catalogue.default_revision,
        can_set_default: p.catalogue.can_set_default,
    };
    let id = p.id;
    let mut defaults = settings();
    defaults.account = Some(second.clone());
    app.account_reply(
        id,
        Ok(Reply::Loaded(Loaded {
            host,
            catalogue,
            defaults,
            enroll: true,
        })),
    )
    .unwrap();
    assert!(app.accounts.open());
    assert_ne!(
        app.draft_inference_settings(d).unwrap().account,
        Some(first)
    );
    assert!(app.new_drafts[&d].saved.start.is_none());
}
#[test]
fn default_resolution_preserves_explicit_binding_and_provider_mismatch() {
    let mut defaults = settings();
    let mut explicit = defaults.clone();
    explicit.account.as_mut().unwrap().account_id = Uuid::new_v4();
    assert_eq!(
        resolve_draft_default(Some(explicit.clone()), defaults.clone()).account,
        explicit.account
    );
    let mut wrong = defaults.clone();
    wrong.account = None;
    wrong.provider = "anthropic".into();
    assert!(
        resolve_draft_default(Some(wrong), defaults.clone())
            .account
            .is_none()
    );
    defaults.account = None;
    assert!(resolve_draft_default(None, defaults).account.is_none());
}

#[tokio::test]
async fn coverage_accounts_render_empty_search_busy_and_disconnected_states() {
    let fixture = tempfile::tempdir().unwrap();
    let mut app = app(fixture.path());
    let target = live(&mut app);
    let _connection = picker(&mut app, target);
    assert!(draw(&app, 110, 32).contains("Choose account"));
    app.accounts.picker.as_mut().unwrap().query = "no-such-synthetic-account".into();
    let empty = draw(&app, 110, 32);
    assert!(!empty.is_empty());
    app.accounts.picker.as_mut().unwrap().busy = true;
    assert!(draw(&app, 110, 32).contains("Choose account"));
    app.accounts.picker.as_mut().unwrap().disconnected = true;
    app.accounts.picker.as_mut().unwrap().notice = "synthetic connection lost".into();
    assert!(draw(&app, 110, 32).contains("synthetic connection lost"));
    assert_eq!(app.views[&target].draft.text, "preserve my composer");
}

#[tokio::test]
async fn coverage_accounts_small_render_hides_interactive_hits() {
    let fixture = tempfile::tempdir().unwrap();
    let mut app = app(fixture.path());
    let target = live(&mut app);
    let _connection = picker(&mut app, target);
    assert!(app.accounts.visible.get());
    let small = draw(&app, 40, 20);
    assert!(small.contains("Enlarge"));
    assert!(!app.accounts.visible.get());
    assert!(app.accounts.hits.borrow().is_empty());
    for (width, height) in [(1, 1), (10, 5), (44, 22)] {
        let _ = draw(&app, width, height);
    }
    app.accounts.picker = None;
    let _ = draw(&app, 110, 32);
    assert!(!app.accounts.visible.get());
    assert!(app.accounts.hits.borrow().is_empty());
}

#[tokio::test]
async fn coverage_accounts_render_connection_and_api_setup_modes() {
    let fixture = tempfile::tempdir().unwrap();
    let mut app = app(fixture.path());
    let target = live(&mut app);
    let _connection = picker(&mut app, target);
    app.accounts.picker.as_mut().unwrap().mode = Mode::Connections;
    assert!(draw(&app, 110, 32).contains("Sign in to an account"));
    app.accounts.picker.as_mut().unwrap().mode = Mode::ApiSetup;
    assert!(draw(&app, 110, 32).contains("Private API setup"));
    assert!(app.views[&target].pending.is_none());
}

#[tokio::test]
async fn coverage_catalogue_refresh_is_identity_fenced_and_preserves_authored_state() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let target = live(&mut app);
    let _socket = picker(&mut app, target);
    let id = app.accounts.picker.as_ref().unwrap().id;
    let mut catalogue = copy_catalogue(&app.accounts.picker.as_ref().unwrap().catalogue);
    catalogue.accounts[0].label = "Fresh label".into();
    let account = catalogue.accounts[0].id;
    app.accounts.picker.as_mut().unwrap().query = "stale search".into();
    app.account_reply(
        Uuid::new_v4(),
        Ok(Reply::Catalogue(copy_catalogue(&catalogue), Some(account))),
    )
    .unwrap();
    assert_eq!(app.accounts.picker.as_ref().unwrap().query, "stale search");
    app.account_reply(id, Ok(Reply::Catalogue(catalogue, Some(account))))
        .unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.query.is_empty());
    assert!(p.notice.contains("Signed in"));
    assert_eq!(app.accounts.labels[&(target.route, account)], "Fresh label");
    assert_eq!(app.views[&target].draft.text, "preserve my composer");
    assert!(app.views[&target].pending.is_none());
    app.accounts.picker.as_mut().unwrap().disconnected = true;
    app.account_reply(id, Err(anyhow::anyhow!("late private failure")))
        .unwrap();
    assert!(
        !app.accounts
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("late private")
    );
}
#[tokio::test]
async fn coverage_loaded_catalogue_rejects_oversize_without_starting_enrollment() {
    let root = tempfile::tempdir().unwrap();
    let mut app = app(root.path());
    let target = live(&mut app);
    let _socket = picker(&mut app, target);
    let id = app.accounts.picker.as_ref().unwrap().id;
    let mut catalogue = copy_catalogue(&app.accounts.picker.as_ref().unwrap().catalogue);
    catalogue.accounts = vec![catalogue.accounts[0].clone(); 129];
    assert!(
        app.account_reply(id, Ok(Reply::Catalogue(catalogue, None)))
            .is_err()
    );
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.notice.contains("limits"));
    assert!(!p.busy);
    assert!(p.intent.is_none());
    assert_eq!(app.views[&target].draft.text, "preserve my composer");
}

fn copy_catalogue(c: &Catalogue) -> Catalogue {
    Catalogue {
        accounts: c.accounts.clone(),
        connections: c.connections.clone(),
        default_account: c.default_account.clone(),
        default_revision: c.default_revision,
        can_set_default: c.can_set_default,
    }
}
