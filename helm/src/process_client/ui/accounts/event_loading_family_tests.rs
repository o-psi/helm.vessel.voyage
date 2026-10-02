//! Ordinary App keyboard/private-view journeys through the maintained scripted
//! socket host. No provider, executing service, credentials or real browser.
use super::super::{
    account_socket_support_tests::{CODE, Peer},
    account_test_support,
};
use super::*;
use crossterm::event::{KeyEvent, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};

fn paint(peer: &Peer) -> String {
    let mut terminal = Terminal::new(TestBackend::new(110, 36)).unwrap();
    terminal
        .draw(|frame| peer.app.draw_accounts(frame))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}
fn input(peer: &mut Peer, code: KeyCode) {
    peer.app
        .input(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .unwrap();
}
async fn settle(peer: &mut Peer) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            peer.app.account_tick();
            if peer.app.accounts.reply.is_none()
                && peer.app.accounts.usage_reply.is_none()
                && peer.app.accounts.picker.as_ref().is_none_or(|p| !p.busy)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn open(peer: &mut Peer) {
    peer.app
        .open_accounts(Destination::Live(peer.target), "")
        .unwrap();
    settle(peer).await;
    paint(peer);
}
fn calls(peer: &Peer, predicate: impl Fn(&VesselCommand) -> bool) -> usize {
    peer.host
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|command| predicate(command))
        .count()
}
fn mutation_count(peer: &Peer) -> usize {
    calls(peer, |command| {
        matches!(
            command,
            VesselCommand::EnrollAccount { .. }
                | VesselCommand::CancelAccountEnrollment { .. }
                | VesselCommand::AccountSetDefault { .. }
                | VesselCommand::Voyage(voyage_protocol::vessel::VoyageRequest {
                    command: VoyageCommand::SetAccountInference { .. },
                    ..
                })
        )
    })
}
fn start_identity(peer: &Peer) -> (Uuid, Uuid, PathBuf, Uuid, String, String) {
    match peer.host.lock().unwrap().enrollment.as_ref().unwrap() {
        VesselCommand::EnrollAccount {
            command_id,
            enrollment_id,
            workspace,
            connection_id,
            alias,
            label,
        } => (
            *command_id,
            *enrollment_id,
            workspace.clone(),
            *connection_id,
            alias.clone(),
            label.clone(),
        ),
        _ => panic!("must retain exact sign-in request"),
    }
}
async fn sign_in(peer: &mut Peer) {
    input(peer, KeyCode::Down);
    input(peer, KeyCode::Enter);
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::Alias(_)
    ));
    for ch in "human_account".chars() {
        input(peer, KeyCode::Char(ch));
    }
    input(peer, KeyCode::Enter);
    settle(peer).await;
    paint(peer);
    assert_eq!(
        calls(peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
}

#[tokio::test]
async fn search_paste_resize_and_release_stay_inside_private_modal_and_do_not_select() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    input(&mut peer, KeyCode::Char('L'));
    input(&mut peer, KeyCode::Char('o'));
    assert_eq!(peer.app.accounts.picker.as_ref().unwrap().query, "Lo");
    peer.app.input(Event::Paste(CODE.into())).unwrap();
    peer.app
        .input(Event::Key(KeyEvent::new_with_kind(
            KeyCode::Enter,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )))
        .unwrap();
    assert_eq!(peer.app.accounts.picker.as_ref().unwrap().query, "Lo");
    assert_eq!(mutation_count(&peer), 0);
    peer.app.input(Event::Resize(20, 10)).unwrap();
    input(&mut peer, KeyCode::Enter);
    assert!(peer.app.accounts.open());
    assert!(!peer.app.accounts.visible.get());
    paint(&peer);
    input(&mut peer, KeyCode::Backspace);
    assert_eq!(peer.app.accounts.picker.as_ref().unwrap().query, "L");
    input(&mut peer, KeyCode::Esc);
    assert!(!peer.app.accounts.open());
    assert!(peer.app.accounts.hits.borrow().is_empty() || !peer.app.accounts.visible.get());
    peer.unchanged();
    assert_eq!(mutation_count(&peer), 0);
}

#[tokio::test]
async fn device_connection_keyboard_filter_rejects_bad_alias_before_signin_then_escape_returns_list()
 {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    input(&mut peer, KeyCode::Down);
    input(&mut peer, KeyCode::Enter);
    for ch in "invalid alias".chars() {
        input(&mut peer, KeyCode::Char(ch));
    }
    input(&mut peer, KeyCode::Enter);
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::Alias(_)
    ));
    assert!(
        peer.app
            .accounts
            .picker
            .as_ref()
            .unwrap()
            .notice
            .contains("alias")
    );
    assert_eq!(mutation_count(&peer), 0);
    input(&mut peer, KeyCode::Esc);
    let picker = peer.app.accounts.picker.as_ref().unwrap();
    assert!(matches!(picker.mode, Mode::List));
    assert!(picker.query.is_empty());
    assert!(picker.notice.contains("No change"));
    input(&mut peer, KeyCode::Down);
    input(&mut peer, KeyCode::Enter);
    for _ in 0..130 {
        input(&mut peer, KeyCode::Char('x'));
    }
    assert_eq!(peer.app.accounts.picker.as_ref().unwrap().query.len(), 128);
    input(&mut peer, KeyCode::Enter);
    assert_eq!(mutation_count(&peer), 0);
    peer.unchanged();
}

#[tokio::test]
async fn multiple_native_connections_mouse_and_keyboard_choose_only_the_reviewed_alias_target() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let other = Uuid::new_v4();
    peer.app
        .accounts
        .picker
        .as_mut()
        .unwrap()
        .catalogue
        .connections
        .push(ConnectionDescriptor {
            id: other,
            revision: 9,
            label: "Second permitted device connection".into(),
            endpoint: "https://chatgpt.com/backend-api/codex".into(),
            transports: vec![Transport::ChatgptOauth],
        });
    input(&mut peer, KeyCode::Down);
    input(&mut peer, KeyCode::Enter);
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::Connections
    ));
    paint(&peer);
    input(&mut peer, KeyCode::Down);
    input(&mut peer, KeyCode::Up);
    assert_eq!(peer.app.accounts.picker.as_ref().unwrap().selected, 0);
    input(&mut peer, KeyCode::Down);
    paint(&peer);
    let hit = peer
        .app
        .accounts
        .hits
        .borrow()
        .iter()
        .find(|(_, index)| *index == 1)
        .map(|(r, _)| *r)
        .unwrap();
    peer.app
        .input(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: hit.x,
            row: hit.y,
            modifiers: KeyModifiers::NONE,
        }))
        .unwrap();
    assert!(matches!(peer.app.accounts.picker.as_ref().unwrap().mode,Mode::Alias(id) if id==other));
    assert!(peer.app.accounts.picker.as_ref().unwrap().query.is_empty());
    assert_eq!(mutation_count(&peer), 0);
    peer.unchanged();
}

#[tokio::test]
async fn default_consent_escape_and_denied_enter_keep_frozen_current_account_and_send_nothing() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let original = peer.app.accounts.picker.as_ref().unwrap().original.clone();
    input(&mut peer, KeyCode::F(6));
    assert!(
        matches!(&peer.app.accounts.picker.as_ref().unwrap().mode,Mode::DefaultConsent(settings) if settings.account==original.account)
    );
    input(&mut peer, KeyCode::Esc);
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::List
    ));
    peer.app
        .accounts
        .picker
        .as_mut()
        .unwrap()
        .catalogue
        .can_set_default = false;
    paint(&peer);
    input(&mut peer, KeyCode::F(6));
    input(&mut peer, KeyCode::Enter);
    assert!(!peer.app.accounts.picker.as_ref().unwrap().notice.is_empty());
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::DefaultConsent(_)
    ));
    assert_eq!(mutation_count(&peer), 0);
    assert!(peer.host.lock().unwrap().default.is_none());
    peer.unchanged();
}

#[tokio::test]
async fn actual_signin_keys_poll_same_intent_open_only_explicitly_and_escape_never_cancels() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    sign_in(&mut peer).await;
    let exact = start_identity(&peer);
    assert!(paint(&peer).contains(CODE));
    assert!(account_test_support::browsers().is_empty());
    peer.app.input(Event::Paste(CODE.into())).unwrap();
    input(&mut peer, KeyCode::Char('r'));
    settle(&mut peer).await;
    paint(&peer);
    assert_eq!(start_identity(&peer), exact);
    input(&mut peer, KeyCode::Char('o'));
    assert_eq!(
        account_test_support::browsers(),
        vec!["https://auth.openai.com/codex/device"]
    );
    input(&mut peer, KeyCode::Esc);
    assert!(peer.app.accounts.picker.is_none());
    assert!(peer.app.accounts.reply.is_none());
    assert!(peer.app.accounts.usage_reply.is_none());
    assert_eq!(
        calls(&peer, |c| matches!(
            c,
            VesselCommand::CancelAccountEnrollment { .. }
        )),
        0
    );
    peer.unchanged();
    assert!(
        !peer.app.views[&peer.target]
            .history
            .draft
            .text
            .contains(CODE)
    );
}

#[tokio::test]
async fn explicit_cancel_is_one_frozen_request_and_quit_scrubs_private_material_without_replay() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    sign_in(&mut peer).await;
    let exact = start_identity(&peer);
    input(&mut peer, KeyCode::Char('c'));
    settle(&mut peer).await;
    paint(&peer);
    assert_eq!(
        peer.host.lock().unwrap().enrollment_state,
        EnrollmentState::Cancelled
    );
    assert_eq!(start_identity(&peer), exact);
    assert_eq!(
        calls(&peer, |c| matches!(
            c,
            VesselCommand::CancelAccountEnrollment { .. }
        )),
        1
    );
    assert!(peer.app.accounts.picker.as_ref().unwrap().private.is_none());
    peer.app
        .input(Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
    assert!(peer.app.quit);
    assert!(peer.app.accounts.picker.as_ref().unwrap().private.is_none());
    assert_eq!(
        calls(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
    peer.unchanged();
}

#[tokio::test]
async fn restart_key_cancels_original_then_requires_new_alias_confirmation_without_second_signin() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    sign_in(&mut peer).await;
    let exact = start_identity(&peer);
    input(&mut peer, KeyCode::Char('n'));
    settle(&mut peer).await;
    let picker = peer.app.accounts.picker.as_ref().unwrap();
    assert!(matches!(picker.mode,Mode::Alias(id) if id==exact.3));
    assert_eq!(picker.query, exact.4);
    assert!(picker.private.is_none());
    assert!(picker.intent.is_none());
    assert_eq!(picker.outcome, Some(EnrollmentState::Cancelled));
    assert_eq!(
        calls(&peer, |c| matches!(
            c,
            VesselCommand::CancelAccountEnrollment { .. }
        )),
        1
    );
    assert_eq!(
        calls(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
    input(&mut peer, KeyCode::Esc);
    peer.unchanged();
}

#[tokio::test]
async fn socket_generation_loss_scrubs_code_and_ctrl_g_preserves_the_exact_enrollment_without_effect()
 {
    use crate::process_client::duplex::ConnectionState;
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    sign_in(&mut peer).await;
    let exact = start_identity(&peer);
    let picker = peer.app.accounts.picker.as_mut().unwrap();
    let old = *picker.connection.borrow();
    let (sender, receiver) = tokio::sync::watch::channel(old);
    picker.connection = receiver;
    sender
        .send(ConnectionState {
            socket_id: None,
            loss_generation: old.loss_generation + 1,
        })
        .unwrap();
    input(&mut peer, KeyCode::Char('o'));
    assert!(peer.app.accounts.picker.as_ref().unwrap().private.is_none());
    assert!(peer.app.accounts.picker.as_ref().unwrap().disconnected);
    assert!(account_test_support::browsers().is_empty());
    assert_eq!(start_identity(&peer), exact);
    peer.app
        .input(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::CONTROL,
        )))
        .unwrap();
    assert!(peer.app.accounts.picker.is_none());
    assert!(peer.app.accounts.reply.is_none());
    assert_eq!(
        calls(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
    assert_eq!(
        calls(&peer, |c| matches!(
            c,
            VesselCommand::CancelAccountEnrollment { .. }
        )),
        0
    );
    peer.unchanged();
}

#[tokio::test]
async fn api_setup_enter_refreshes_metadata_without_importing_or_exposing_any_api_key() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    input(&mut peer, KeyCode::Down);
    input(&mut peer, KeyCode::Down);
    input(&mut peer, KeyCode::Enter);
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::ApiSetup
    ));
    let before = calls(&peer, |c| matches!(c, VesselCommand::Accounts { .. }));
    input(&mut peer, KeyCode::Enter);
    settle(&mut peer).await;
    paint(&peer);
    assert!(matches!(
        peer.app.accounts.picker.as_ref().unwrap().mode,
        Mode::List
    ));
    assert_eq!(
        calls(&peer, |c| matches!(c, VesselCommand::Accounts { .. })),
        before + 1
    );
    assert_eq!(mutation_count(&peer), 0);
    peer.unchanged();
}

#[tokio::test]
async fn private_account_setup_does_not_cancel_or_reconfigure_an_already_running_voyage() {
    let mut peer = Peer::new().await;
    let run = Uuid::new_v4();
    peer.app
        .views
        .get_mut(&peer.target)
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .run =
        Some(serde_json::from_value(serde_json::json!({"run_id":run,"state":"running"})).unwrap());
    let before = peer.app.views[&peer.target]
        .snapshot
        .as_ref()
        .unwrap()
        .run
        .clone();
    open(&mut peer).await;
    sign_in(&mut peer).await;
    input(&mut peer, KeyCode::Esc);
    let after = &peer.app.views[&peer.target].snapshot.as_ref().unwrap().run;
    assert!(
        *after == before,
        "account setup must retain every active run field"
    );
    assert_eq!(calls(&peer, |c| matches!(c, VesselCommand::Voyage(_))), 0);
    assert_eq!(
        calls(&peer, |c| matches!(
            c,
            VesselCommand::CancelAccountEnrollment { .. }
        )),
        0
    );
    peer.unchanged();
}
