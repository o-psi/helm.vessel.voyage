//! Event/callback journeys over real maintained loopback transport. The host
//! returns only synthetic metadata; no executing runtime/provider is invoked.
use super::super::{
    account_test_support::Fixture,
    accounts::app_tests,
    socket_support_tests::{Server, voyage},
    state::View,
};
use super::*;
use crossterm::event::{KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use voyage_protocol::{
    accounts::{AccountBinding, Transport},
    vessel::VesselCommand,
};

struct Host {
    id: Uuid,
    account: AccountBinding,
    inventory: Value,
    failure: bool,
    wrong_account: bool,
    calls: Vec<VesselCommand>,
}
struct Peer {
    app: App,
    target: Target,
    host: Arc<Mutex<Host>>,
    updates: tokio::sync::mpsc::Receiver<Update>,
    _server: Server,
    _fixture: Fixture,
}
impl Peer {
    async fn new() -> Self {
        let fixture = Fixture::new();
        let account = AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 4,
            connection_revision: 6,
            transport: Transport::OpenaiResponses,
        };
        let host = Arc::new(Mutex::new(Host {
            id: Uuid::new_v4(),
            account: account.clone(),
            inventory: json!([
                {"id":"current-custom","display_name":"Current","reasoning_efforts":["low","high"],"service_tiers":["standard","priority"]},
                {"id":"remote-default","display_name":"Remote default","is_default":true,"reasoning_efforts":["low"],"service_tiers":["standard"]}
            ]),
            failure: false,
            wrong_account: false,
            calls: vec![],
        }));
        let capture = host.clone();
        let server=Server::new(move |command| {
            let mut host=capture.lock().unwrap();host.calls.push(command.clone());
            match command {
                VesselCommand::Capabilities => Ok(json!({"vessel_id":host.id,"features":["provider_accounts"],"rights":["account_use"]})),
                VesselCommand::AccountModels {account,..} => {
                    assert_eq!(account,&host.account,"request must retain full reviewed account tuple");
                    if host.failure {return Err("[model_catalog:authentication] SYNTHETIC-NOT-FOR-DISPLAY".into());}
                    let mut actual=account.clone();if host.wrong_account {actual.connection_revision+=1;}
                    Ok(json!({"account":actual,"models":host.inventory,"account_label":"Scoped human account"}))
                }
                VesselCommand::Voyage(request) => match &request.command {
                    VoyageCommand::SetAccountInference {command_id,..} => Ok(voyage(command,json!({"command_id":command_id,"status":"applied","revision":18}))),
                    _ => panic!("unapproved synthetic model operation"),
                },
                _ => panic!("metadata journey cannot invoke another operation"),
            }
        }).await;
        server
            .client
            .request(VesselCommand::Capabilities)
            .await
            .unwrap();
        let target = server.target;
        let mut app = app_tests::app(fixture.0.path());
        app.clients = super::super::routes::Routes::new(vec![server.client.clone()]);
        let settings = Settings {
            account: Some(account),
            model: "current-custom".into(),
            provider: "openai-responses".into(),
            reasoning_effort: None,
            service_tier: None,
            ..Default::default()
        };
        let mut view=View::new(serde_json::from_value(json!({"session_id":target.session,"incarnation":server.incarnation,"workspace":fixture.0.path(),"state":"live","name":"Scoped models"})).unwrap());
        view.snapshot=Some(serde_json::from_value(json!({"session_id":target.session,"revision":17,"model":"current-custom","messages":[],"inference":settings,"run":null})).unwrap());
        view.draft.insert_str("Models keep unsent Δ");
        app.views.insert(target, view);
        app.selected = Some(target);
        let (sender, updates) = tokio::sync::mpsc::channel(32);
        app.sender = sender;
        Self {
            app,
            target,
            host,
            updates,
            _server: server,
            _fixture: fixture,
        }
    }
    async fn reply(&mut self) -> Update {
        tokio::time::timeout(std::time::Duration::from_secs(3), self.updates.recv())
            .await
            .unwrap()
            .unwrap()
    }
    fn install(&mut self, update: Update) {
        match update {
            Update::InferenceModels {
                route,
                id,
                context,
                generation,
                result,
            } => {
                assert_eq!(route, Some(self.target.route));
                self.app.inference_models(id, context, generation, result);
            }
            _ => panic!("must be model metadata callback"),
        }
    }
    fn count(&self) -> usize {
        self.host
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|c| matches!(c, VesselCommand::AccountModels { .. }))
            .count()
    }
    fn unchanged(&self) {
        let view = &self.app.views[&self.target];
        assert_eq!(view.draft.text, "Models keep unsent Δ");
        assert!(view.pending.is_none());
        assert!(view.snapshot.as_ref().unwrap().messages.is_empty());
        assert!(view.snapshot.as_ref().unwrap().run.is_none());
        assert_eq!(view.snapshot.as_ref().unwrap().model, "current-custom");
        assert!(!self.app.status.contains("SYNTHETIC-NOT-FOR-DISPLAY"));
        assert!(
            !self
                .host
                .lock()
                .unwrap()
                .calls
                .iter()
                .any(|c| matches!(c, VesselCommand::Voyage(_)))
        );
    }
    async fn finish(mut self) {
        self.app.cancel_model_catalog();
        self.app.cancel_model_preload();
        for job in self.app.retired_observers.drain(..) {
            job.abort();
            let _ = job.await;
        }
    }
}
fn key(peer: &mut Peer, code: KeyCode) {
    peer.app
        .input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}
fn paint(peer: &Peer, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| peer.app.draw_inference_picker(frame))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
async fn open(peer: &mut Peer, command: &str) {
    peer.app
        .inference_command(Destination::Live(peer.target), command, true)
        .unwrap();
    let update = peer.reply().await;
    peer.install(update);
    paint(peer, 110, 36);
}

#[tokio::test]
async fn current_model_inventory_installs_exact_account_label_and_capabilities_without_live_change()
{
    let mut peer = Peer::new().await;
    open(&mut peer, "/thinking").await;
    let picker = peer.app.inference.picker.as_ref().unwrap();
    assert!(picker.models_loaded);
    assert!(!picker.loading);
    assert_eq!(picker.options[0], "inherit");
    assert!(picker.options.contains(&"high".into()));
    let settings = peer
        .app
        .inference_settings(Destination::Live(peer.target))
        .unwrap();
    assert_eq!(
        peer.app
            .account_control_label(Destination::Live(peer.target), &settings),
        "Scoped human account"
    );
    assert_eq!(peer.count(), 1);
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn delayed_inventory_rejects_changed_account_incarnation_and_removed_destination_without_apply()
 {
    for change in 0..3 {
        let mut peer = Peer::new().await;
        peer.app
            .inference_command(Destination::Live(peer.target), "/thinking", true)
            .unwrap();
        let update = peer.reply().await;
        match change {
            0 => {
                peer.app
                    .views
                    .get_mut(&peer.target)
                    .unwrap()
                    .snapshot
                    .as_mut()
                    .unwrap()
                    .inference
                    .as_mut()
                    .unwrap()
                    .account
                    .as_mut()
                    .unwrap()
                    .identity_generation += 1
            }
            1 => {
                peer.app
                    .views
                    .get_mut(&peer.target)
                    .unwrap()
                    .process
                    .incarnation = Uuid::new_v4()
            }
            _ => {
                peer.app.views.remove(&peer.target);
            }
        }
        peer.install(update);
        let picker = peer.app.inference.picker.as_ref().unwrap();
        assert!(!picker.loading);
        assert!(!picker.models_loaded);
        assert!(picker.notice.contains("changed"));
        assert_eq!(peer.count(), 1);
        assert!(
            peer.host
                .lock()
                .unwrap()
                .calls
                .iter()
                .all(|c| !matches!(c, VesselCommand::Voyage(_)))
        );
        if change < 2 {
            peer.unchanged();
        }
        peer.finish().await;
    }
}

#[tokio::test]
async fn wrong_account_or_invalid_inventory_remains_unavailable_and_retains_custom_model() {
    for invalid_inventory in [false, true] {
        let mut peer = Peer::new().await;
        {
            let mut host = peer.host.lock().unwrap();
            host.wrong_account = !invalid_inventory;
            if invalid_inventory {
                host.inventory = json!([{"id":"duplicate"},{"id":"duplicate"}]);
            }
        }
        open(&mut peer, "/model").await;
        let picker = peer.app.inference.picker.as_ref().unwrap();
        assert!(!picker.models_loaded);
        assert!(!picker.loading);
        assert_eq!(picker.original.model, "current-custom");
        assert_eq!(picker.chooser.model, "current-custom");
        assert!(picker.options.contains(&"current-custom".into()));
        assert_eq!(peer.count(), 1);
        peer.unchanged();
        peer.finish().await;
    }
}

#[tokio::test]
async fn authentication_failure_is_sanitized_negative_warm_cache_and_retry_is_explicit_new_read_only()
 {
    let mut peer = Peer::new().await;
    peer.host.lock().unwrap().failure = true;
    open(&mut peer, "/model").await;
    assert!(
        peer.app
            .inference
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("sign-in")
    );
    assert!(!paint(&peer, 110, 36).contains("SYNTHETIC-NOT-FOR-DISPLAY"));
    key(&mut peer, KeyCode::Esc);
    for _ in 0..4 {
        peer.app.warm_selected_models();
        tokio::task::yield_now().await;
    }
    assert_eq!(peer.count(), 1);
    peer.app
        .inference_command(Destination::Live(peer.target), "/model", true)
        .unwrap();
    paint(&peer, 110, 36);
    assert_eq!(peer.count(), 1);
    peer.host.lock().unwrap().failure = false;
    for _ in 0..8 {
        if peer.app.inference.picker.as_ref().unwrap().chooser.focus == chooser::Control::Retry {
            break;
        }
        key(&mut peer, KeyCode::Tab);
    }
    assert!(peer.app.inference.picker.as_ref().unwrap().chooser.focus == chooser::Control::Retry);
    key(&mut peer, KeyCode::Enter);
    let update = peer.reply().await;
    peer.install(update);
    assert_eq!(peer.count(), 2);
    assert!(peer.app.inference.picker.as_ref().unwrap().models_loaded);
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn warmed_owned_request_hands_off_to_foreground_without_duplicate_model_transport() {
    let mut peer = Peer::new().await;
    peer.app.warm_selected_models();
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while peer.count() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    peer.app
        .inference_command(Destination::Live(peer.target), "/service", true)
        .unwrap();
    let update = peer.reply().await;
    peer.install(update);
    assert_eq!(peer.count(), 1);
    assert!(peer.app.inference.picker.as_ref().unwrap().models_loaded);
    assert!(
        peer.app
            .inference
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("cost")
    );
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn timeout_rotates_picker_and_late_real_response_cannot_reinstall_or_issue_a_mutation() {
    let mut peer = Peer::new().await;
    peer.app
        .inference_command(Destination::Live(peer.target), "/thinking", true)
        .unwrap();
    let update = peer.reply().await;
    let old = peer.app.inference.picker.as_ref().unwrap().id;
    peer.app.inference.catalog_job.as_mut().unwrap().1 =
        std::time::Instant::now() - std::time::Duration::from_secs(1);
    peer.app.poll_model_catalog();
    let fresh = peer.app.inference.picker.as_ref().unwrap().id;
    assert_ne!(old, fresh);
    peer.install(update);
    let picker = peer.app.inference.picker.as_ref().unwrap();
    assert_eq!(picker.id, fresh);
    assert!(!picker.models_loaded);
    assert!(!picker.loading);
    assert!(picker.notice.contains("too long"));
    assert_eq!(peer.count(), 1);
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn capability_only_sync_refreshes_options_but_external_requested_setting_change_remains_fenced()
 {
    let mut peer = Peer::new().await;
    open(&mut peer, "/service").await;
    {
        let latest = peer
            .app
            .views
            .get_mut(&peer.target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .inference
            .as_mut()
            .unwrap();
        latest.resolve(&[]);
        latest.service_tiers = vec!["new-tier".into()];
    }
    peer.app.sync_live_inference_picker(peer.target);
    let picker = peer.app.inference.picker.as_ref().unwrap();
    assert_eq!(picker.options, vec!["inherit", "new-tier"]);
    assert_eq!(picker.selected, 0);
    assert!(picker.notice.contains("refreshed"));
    let frozen = picker.original.clone();
    peer.app
        .views
        .get_mut(&peer.target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .inference
        .as_mut()
        .unwrap()
        .service_tier = Some("external-choice".into());
    peer.app.sync_live_inference_picker(peer.target);
    let picker = peer.app.inference.picker.as_ref().unwrap();
    assert!(picker.notice.contains("another view"));
    assert!(picker.original == frozen);
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn setting_mouse_cancel_and_keyboard_quit_never_apply_hidden_or_stale_hit_targets() {
    let mut peer = Peer::new().await;
    open(&mut peer, "/thinking").await;
    let cancel = peer.app.inference.cancel_hit.get().unwrap();
    peer.app
        .input(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: cancel.x,
            row: cancel.y,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
    assert!(peer.app.inference.picker.is_none());
    peer.unchanged();
    peer.app
        .inference_command(Destination::Live(peer.target), "/thinking", true)
        .unwrap();
    paint(&peer, 110, 36);
    peer.app.inference_input(&Event::Resize(20, 10)).unwrap();
    key(&mut peer, KeyCode::Enter);
    assert!(peer.app.inference.picker.is_some());
    assert!(peer.app.status.contains("not visible"));
    peer.app
        .input(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
    assert!(peer.app.quit);
    assert!(peer.app.inference.picker.is_none());
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn actual_footer_hit_is_retired_when_its_selected_destination_disappears() {
    let mut peer = Peer::new().await;
    let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
    terminal
        .draw(|frame| {
            peer.app
                .draw_inference_controls(frame, ratatui::layout::Rect::new(2, 2, 100, 4))
        })
        .unwrap();
    let (hit, destination) = peer.app.inference.options_hit.get().unwrap();
    assert!(destination == Destination::Live(peer.target));
    let event = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: hit.x,
        row: hit.y,
        modifiers: KeyModifiers::NONE,
    });
    peer.app.selected = None;
    assert!(peer.app.model_options_input(&event).unwrap());
    assert!(peer.app.inference.options_hit.get().is_none());
    assert!(peer.app.inference.picker.is_none());
    assert!(peer.app.inference.profiles.panel.is_none());
    assert_eq!(peer.count(), 0);
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn live_metadata_does_not_authorize_account_bound_apply_without_original_host_review() {
    let mut peer = Peer::new().await;
    open(&mut peer, "/thinking").await;
    key(&mut peer, KeyCode::Down);
    let error = peer
        .app
        .inference_input(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )))
        .unwrap_err();
    assert!(error.to_string().contains("Review /account"));
    assert_eq!(peer.count(), 1);
    peer.unchanged();
    peer.finish().await;
}

#[tokio::test]
async fn explicit_setting_selection_retains_one_revision_bound_command_and_never_submits_the_draft()
{
    let mut peer = Peer::new().await;
    open(&mut peer, "/thinking").await;
    peer.app
        .cache_account_host(peer.target.route, peer.host.lock().unwrap().id);
    key(&mut peer, KeyCode::Down);
    key(&mut peer, KeyCode::Enter);
    let pending = peer.app.views[&peer.target]
        .pending
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(pending.draft, "/thinking");
    assert_eq!(
        peer.app.views[&peer.target].draft.text,
        "Models keep unsent Δ"
    );
    assert!(pending.preserve_draft);
    let original = pending.original.as_ref().unwrap();
    let VoyageCommand::SetAccountInference {
        command_id,
        expected_revision,
        model,
        account,
        reasoning_effort,
        ..
    } = original.as_ref()
    else {
        panic!("must be an exact inference mutation")
    };
    assert_eq!(*command_id, pending.command_id);
    assert_eq!(*expected_revision, 17);
    assert_eq!(account, &peer.host.lock().unwrap().account);
    assert_eq!(model, "current-custom");
    assert!(reasoning_effort.is_some());
    let update = peer.reply().await;
    peer.app.update(update);
    {
        let host = peer.host.lock().unwrap();
        let requests: Vec<_> = host
            .calls
            .iter()
            .filter_map(|call| match call {
                VesselCommand::Voyage(request) => Some(request),
                _ => None,
            })
            .collect();
        assert_eq!(
            requests
                .iter()
                .filter(|r| matches!(r.command, VoyageCommand::SetAccountInference { .. }))
                .count(),
            1
        );
        let mutation = requests
            .iter()
            .find(|r| matches!(r.command, VoyageCommand::SetAccountInference { .. }))
            .unwrap();
        assert_eq!(mutation.session_id, peer.target.session);
        // This mutation is session/revision/account scoped, unlike a live-resource
        // action. The durable pending record retains its observed incarnation while
        // the authoritative command contract intentionally omits it on the wire.
        assert_eq!(
            pending.incarnation,
            peer.app.views[&peer.target].process.incarnation
        );
        assert!(!mutation.command.requires_incarnation());
        assert_eq!(mutation.incarnation, None);
        assert!(
            matches!(mutation.command,VoyageCommand::SetAccountInference {command_id,..} if command_id==pending.command_id)
        );
        assert_eq!(
            serde_json::to_value(&mutation.command).unwrap(),
            serde_json::to_value(original.as_ref()).unwrap()
        );
    }
    assert_eq!(
        peer.app.views[&peer.target].draft.text,
        "Models keep unsent Δ"
    );
    assert!(
        peer.app.views[&peer.target]
            .snapshot
            .as_ref()
            .unwrap()
            .messages
            .is_empty()
    );
    peer.finish().await;
}
