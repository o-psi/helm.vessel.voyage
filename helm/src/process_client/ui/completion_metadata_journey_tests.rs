//! Actual App input/render and held real WebSocket metadata replies. Synthetic
//! scoped peer only: no provider, executing Voyage, OS browser or human input.
use super::super::{
    account_test_support::Fixture, accounts::app_tests, routes::Routes, state::View,
};
use super::*;
use crate::process_client::loopback_tests::{Peer, response, send};
use crossterm::event::{Event, KeyEventKind};
use futures_util::{SinkExt, StreamExt};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::{Value, json};
use std::time::Duration;
use voyage_protocol::{
    accounts::{AccountBinding, Transport},
    duplex::ServerFrame,
    vessel::{VesselCommand, VoyageRequest},
};

const WAIT: Duration = Duration::from_secs(3);
struct Journey {
    peer: Peer,
    app: App,
    target: Target,
    incarnation: Uuid,
    updates: tokio::sync::mpsc::Receiver<Update>,
    fixture: Fixture,
}
impl Journey {
    async fn new() -> Self {
        let fixture = Fixture::new();
        let peer = Peer::open().await;
        let mut app = app_tests::app(fixture.0.path());
        app.clients = Routes::new(vec![peer.client.clone()]);
        let target = Target {
            route: super::super::state::Route::of(&peer.client),
            session: Uuid::new_v4(),
        };
        let incarnation = Uuid::new_v4();
        let mut view = View::new(serde_json::from_value(json!({"session_id":target.session,"incarnation":incarnation,"workspace":fixture.0.path(),"state":"live","name":"Completion fixture"})).unwrap());
        let account = AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 3,
            connection_revision: 4,
            transport: Transport::OpenaiResponses,
        };
        view.snapshot = Some(serde_json::from_value(json!({"session_id":target.session,"revision":17,"model":"current-fixture","total_messages":1,"messages":[{"message_index":0,"role":"user","content":"Canonical history Δ"}],"inference":{"account":account,"model":"current-fixture","provider":"openai-responses"}})).unwrap());
        app.views.insert(target, view);
        app.selected = Some(target);
        app.sidebar.focus = super::super::sidebar::Focus::Composer;
        let (sender, updates) = tokio::sync::mpsc::channel(32);
        app.sender = sender;
        Self {
            peer,
            app,
            target,
            incarnation,
            updates,
            fixture,
        }
    }
    fn text(&mut self, text: &str) {
        self.app
            .views
            .get_mut(&self.target)
            .unwrap()
            .draft
            .set_text(text.into());
    }
    fn key(&mut self, code: KeyCode) {
        self.app
            .input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap();
    }
    fn paint(&self) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 14)).unwrap();
        terminal
            .draw(|frame| self.app.draw_completion(frame, Rect::new(0, 0, 100, 14)))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    fn paint_accounts(&self) -> String {
        let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
        terminal
            .draw(|frame| self.app.draw_accounts(frame))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }
    async fn command(&mut self, section: &str, run: Option<Uuid>) -> Uuid {
        let (wire, command) = tokio::time::timeout(WAIT, self.peer.command())
            .await
            .unwrap();
        let VesselCommand::Voyage(VoyageRequest {
            session_id,
            incarnation,
            command:
                VoyageCommand::Controls {
                    run_id,
                    section: actual,
                },
        }) = command
        else {
            panic!("completion may emit only the exact read-only Controls request")
        };
        assert_eq!(session_id, self.target.session);
        assert_eq!(incarnation, Some(self.incarnation));
        assert_eq!(run_id, run);
        assert_eq!(actual, section);
        wire
    }
    async fn reply(&mut self, wire: Uuid, value: Value) {
        self.peer
            .voyage_reply(wire, self.target.session, self.incarnation, value)
            .await;
    }
    async fn update(&mut self) -> Update {
        let update = tokio::time::timeout(WAIT, self.updates.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(&update, Update::Completion { .. }));
        update
    }
    async fn install(&mut self) {
        let update = self.update().await;
        self.app.update(update);
    }
    async fn quiet(&mut self) {
        assert!(
            tokio::time::timeout(Duration::from_millis(80), async {
                loop {
                    match self.peer.socket.next().await {
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(bytes))) => self
                            .peer
                            .socket
                            .send(tokio_tungstenite::tungstenite::Message::Pong(bytes))
                            .await
                            .unwrap(),
                        Some(Ok(tokio_tungstenite::tungstenite::Message::Pong(_))) => {}
                        other => panic!("unexpected completion wire activity: {other:?}"),
                    }
                }
            })
            .await
            .is_err()
        );
    }
    fn preserved(&self, text: &str) {
        let view = &self.app.views[&self.target];
        let snapshot = view.snapshot.as_ref().unwrap();
        assert_eq!(view.draft.text, text);
        assert_eq!(view.draft.cursor, text.len());
        assert!(view.pending.is_none());
        assert_eq!(snapshot.revision, 17);
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(snapshot.messages[0].message_index, 0);
        assert_eq!(snapshot.messages[0].role, "user");
        assert_eq!(snapshot.messages[0].content, "Canonical history Δ");
        assert!(self.app.command_checks.is_empty());
        assert!(self.app.first_send_checks.is_empty());
        assert!(self.app.route_tasks.is_empty());
        assert!(self.app.browsers.is_empty());
        assert!(super::super::account_test_support::browsers().is_empty());
    }
    async fn finish(mut self) {
        self.peer.client.disconnect();
        tokio::time::timeout(WAIT, async {
            loop {
                match self.peer.socket.next().await {
                    None
                    | Some(Err(_))
                    | Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => break,
                    Some(Ok(
                        tokio_tungstenite::tungstenite::Message::Ping(_)
                        | tokio_tungstenite::tungstenite::Message::Pong(_),
                    )) => {}
                    _ => panic!("owned connection retirement cannot hide another command"),
                }
            }
        })
        .await
        .expect("owned metadata socket must terminate");
        assert!(self.fixture.0.path().exists());
        // This proves this owned transport's termination; it is not Voyage/browser cleanup.
    }
}

#[tokio::test]
async fn actual_model_tool_terminal_replies_render_and_tab_only_changes_unsent_composer() {
    for (command, section, value, chosen) in [
        (
            "model",
            "models",
            json!({"value":[{"id":"fixture-model","display_name":"Synthetic model"}]}),
            "/model fixture-model",
        ),
        (
            "tool",
            "tools",
            json!({"inventory":[{"name":"fixture_tool","description":"Synthetic tool"}]}),
            "/tool fixture_tool ",
        ),
        (
            "terminal",
            "terminals",
            json!([{"id":"fixture-terminal","title":"Synthetic terminal"}]),
            "/terminal fixture-terminal",
        ),
    ] {
        let mut j = Journey::new().await;
        let original = format!("/{command} ");
        j.text(&original);
        j.app.sync_completion();
        let wire = j.command(section, None).await;
        assert!(
            j.app
                .completion_menu()
                .unwrap()
                .hint
                .contains("Loading options")
        );
        j.app.sync_completion();
        j.app.sync_completion();
        j.quiet().await;
        j.reply(wire, value).await;
        j.install().await;
        j.preserved(&original);
        assert!(j.paint().contains(chosen));
        j.key(KeyCode::Tab);
        j.preserved(chosen);
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn held_model_tool_model_cycle_refuses_first_lookup_even_with_same_target_and_incarnation() {
    let mut j = Journey::new().await;
    j.text("/model ");
    j.app.sync_completion();
    let first = j.command("models", None).await;
    j.text("/tool ");
    j.app.sync_completion();
    let middle = j.command("tools", None).await;
    j.text("/model ");
    j.app.sync_completion();
    let last = j.command("models", None).await;
    j.reply(first, json!([{"id":"OLD-STALE-MODEL"}])).await;
    j.install().await;
    assert!(!j.paint().contains("OLD-STALE-MODEL"));
    assert!(
        j.app
            .completion_menu()
            .unwrap()
            .hint
            .contains("Loading options")
    );
    j.reply(middle, json!([{"name":"OLD-STALE-TOOL"}])).await;
    j.install().await;
    assert!(j.app.completion.metadata.as_ref().unwrap().value.is_none());
    j.reply(last, json!([{"id":"CURRENT-MODEL"}])).await;
    j.install().await;
    assert!(j.paint().contains("CURRENT-MODEL"));
    j.preserved("/model ");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn queued_reply_checks_full_account_and_current_inference_context_before_install_or_disclosure()
 {
    for change in 0..9 {
        let mut j = Journey::new().await;
        j.text("/model ");
        j.app.sync_completion();
        let old = j.command("models", None).await;
        j.reply(old, json!([{"id":"OLD-ACCOUNT-CATALOG"}])).await;
        let queued = j.update().await;
        let snapshot = j
            .app
            .views
            .get_mut(&j.target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap();
        let settings = snapshot.inference.as_mut().unwrap();
        let account = settings.account.as_mut().unwrap();
        match change {
            0 => account.account_id = Uuid::new_v4(),
            1 => account.connection_id = Uuid::new_v4(),
            2 => account.identity_generation += 1,
            3 => account.connection_revision += 1,
            4 => account.transport = Transport::OpenaiChat,
            5 => settings.model = "different-configured-model".into(),
            6 => settings.provider = "different-provider".into(),
            7 => {
                snapshot.inference_current = Some(super::super::inference::Settings {
                    model: "different-active-model".into(),
                    ..Default::default()
                })
            }
            _ => snapshot.model = "different-snapshot-model".into(),
        }
        j.app.update(queued);
        assert!(!j.paint().contains("OLD-ACCOUNT-CATALOG"));
        assert!(j.app.completion.metadata.as_ref().unwrap().value.is_none());
        j.app.sync_completion();
        let new = j.command("models", None).await;
        j.reply(new, json!([{"id":"NEW-ACCOUNT-CATALOG"}])).await;
        j.install().await;
        assert!(j.paint().contains("NEW-ACCOUNT-CATALOG"));
        j.preserved("/model ");
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn active_run_identity_and_finished_transition_are_exact_metadata_contexts() {
    for change in 0..2 {
        let mut j = Journey::new().await;
        let run = Uuid::new_v4();
        j.app
            .views
            .get_mut(&j.target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap()
            .run = Some(
            serde_json::from_value(
                json!({"run_id":run,"state":"running","partial_text":"Uncommitted Δ"}),
            )
            .unwrap(),
        );
        j.text("/terminal ");
        j.app.sync_completion();
        let old = j.command("terminals", Some(run)).await;
        let next = if change == 0 {
            Some(Uuid::new_v4())
        } else {
            None
        };
        let snapshot = j
            .app
            .views
            .get_mut(&j.target)
            .unwrap()
            .snapshot
            .as_mut()
            .unwrap();
        if let Some(next) = next {
            snapshot.run.as_mut().unwrap().run_id = next;
        } else {
            snapshot.run.as_mut().unwrap().state = "completed".into();
        }
        j.reply(old, json!([{"id":"OLD-RUN-TERMINAL"}])).await;
        j.install().await;
        assert!(!j.paint().contains("OLD-RUN-TERMINAL"));
        j.app.sync_completion();
        let new = j.command("terminals", next).await;
        j.reply(new, json!([{"id":"CURRENT-RUN-TERMINAL"}])).await;
        j.install().await;
        assert!(j.paint().contains("CURRENT-RUN-TERMINAL"));
        assert_eq!(
            j.app.views[&j.target]
                .snapshot
                .as_ref()
                .unwrap()
                .run
                .as_ref()
                .unwrap()
                .partial_text,
            "Uncommitted Δ"
        );
        j.preserved("/terminal ");
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn replaced_incarnation_and_selected_destination_cannot_receive_a_held_catalog() {
    for change in 0..3 {
        let mut j = Journey::new().await;
        j.text("/tool ");
        j.app.sync_completion();
        let old = j.command("tools", None).await;
        match change {
            0 => j.app.views.get_mut(&j.target).unwrap().process.incarnation = Uuid::new_v4(),
            1 => j.app.selected = None,
            _ => {
                let other = Target {
                    route: j.target.route,
                    session: Uuid::new_v4(),
                };
                let mut process = j.app.views[&j.target].process.clone();
                process.session_id = other.session;
                let view = View::new(process);
                j.app.views.insert(other, view);
                j.app.selected = Some(other);
            }
        }
        j.reply(old, json!([{"name":"STALE-DESTINATION-TOOL"}]))
            .await;
        j.install().await;
        assert!(!j.paint().contains("STALE-DESTINATION-TOOL"));
        assert!(j.app.completion.metadata.as_ref().unwrap().value.is_none());
        j.preserved("/tool ");
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn retired_route_generation_or_unavailable_route_never_discloses_old_metadata() {
    for unavailable in [false, true] {
        let mut j = Journey::new().await;
        j.text("/model ");
        j.app.sync_completion();
        let old = j.command("models", None).await;
        j.reply(old, json!([{"id":"OLD-ROUTE-CATALOG"}])).await;
        let queued = j.update().await;
        if unavailable {
            j.app.clients.mark_unavailable(j.target.route);
        } else {
            let new = j.app.clients.insert(j.peer.client.clone());
            assert_eq!(new.id, j.target.route.id);
            assert_ne!(new.generation, j.target.route.generation);
        }
        j.app.update(queued);
        j.app.sync_completion();
        assert!(!j.paint().contains("OLD-ROUTE-CATALOG"));
        assert!(j.app.completion.metadata.is_none());
        j.preserved("/model ");
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn completion_reply_during_existing_modal_is_not_installed_and_draft_does_not_change() {
    for help in [false, true] {
        let mut j = Journey::new().await;
        j.text("/model ");
        j.app.sync_completion();
        let held = j.command("models", None).await;
        if help {
            j.app.help = true;
        } else {
            j.app.sidebar.focus = super::super::sidebar::Focus::Voyages;
        }
        j.reply(held, json!([{"id":"HIDDEN-MODAL-CATALOG"}])).await;
        j.install().await;
        assert!(!j.paint().contains("HIDDEN-MODAL-CATALOG"));
        assert!(j.app.completion.metadata.as_ref().unwrap().value.is_none());
        j.app.sync_completion();
        assert!(j.app.completion.metadata.is_none());
        j.preserved("/model ");
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn unavailable_metadata_has_no_diagnostic_payload_or_implicit_retry_and_explicit_reload_is_one_read()
 {
    let mut j = Journey::new().await;
    j.text("/tool ");
    j.app.sync_completion();
    let wire = j.command("tools", None).await;
    let mut refused = response(Value::Null);
    refused.error = Some("SYNTHETIC-PRIVATE-DIAGNOSTIC".into());
    send(
        &mut j.peer.socket,
        ServerFrame::Reply {
            request_id: wire,
            response: refused,
        },
    )
    .await;
    j.install().await;
    assert!(j.paint().contains("Options unavailable"));
    assert!(!j.paint().contains("SYNTHETIC-PRIVATE-DIAGNOSTIC"));
    j.app.sync_completion();
    j.quiet().await;
    j.preserved("/tool ");
    j.key(KeyCode::Backspace);
    j.app.sync_completion();
    assert!(j.app.completion.metadata.is_none());
    j.key(KeyCode::Char(' '));
    j.app.sync_completion();
    let retry = j.command("tools", None).await;
    assert_ne!(retry, wire);
    j.reply(retry, json!([{"name":"explicit_retry"}])).await;
    j.install().await;
    assert!(j.paint().contains("explicit_retry"));
    j.preserved("/tool ");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn exact_public_receipt_envelope_refuses_foreign_owner_before_installing_inventory() {
    let mut j = Journey::new().await;
    j.text("/terminal ");
    j.app.sync_completion();
    let wire = j.command("terminals", None).await;
    j.peer
        .voyage_reply(
            wire,
            Uuid::new_v4(),
            j.incarnation,
            json!([{"id":"FOREIGN-TERMINAL"}]),
        )
        .await;
    j.install().await;
    assert!(j.paint().contains("Options unavailable"));
    assert!(!j.paint().contains("FOREIGN-TERMINAL"));
    j.preserved("/terminal ");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn actual_pending_read_deadline_keeps_options_unavailable_and_late_reply_never_replays() {
    let mut j = Journey::new().await;
    j.text("/tool ");
    j.app.sync_completion();
    let wire = j.command("tools", None).await;
    let update = tokio::time::timeout(Duration::from_secs(28), j.updates.recv())
        .await
        .unwrap()
        .unwrap();
    j.app.update(update);
    assert!(j.paint().contains("Options unavailable"));
    j.preserved("/tool ");
    j.app.sync_completion();
    j.quiet().await;
    j.reply(wire, json!([{"name":"LATE-TIMED-OUT-TOOL"}])).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(80), j.updates.recv())
            .await
            .is_err()
    );
    assert!(!j.paint().contains("LATE-TIMED-OUT-TOOL"));
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn metadata_inventory_is_bounded_and_control_tokens_never_enter_draft_or_canonical_history() {
    let mut j = Journey::new().await;
    j.text("/tool ");
    j.app.sync_completion();
    let wire = j.command("tools", None).await;
    let mut entries = vec![
        json!({"name":"bad name"}),
        json!({"name":"bad\u{1b}"}),
        json!({"name":"safe_fixture","description":"Visible\u{1b}\u{202e} description"}),
    ];
    entries.extend(
        (0..300).map(|i| json!({"name":format!("fixture_{i:03}"),"description":"Synthetic"})),
    );
    j.reply(wire, json!({"value":{"inventory":entries}})).await;
    j.install().await;
    let menu = j.app.completion_menu().unwrap();
    assert_eq!(menu.entries.len(), 254);
    assert!(
        menu.entries
            .iter()
            .all(|(text, description)| !text.contains('\u{1b}')
                && !description.contains('\u{1b}')
                && !description.contains('\u{202e}'))
    );
    assert!(
        menu.entries
            .iter()
            .all(|(text, _)| text != "/tool fixture_299 ")
    );
    j.key(KeyCode::Tab);
    j.preserved("/tool safe_fixture ");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn keyboard_release_modifiers_and_dismissal_are_local_and_do_not_submit_a_choice() {
    let mut j = Journey::new().await;
    j.text("/terminal ");
    j.app.sync_completion();
    let wire = j.command("terminals", None).await;
    j.reply(wire, json!([{"id":"first"},{"id":"second"}])).await;
    j.install().await;
    j.app
        .input(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Tab,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )))
        .unwrap();
    j.preserved("/terminal ");
    assert!(
        !j.app
            .completion_input(&KeyEvent::new(KeyCode::Tab, KeyModifiers::ALT))
            .unwrap()
    );
    j.preserved("/terminal ");
    j.key(KeyCode::Up);
    j.key(KeyCode::BackTab);
    j.key(KeyCode::Down);
    j.key(KeyCode::Tab);
    j.preserved("/terminal second");
    j.text("/terminal ");
    j.app.sync_completion();
    j.key(KeyCode::Esc);
    assert!(j.app.completion_menu().is_none());
    j.preserved("/terminal ");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn already_loaded_account_catalog_is_hidden_before_replacement_lookup_or_tab() {
    let mut j = Journey::new().await;
    j.text("/model ");
    j.app.sync_completion();
    let wire = j.command("models", None).await;
    j.reply(wire, json!([{"id":"ORIGINAL-ACCOUNT-CATALOG"}]))
        .await;
    j.install().await;
    assert!(j.paint().contains("ORIGINAL-ACCOUNT-CATALOG"));
    j.app
        .views
        .get_mut(&j.target)
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
        .connection_revision += 1;
    assert!(!j.paint().contains("ORIGINAL-ACCOUNT-CATALOG"));
    j.key(KeyCode::Tab);
    j.preserved("/model current-fixture");
    j.text("/model ");
    j.app.sync_completion();
    let fresh = j.command("models", None).await;
    j.reply(fresh, json!([{"id":"CURRENT-ACCOUNT-CATALOG"}]))
        .await;
    j.install().await;
    assert!(j.paint().contains("CURRENT-ACCOUNT-CATALOG"));
    j.preserved("/model ");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn prefix_edit_reuses_current_section_request_and_filters_against_current_unsent_text() {
    let mut j = Journey::new().await;
    j.text("/model ");
    j.app.sync_completion();
    let wire = j.command("models", None).await;
    for ch in "fixture".chars() {
        j.key(KeyCode::Char(ch));
        j.app.sync_completion();
    }
    j.quiet().await;
    j.reply(
        wire,
        json!([{"id":"fixture-good"},{"id":"other-account-model"}]),
    )
    .await;
    j.install().await;
    let menu = j.app.completion_menu().unwrap();
    assert_eq!(menu.entries.len(), 1);
    assert_eq!(menu.entries[0].0, "/model fixture-good");
    j.preserved("/model fixture");
    j.key(KeyCode::Tab);
    j.preserved("/model fixture-good");
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn real_private_account_modal_consumes_input_and_periodic_completion_sends_no_controls() {
    let mut j = Journey::new().await;
    j.text("/model ");
    j.app.sync_completion();
    let old = j.command("models", None).await;
    j.app
        .open_accounts(super::super::inference::Destination::Live(j.target), "")
        .unwrap();
    assert!(j.app.accounts.open());
    let (caps, command) = j.peer.command().await;
    assert!(matches!(command, VesselCommand::Capabilities));
    j.peer.reply(caps, json!({"features":[]})).await;
    tokio::time::timeout(WAIT, async {
        loop {
            j.app.account_tick();
            if j.paint_accounts().contains("account-aware upgrade") {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    j.key(KeyCode::Char('X'));
    j.app
        .input(Event::Paste("SYNTHETIC-PRIVATE-MODAL-PAYLOAD".into()))
        .unwrap();
    j.app.sync_completion();
    assert!(j.app.completion.metadata.is_none());
    assert_eq!(j.app.completion_height(), 0);
    j.reply(old, json!([{"id":"OLD-LIVE-MODAL-CATALOG"}])).await;
    j.install().await;
    assert!(!j.paint().contains("OLD-LIVE-MODAL-CATALOG"));
    j.preserved("/model ");
    j.quiet().await;
    j.key(KeyCode::Esc);
    assert!(!j.app.accounts.open());
    j.finish().await;
}

#[tokio::test]
async fn actual_inference_picker_keeps_live_completion_suppressed_and_never_applies_a_setting() {
    let mut j = Journey::new().await;
    j.text("/terminal ");
    j.app.sync_completion();
    let old = j.command("terminals", None).await;
    // The actual picker has its own read-only AccountModels request; this does
    // not permit periodic completion to issue the old composer's Controls.
    j.app
        .inference_command(
            super::super::inference::Destination::Live(j.target),
            "/thinking",
            true,
        )
        .unwrap();
    assert!(j.app.inference_picker_open());
    let (models, command) = j.peer.command().await;
    let VesselCommand::AccountModels { workspace, account } = command else {
        panic!("only the existing picker's account metadata read is permitted")
    };
    assert_eq!(workspace, j.fixture.0.path());
    assert!(
        Some(account.clone())
            == j.app.views[&j.target]
                .snapshot
                .as_ref()
                .unwrap()
                .inference
                .as_ref()
                .unwrap()
                .account
    );
    j.peer
        .reply(
            models,
            json!({"account":account,"models":[],"account_label":"Synthetic metadata account"}),
        )
        .await;
    let loaded = tokio::time::timeout(WAIT, j.updates.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(&loaded, Update::InferenceModels { .. }));
    j.app.update(loaded);
    j.app.sync_completion();
    assert!(j.app.completion.metadata.is_none());
    j.reply(old, json!([{"id":"OLD-INFERENCE-MODAL-TERMINAL"}]))
        .await;
    j.install().await;
    assert!(!j.paint().contains("OLD-INFERENCE-MODAL-TERMINAL"));
    j.key(KeyCode::Char('x'));
    j.preserved("/terminal ");
    j.quiet().await;
    j.key(KeyCode::Esc);
    assert!(!j.app.inference_picker_open());
    j.finish().await;
}

#[tokio::test]
async fn actual_unsent_configuration_draft_keeps_old_live_completion_and_creation_effects_absent() {
    let mut j = Journey::new().await;
    j.text("/model ");
    j.app.sync_completion();
    let old = j.command("models", None).await;
    let local = j
        .app
        .clients
        .insert(crate::process_client::transport::Client::local(
            j.fixture.0.path().join("no-vessel"),
        ));
    let workspace = j.fixture.0.path().to_str().unwrap().to_owned();
    j.app.create_on_route(local, Some(&workspace)).unwrap();
    let draft = j.app.active_draft.unwrap();
    assert!(j.app.selected.is_none());
    for ch in "/model private-draft-Δ".chars() {
        j.key(KeyCode::Char(ch));
    }
    j.app.sync_completion();
    assert!(j.app.completion.metadata.is_none());
    j.reply(old, json!([{"id":"OLD-LIVE-DRAFT-CATALOG"}])).await;
    j.install().await;
    assert!(!j.paint().contains("OLD-LIVE-DRAFT-CATALOG"));
    j.preserved("/model ");
    assert_eq!(
        j.app.new_drafts[&draft].navigation_title(),
        "/model private-draft-Δ"
    );
    assert!(j.app.new_drafts[&draft].saved.start.is_none());
    assert!(!j.app.new_drafts[&draft].busy);
    assert_eq!(j.app.new_drafts.len(), 1);
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn remote_local_paths_destructive_confirmations_and_ambiguous_session_ids_are_never_inserted()
{
    let mut j = Journey::new().await;
    let other_route = j
        .app
        .clients
        .insert(crate::process_client::transport::Client::local(
            j.fixture.0.path().join("unopened-local"),
        ));
    let other = Target {
        route: other_route,
        session: j.target.session,
    };
    let view = View::new(j.app.views[&j.target].process.clone());
    j.app.views.insert(other, view);
    for text in ["/new /", "/configure /", "/clear ", "/delete ", "/use "] {
        j.text(text);
        j.app.sync_completion();
        let menu = j.app.completion_menu().unwrap();
        assert!(menu.entries.is_empty());
        j.key(KeyCode::Tab);
        j.preserved(text);
        assert!(!j.paint().contains("SYNTHETIC-PRIVATE"));
    }
    assert!(
        j.app.clients[other_route]
            .connection_state()
            .borrow()
            .socket_id
            .is_none()
    );
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn old_section_inventory_is_never_reinterpreted_before_the_next_sync() {
    let mut j = Journey::new().await;
    j.text("/model ");
    j.app.sync_completion();
    let model = j.command("models", None).await;
    j.reply(
        model,
        json!([{"id":"MODEL-ID-NOT-A-TERMINAL","name":"MODEL-ID-NOT-A-TOOL"}]),
    )
    .await;
    j.install().await;
    for command in ["terminal", "tool"] {
        let text = format!("/{command} ");
        j.text(&text);
        assert!(!j.paint().contains("MODEL-ID-NOT"));
        j.key(KeyCode::Tab);
        j.preserved(&text);
    }
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn malformed_inventory_and_empty_candidates_cannot_change_the_unsent_draft() {
    for value in [
        Value::Null,
        json!({"value":{"inventory":{}}}),
        json!([{"id":"not-a-tool"},{"name":42},{"name":"two words"}]),
    ] {
        let mut j = Journey::new().await;
        j.text("/tool ");
        j.app.sync_completion();
        let wire = j.command("tools", None).await;
        j.reply(wire, value).await;
        j.install().await;
        assert!(j.app.completion_menu().unwrap().entries.is_empty());
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::BackTab, KeyCode::Tab] {
            j.key(code);
            j.preserved("/tool ");
        }
        assert_eq!(j.app.completion_height(), 3);
        j.quiet().await;
        j.finish().await;
    }
}

#[tokio::test]
async fn current_inference_choices_reject_unsafe_values_and_never_apply_them_on_tab() {
    let mut j = Journey::new().await;
    let settings = j
        .app
        .views
        .get_mut(&j.target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .inference
        .as_mut()
        .unwrap();
    settings.reasoning_efforts = vec![
        "inherit".into(),
        "fixture-effort".into(),
        "bad effort".into(),
        "unsafe\u{1b}".into(),
    ];
    settings.service_tiers = vec!["fixture-tier".into(), "bad tier".into()];
    for (text, expected) in [
        ("/thinking fi", "/thinking fixture-effort"),
        ("/service fi", "/service fixture-tier"),
        ("/compact 5", "/compact 50"),
    ] {
        j.text(text);
        j.app.sync_completion();
        assert_eq!(j.app.completion_menu().unwrap().entries[0].0, expected);
        j.key(KeyCode::Tab);
        j.preserved(expected);
    }
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .inference
            .as_ref()
            .unwrap()
            .reasoning_effort,
        None
    );
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .inference
            .as_ref()
            .unwrap()
            .service_tier,
        None
    );
    j.quiet().await;
    j.finish().await;
}

#[tokio::test]
async fn pending_decision_suggestions_are_current_incarnation_and_kind_without_decision_effects() {
    let mut j = Journey::new().await;
    let approval = Uuid::new_v4();
    let question = Uuid::new_v4();
    let foreign = Uuid::new_v4();
    let run = Uuid::new_v4();
    let expires = chrono::Utc::now().timestamp_millis().max(0) as u64 + 60000;
    j.app.views.get_mut(&j.target).unwrap().snapshot.as_mut().unwrap().decisions=serde_json::from_value(json!([
        {"decision_id":approval,"run_id":run,"incarnation":j.incarnation,"expires_at_ms":expires,"request":{"kind":"approval"}},
        {"decision_id":question,"run_id":run,"incarnation":j.incarnation,"expires_at_ms":expires,"request":{"kind":"question"}},
        {"decision_id":foreign,"run_id":run,"incarnation":Uuid::new_v4(),"expires_at_ms":expires,"request":{"kind":"approval"}}
    ])).unwrap();
    for (command, id, suffix) in [
        ("approve", approval, ""),
        ("deny", approval, ""),
        ("answer", question, " "),
    ] {
        let text = format!("/{command} ");
        j.text(&text);
        j.app.sync_completion();
        let entries = j.app.completion_menu().unwrap().entries;
        let expected = format!("/{command} {id}{suffix}");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, expected);
        assert!(!j.paint().contains(&foreign.to_string()));
        j.key(KeyCode::Tab);
        j.preserved(&expected);
    }
    assert_eq!(
        j.app.views[&j.target]
            .snapshot
            .as_ref()
            .unwrap()
            .decisions
            .len(),
        3
    );
    j.quiet().await;
    j.finish().await;
}
