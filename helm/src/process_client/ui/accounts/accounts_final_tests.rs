//! Reply-state regressions with synthetic private channels and isolated storage.
use super::super::{
    account_test_support::Fixture,
    state::{Target, View},
};
use super::*;
use crate::process_client::duplex::ConnectionState;

fn catalogue() -> Catalogue {
    let binding = super::tests::binding();
    Catalogue {
        default_account: None,
        default_revision: 7,
        can_set_default: true,
        accounts: vec![AccountDescriptor {
            id: binding.account_id,
            connection_id: binding.connection_id,
            alias: "synthetic".into(),
            label: "Synthetic account".into(),
            metadata_revision: 1,
            identity_generation: binding.identity_generation,
            credential_revision: 1,
            capability_revision: 1,
            availability: CredentialAvailability::Available,
            state: AccountState::Ready,
        }],
        connections: vec![ConnectionDescriptor {
            id: binding.connection_id,
            revision: binding.connection_revision,
            label: "Synthetic connection".into(),
            endpoint: "https://example.invalid".into(),
            transports: vec![binding.transport],
        }],
    }
}

fn setup(f: &Fixture) -> (App, Target, tokio::sync::watch::Sender<ConnectionState>) {
    let mut app = super::app_tests::app(f.0.path());
    let target = Target {
        route: app.clients.first_route().unwrap(),
        session: Uuid::new_v4(),
    };
    let mut view = View::new(
        serde_json::from_value(serde_json::json!({
            "session_id": target.session, "incarnation": Uuid::new_v4(),
            "workspace": f.0.path(), "state": "live", "name": "synthetic"
        }))
        .unwrap(),
    );
    view.snapshot = Some(
        serde_json::from_value(serde_json::json!({
            "session_id": target.session, "revision": 4, "model": "original",
            "inference": {"model": "original", "provider": "fixture"},
            "messages": [], "run": null
        }))
        .unwrap(),
    );
    view.draft.text = "unsent composer".into();
    app.views.insert(target, view);
    app.selected = Some(target);
    let state = ConnectionState {
        socket_id: Some(Uuid::new_v4()),
        loss_generation: 3,
    };
    let (sender, receiver) = tokio::sync::watch::channel(state);
    app.accounts.picker = Some(Picker {
        auto_initialize: false,
        id: Uuid::new_v4(),
        destination: Destination::Live(target),
        route: target.route,
        workspace: f.0.path().into(),
        host: Some(Uuid::new_v4()),
        _lock: None,
        catalogue: catalogue(),
        original: app.inference_settings(Destination::Live(target)).unwrap(),
        mode: Mode::Enrollment,
        query: String::new(),
        selected: 0,
        usage: Default::default(),
        usage_requested: Default::default(),
        default_change_pending: false,
        enroll: true,
        notice: "before reply".into(),
        busy: true,
        intent: None,
        private: None,
        outcome: None,
        retry: None,
        restart: false,
        poll: Instant::now(),
        connection: receiver,
        loss_generation: state.loss_generation,
        disconnected: false,
    });
    (app, target, sender)
}

fn private(app: &mut App, state: EnrollmentState) -> PrivateEnrollmentStatus {
    let p = app.accounts.picker.as_mut().unwrap();
    let id = Uuid::new_v4();
    p.intent = Some(storage::Intent {
        host: p.host.unwrap(),
        workspace: p.workspace.clone(),
        cancel: None,
        command: VesselCommand::EnrollAccount {
            command_id: Uuid::new_v4(),
            enrollment_id: id,
            workspace: p.workspace.clone(),
            connection_id: Uuid::new_v4(),
            alias: "synthetic".into(),
            label: "Synthetic".into(),
        },
    });
    let value = PrivateEnrollmentStatus {
        failure: None,
        status: EnrollmentStatus {
            enrollment_id: id,
            state,
            account_id: None,
            expires_at: now() + 600,
            effects_may_have_occurred: false,
        },
        user_code: Some("FINAL-SYNTHETIC-CODE".into()),
        verification_uri: Some("https://auth.openai.com/codex/device".into()),
    };
    p.private = Some(value.clone());
    value
}

fn reply(app: &mut App, value: Result<Reply>) -> Result<()> {
    app.account_reply(app.accounts.picker.as_ref().unwrap().id, value)
}

fn unchanged(app: &App, t: Target) {
    assert_eq!(app.views[&t].draft.text, "unsent composer");
    assert!(app.views[&t].pending.is_none());
    assert!(app.views[&t].snapshot.as_ref().unwrap().messages.is_empty());
    assert_eq!(
        app.inference_settings(Destination::Live(t)).unwrap().model,
        "original"
    );
    assert!(!app.status.contains("FINAL-SYNTHETIC-CODE"));
    if let Some(p) = &app.accounts.picker {
        assert!(!p.notice.contains("FINAL-SYNTHETIC-CODE"));
    }
}

#[tokio::test]
async fn closed_picker_discards_private_reply() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    let id = app.accounts.picker.take().unwrap().id;
    app.account_reply(id, Ok(Reply::Private(value))).unwrap();
    assert!(app.accounts.picker.is_none());
    assert!(app.accounts.reply.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn replaced_picker_does_not_consume_old_failure() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    app.account_reply(Uuid::new_v4(), Err(anyhow::anyhow!("old failure")))
        .unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.busy);
    assert_eq!(p.notice, "before reply");
    assert_eq!(
        p.private.as_ref().unwrap().status.enrollment_id,
        value.status.enrollment_id
    );
    unchanged(&app, t);
}

#[tokio::test]
async fn disconnected_picker_does_not_accept_loaded_host() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    app.accounts.picker.as_mut().unwrap().disconnected = true;
    let host = Uuid::new_v4();
    reply(
        &mut app,
        Ok(Reply::Loaded(Loaded {
            host,
            catalogue: catalogue(),
            defaults: Settings::default(),
            enroll: false,
        })),
    )
    .unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert_ne!(p.host, Some(host));
    assert!(p.busy);
    assert!(app.account_host(t.route).is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn changed_socket_generation_rejects_queued_private_material() {
    let f = Fixture::new();
    let (mut app, t, connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    app.accounts.picker.as_mut().unwrap().private = None;
    connection
        .send(ConnectionState {
            socket_id: Some(Uuid::new_v4()),
            loss_generation: 4,
        })
        .unwrap();
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none());
    assert!(p.busy);
    assert_eq!(p.notice, "before reply");
    unchanged(&app, t);
}

#[tokio::test]
async fn access_denial_clears_code_without_erasing_original_intent() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    let error = reply(
        &mut app,
        Err(anyhow::anyhow!("Private account access denied")),
    )
    .unwrap_err();
    assert!(error.to_string().contains("denied"));
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(!p.busy);
    assert!(p.private.is_none());
    assert_eq!(
        p.intent.as_ref().and_then(enrollment_id),
        Some(value.status.enrollment_id)
    );
    assert!(p.notice.contains("denied"));
    unchanged(&app, t);
}

#[tokio::test]
async fn mismatched_enrollment_clears_code_but_retains_request_identity() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let mut value = private(&mut app, EnrollmentState::Pending);
    let original = value.status.enrollment_id;
    value.status.enrollment_id = Uuid::new_v4();
    assert!(reply(&mut app, Ok(Reply::Private(value))).is_err());
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none());
    assert_eq!(p.intent.as_ref().and_then(enrollment_id), Some(original));
    assert_eq!(p.notice, "Enrollment identity mismatch");
    assert!(p.outcome.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn mutation_acknowledgement_schedules_status_not_account_selection() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Starting);
    reply(&mut app, Ok(Reply::Mutation)).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(!p.busy);
    assert!(p.poll.elapsed() >= Duration::from_secs(3));
    assert!(p.notice.contains("does not select"));
    assert_eq!(
        p.intent.as_ref().and_then(enrollment_id),
        Some(value.status.enrollment_id)
    );
    unchanged(&app, t);
}

#[tokio::test]
async fn pending_reply_retains_private_material_only_in_picker() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(!p.busy);
    assert_eq!(
        p.private.as_ref().unwrap().user_code.as_deref(),
        Some("FINAL-SYNTHETIC-CODE")
    );
    assert_eq!(p.outcome, Some(EnrollmentState::Pending));
    assert_eq!(p.retry.as_ref().unwrap().1, "synthetic");
    assert!(p.intent.is_some());
    unchanged(&app, t);
}

#[tokio::test]
async fn uncertain_reply_retains_identity_without_active_code() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Uncertain);
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert_eq!(p.outcome, Some(EnrollmentState::Uncertain));
    let status = p.private.as_ref().unwrap();
    assert!(status.user_code.is_none());
    assert!(status.verification_uri.is_none());
    assert!(p.intent.is_some());
    assert!(p.notice.contains("could be confirmed"));
    unchanged(&app, t);
}

#[tokio::test]
async fn expired_pending_material_is_scrubbed_even_before_terminal_status() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let mut value = private(&mut app, EnrollmentState::Pending);
    value.status.expires_at = 0;
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    let status = p.private.as_ref().unwrap();
    assert!(status.user_code.is_none());
    assert!(status.verification_uri.is_none());
    assert!(p.intent.is_some());
    assert_eq!(p.outcome, Some(EnrollmentState::Pending));
    unchanged(&app, t);
}

#[tokio::test]
async fn terminal_denial_clears_persisted_intent_and_allows_explicit_restart() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Denied);
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.intent.is_none());
    assert!(p.private.is_none());
    assert_eq!(p.outcome, Some(EnrollmentState::Denied));
    assert!(
        storage::load(p.host.unwrap(), &p.workspace)
            .unwrap()
            .enrollment
            .is_none()
    );
    app.restart_enrollment().unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::Alias(_)));
    assert_eq!(p.query, "synthetic");
    assert!(p.notice.contains("previous code cannot be used"));
    assert!(app.accounts.reply.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn cancelled_restart_returns_to_alias_without_issuing_request() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Cancelled);
    app.accounts.picker.as_mut().unwrap().restart = true;
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(!p.restart);
    assert!(p.intent.is_none());
    assert!(p.private.is_none());
    assert!(matches!(p.mode, Mode::Alias(_)));
    assert_eq!(p.query, "synthetic");
    assert!(p.notice.contains("Enter requests a new code"));
    assert!(app.accounts.reply.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn expired_terminal_reply_does_not_automatically_restart() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Expired);
    reply(&mut app, Ok(Reply::Private(value))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::Enrollment));
    assert!(p.intent.is_none());
    assert!(p.retry.is_some());
    assert_eq!(p.outcome, Some(EnrollmentState::Expired));
    assert!(p.notice.contains("expired"));
    assert!(app.accounts.reply.is_none());
    unchanged(&app, t);
}

#[tokio::test]
async fn loaded_catalogue_caches_host_and_labels_without_changing_live_settings() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let host = Uuid::new_v4();
    let c = catalogue();
    let account = c.accounts[0].id;
    reply(
        &mut app,
        Ok(Reply::Loaded(Loaded {
            host,
            catalogue: c,
            defaults: Settings::default(),
            enroll: false,
        })),
    )
    .unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert_eq!(p.host, Some(host));
    assert!(!p.enroll);
    assert!(!p.busy);
    assert!(p._lock.is_some());
    assert_eq!(app.account_host(t.route), Some(host));
    assert_eq!(
        app.accounts.labels[&(t.route, account)],
        "Synthetic account"
    );
    unchanged(&app, t);
}

#[tokio::test]
async fn oversized_loaded_catalogue_is_rejected_before_caching_host() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let mut c = catalogue();
    c.accounts = vec![c.accounts[0].clone(); 129];
    let host = Uuid::new_v4();
    assert!(
        reply(
            &mut app,
            Ok(Reply::Loaded(Loaded {
                host,
                catalogue: c,
                defaults: Settings::default(),
                enroll: true,
            }))
        )
        .is_err()
    );
    let p = app.accounts.picker.as_ref().unwrap();
    assert_ne!(p.host, Some(host));
    assert!(p._lock.is_none());
    assert!(!p.busy);
    assert!(app.account_host(t.route).is_none());
    assert_eq!(p.catalogue.accounts.len(), 1);
    unchanged(&app, t);
}

#[tokio::test]
async fn catalogue_refresh_resets_filter_and_focus_without_applying_account() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let c = catalogue();
    let focus = c.accounts[0].id;
    let p = app.accounts.picker.as_mut().unwrap();
    p.query = "no match".into();
    p.selected = 80;
    p.usage_requested.insert(focus);
    reply(&mut app, Ok(Reply::Catalogue(c, Some(focus)))).unwrap();
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(matches!(p.mode, Mode::List));
    assert!(p.query.is_empty());
    assert_eq!(p.selected, 0);
    assert!(p.usage_requested.is_empty());
    assert!(p.notice.contains("current account has not changed"));
    unchanged(&app, t);
}

#[tokio::test]
async fn default_mismatch_keeps_pending_review_and_never_applies_settings() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let mut settings = Settings::default();
    settings.account = Some(super::tests::binding());
    let p = app.accounts.picker.as_mut().unwrap();
    p.mode = Mode::DefaultConsent(settings);
    p.default_change_pending = true;
    assert!(reply(&mut app, Ok(Reply::DefaultAccount(catalogue()))).is_err());
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(p.default_change_pending);
    assert!(matches!(p.mode, Mode::DefaultConsent(_)));
    assert!(p.notice.contains("do not repeat"));
    assert!(!p.busy);
    unchanged(&app, t);
}

#[tokio::test]
async fn empty_private_channel_remains_owned_until_a_reply_arrives() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    let id = app.accounts.picker.as_ref().unwrap().id;
    let (sender, receiver) = oneshot::channel();
    app.accounts.reply = Some((id, receiver));
    app.account_tick();
    assert_eq!(app.accounts.reply.as_ref().unwrap().0, id);
    assert!(app.accounts.picker.as_ref().unwrap().busy);
    assert!(sender.send(Ok(Reply::Private(value))).is_ok());
    app.account_tick();
    assert!(app.accounts.reply.is_none());
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(!p.busy);
    assert_eq!(p.outcome, Some(EnrollmentState::Pending));
    assert!(p.private.is_some());
    unchanged(&app, t);
}

#[tokio::test]
async fn closed_private_channel_retains_intent_and_stops_loading() {
    let f = Fixture::new();
    let (mut app, t, _connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    let id = app.accounts.picker.as_ref().unwrap().id;
    let (sender, receiver) = oneshot::channel();
    app.accounts.reply = Some((id, receiver));
    drop(sender);
    app.account_tick();
    assert!(app.accounts.reply.is_none());
    let p = app.accounts.picker.as_ref().unwrap();
    assert!(!p.busy);
    assert!(p.private.is_none());
    assert_eq!(
        p.intent.as_ref().and_then(enrollment_id),
        Some(value.status.enrollment_id)
    );
    assert!(p.notice.contains("Request interrupted"));
    assert!(p.notice.contains("not retry"));
    unchanged(&app, t);
}

#[tokio::test]
async fn connection_tick_invalidates_both_channels_before_consuming_queued_code() {
    let f = Fixture::new();
    let (mut app, t, connection) = setup(&f);
    let value = private(&mut app, EnrollmentState::Pending);
    let id = app.accounts.picker.as_ref().unwrap().id;
    let (sender, receiver) = oneshot::channel();
    assert!(sender.send(Ok(Reply::Private(value))).is_ok());
    app.accounts.reply = Some((id, receiver));
    let (_usage_sender, usage_receiver) = oneshot::channel();
    app.accounts.usage_reply = Some((id, usage_receiver));
    connection
        .send(ConnectionState {
            socket_id: None,
            loss_generation: 4,
        })
        .unwrap();
    app.account_tick();
    let p = app.accounts.picker.as_ref().unwrap();
    assert_ne!(p.id, id);
    assert!(p.disconnected);
    assert!(!p.busy);
    assert!(p.private.is_none());
    assert!(p.intent.is_some());
    assert!(p.outcome.is_none());
    assert!(app.accounts.reply.is_none());
    assert!(app.accounts.usage_reply.is_none());
    unchanged(&app, t);
}
