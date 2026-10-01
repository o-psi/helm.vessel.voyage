//! Profile mutation/observation journeys over deterministic public metadata.
use super::*;
use crate::process_client::ui::account_socket_support_tests::Peer;
use crossterm::event::KeyEvent;
use std::time::Duration;

async fn finish(peer: &mut Peer) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            peer.app.poll_profiles();
            if peer.app.inference.profiles.job.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
async fn open(peer: &mut Peer) {
    peer.app.open_profiles().unwrap();
    finish(peer).await;
}
fn key(peer: &mut Peer, code: KeyCode) {
    assert!(
        peer.app
            .profiles_input(&Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
            .unwrap()
    );
}
fn profile_changes(peer: &Peer) -> Vec<VesselCommand> {
    peer.host
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|c| {
            matches!(
                c,
                VesselCommand::SaveProfile { .. }
                    | VesselCommand::DeleteProfile { .. }
                    | VesselCommand::SetDefaultProfile { .. }
            )
        })
        .cloned()
        .collect()
}

#[tokio::test]
async fn profile_host_load_preserves_live_settings_and_caches_only_authenticated_metadata() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let host = peer.host.lock().unwrap();
    let panel = peer.app.inference.profiles.panel.as_ref().unwrap();
    assert_eq!(panel.catalogue.as_ref().unwrap().revision, 9);
    assert_eq!(panel.catalogue.as_ref().unwrap().profiles[0], host.profile);
    assert_eq!(peer.app.account_host(peer.target.route), Some(host.host));
    drop(host);
    assert!(profile_changes(&peer).is_empty());
    peer.unchanged();
}

#[tokio::test]
async fn host_profile_default_and_delete_mutations_have_exact_cas_and_no_live_reconfiguration() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let id = peer.host.lock().unwrap().profile.id;
    key(&mut peer, KeyCode::Char('f'));
    finish(&mut peer).await;
    assert_eq!(peer.host.lock().unwrap().profile_default, Some(id));
    assert_eq!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .catalogue
            .as_ref()
            .unwrap()
            .revision,
        10
    );
    peer.unchanged();
    key(&mut peer, KeyCode::Char('x'));
    assert!(profile_changes(&peer).len() == 1);
    key(&mut peer, KeyCode::Char('y'));
    finish(&mut peer).await;
    let changes = profile_changes(&peer);
    assert_eq!(changes.len(), 2);
    let VesselCommand::SetDefaultProfile {
        command_id: first,
        expected_revision: 9,
        profile_id: first_id,
        ..
    } = &changes[0]
    else {
        panic!("wrong default mutation");
    };
    let VesselCommand::DeleteProfile {
        command_id: second,
        expected_revision: 10,
        profile_id: second_id,
        ..
    } = &changes[1]
    else {
        panic!("wrong delete mutation");
    };
    assert!(!first.is_nil());
    assert!(!second.is_nil());
    assert_ne!(first, second);
    assert_eq!(first_id, second_id);
    assert_eq!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .catalogue
            .as_ref()
            .unwrap()
            .profiles
            .len(),
        0
    );
    peer.unchanged();
}

#[tokio::test]
async fn first_profile_save_explicitly_sets_default_and_waits_for_the_returned_catalogue() {
    let mut peer = Peer::new().await;
    {
        let mut host = peer.host.lock().unwrap();
        host.profiles.clear();
        host.profile_default = None;
    }
    open(&mut peer).await;
    let id = Uuid::new_v4();
    peer.app.inference.profiles.editing = Some((id, "Reviewed new profile".into()));
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    assert!(peer.app.inference.profiles.pending_save.is_some());
    assert!(peer.app.inference.profiles.editing.is_none());
    finish(&mut peer).await;
    let changes = profile_changes(&peer);
    assert_eq!(changes.len(), 1);
    let VesselCommand::SaveProfile {
        profile: written,
        make_default,
        expected_revision,
        ..
    } = &changes[0]
    else {
        panic!("wrong save");
    };
    assert_eq!(written.id, id);
    assert_eq!(written.name, "Reviewed new profile");
    assert!(*make_default);
    assert_eq!(*expected_revision, 9);
    assert_eq!(peer.host.lock().unwrap().profile_default, Some(id));
    assert!(peer.app.inference.profiles.pending_save.is_none());
    peer.unchanged();
}

#[tokio::test]
async fn editing_existing_profile_never_implicitly_changes_the_host_default_or_live_model() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Renamed preference".into()));
    let mut value = settings(&profile);
    value.model = "saved-preference-model".into();
    peer.app.save_profile_settings(value).unwrap();
    finish(&mut peer).await;
    let changes = profile_changes(&peer);
    let VesselCommand::SaveProfile {
        make_default,
        profile,
        ..
    } = &changes[0]
    else {
        panic!("wrong save");
    };
    assert!(!make_default);
    assert_eq!(profile.model, "saved-preference-model");
    assert!(peer.host.lock().unwrap().profile_default.is_none());
    peer.unchanged();
}

#[tokio::test]
async fn withdrawn_management_or_malformed_save_ack_retains_review_and_requires_explicit_reload() {
    for observation in [false, true] {
        let mut peer = Peer::new().await;
        open(&mut peer).await;
        let profile = peer.host.lock().unwrap().profile.clone();
        peer.app.inference.profiles.editing = Some((profile.id, "Retained review".into()));
        {
            let mut host = peer.host.lock().unwrap();
            host.refuse_profile_change = !observation;
            host.malformed_profile_ack = observation;
        }
        peer.app.save_profile_settings(settings(&profile)).unwrap();
        finish(&mut peer).await;
        let panel = peer.app.inference.profiles.panel.as_ref().unwrap();
        let (_, name, seed) = panel.name.as_ref().unwrap();
        assert_eq!(name, "Retained review");
        assert_eq!(seed.as_ref().unwrap().account, profile.account);
        assert!(panel.notice.contains("reload") || panel.notice.contains("reloads"));
        assert_eq!(profile_changes(&peer).len(), 1);
        assert!(peer.app.inference.profiles.job.is_none());
        peer.unchanged();
        tokio::task::yield_now().await;
        peer.app.poll_profiles();
        assert_eq!(profile_changes(&peer).len(), 1);
    }
}

#[tokio::test]
async fn unavailable_optional_account_names_do_not_turn_successful_profile_save_into_failure() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.host.lock().unwrap().refuse_accounts = true;
    peer.app.inference.profiles.editing = Some((profile.id, "Save without account labels".into()));
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    finish(&mut peer).await;
    assert!(peer.app.inference.profiles.pending_save.is_none());
    assert!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    assert_eq!(
        peer.app.inference.profiles.labels[0].name,
        "Save without account labels"
    );
    assert!(peer.app.inference.profiles.accounts.is_empty());
    peer.unchanged();
}

#[tokio::test]
async fn read_only_or_missing_account_profile_save_refuses_before_host_effect() {
    let mut peer = Peer::new().await;
    peer.host.lock().unwrap().can_manage = false;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Forbidden".into()));
    assert!(peer.app.save_profile_settings(settings(&profile)).is_err());
    assert!(profile_changes(&peer).is_empty());
    peer.unchanged();
    peer.app
        .inference
        .profiles
        .panel
        .as_mut()
        .unwrap()
        .catalogue
        .as_mut()
        .unwrap()
        .can_manage = true;
    let mut value = settings(&profile);
    value.account = None;
    assert!(peer.app.save_profile_settings(value).is_err());
    assert!(profile_changes(&peer).is_empty());
    peer.unchanged();
}

#[tokio::test]
async fn closing_pending_profile_view_does_not_cancel_or_reissue_the_host_mutation() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Closed local review".into()));
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    peer.app.inference.profiles.panel = None;
    finish(&mut peer).await;
    assert_eq!(profile_changes(&peer).len(), 1);
    assert!(peer.app.inference.profiles.panel.is_none());
    peer.unchanged();
}

#[tokio::test]
async fn selecting_a_profile_stages_one_exact_account_change_and_keeps_the_unsent_prompt() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    key(&mut peer, KeyCode::Enter);
    let pending = peer.app.views[&peer.target].pending.as_ref().unwrap();
    let Some(command) = pending.original.as_ref() else {
        panic!("missing reviewed settings command");
    };
    let voyage_protocol::vessel::VoyageCommand::SetAccountInference {
        account,
        model,
        reasoning_effort,
        ..
    } = command.as_ref()
    else {
        panic!("wrong profile application");
    };
    assert_eq!(account, &profile.account);
    assert_eq!(model, &profile.model);
    assert_eq!(reasoning_effort, &profile.reasoning_effort);
    assert_eq!(
        peer.app.views[&peer.target].draft.text,
        "Unsent local prompt Δ"
    );
    assert!(profile_changes(&peer).is_empty());
    assert!(peer.app.inference.profiles.panel.is_none());
}

#[tokio::test]
async fn late_failed_save_is_retained_under_original_origin_not_restored_after_destination_change()
{
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    peer.host.lock().unwrap().refuse_profile_change = true;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Original workspace review".into()));
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    let original = peer
        .app
        .inference
        .profiles
        .pending_save
        .as_ref()
        .unwrap()
        .clone();
    peer.app.selected = None;
    finish(&mut peer).await;
    assert!(peer.app.inference.profiles.panel.is_none());
    assert!(peer.app.inference.profiles.pending_save.is_none());
    let retained = &peer.app.inference.profiles.unconfirmed_saves[0];
    assert_eq!(retained.command_id, original.command_id);
    assert_eq!(retained.profile.name, "Original workspace review");
    assert!(retained.origin.destination == Destination::Live(peer.target));
    peer.app.selected = Some(peer.target);
    peer.host.lock().unwrap().refuse_profiles = true;
    peer.app.open_profiles().unwrap();
    finish(&mut peer).await;
    assert!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    assert_eq!(profile_changes(&peer).len(), 1);
    peer.unchanged();
}

#[tokio::test]
async fn closed_failed_save_retains_exact_unknown_command_without_injecting_later_load_failure() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    peer.host.lock().unwrap().refuse_profile_change = true;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Closed original review".into()));
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    let id = peer
        .app
        .inference
        .profiles
        .pending_save
        .as_ref()
        .unwrap()
        .command_id;
    peer.app.inference.profiles.panel = None;
    finish(&mut peer).await;
    assert_eq!(
        peer.app.inference.profiles.unconfirmed_saves[0].command_id,
        id
    );
    peer.host.lock().unwrap().refuse_profiles = true;
    peer.app.open_profiles().unwrap();
    finish(&mut peer).await;
    assert!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    assert_eq!(profile_changes(&peer).len(), 1);
    peer.unchanged();
}

#[tokio::test]
async fn failed_save_cannot_restore_review_after_workspace_host_or_incarnation_changes() {
    for field in 0..3 {
        let mut peer = Peer::new().await;
        open(&mut peer).await;
        peer.host.lock().unwrap().refuse_profile_change = true;
        let profile = peer.host.lock().unwrap().profile.clone();
        peer.app.inference.profiles.editing = Some((profile.id, "Fenced review".into()));
        peer.app.save_profile_settings(settings(&profile)).unwrap();
        let id = peer
            .app
            .inference
            .profiles
            .pending_save
            .as_ref()
            .unwrap()
            .command_id;
        match field {
            0 => {
                peer.app
                    .views
                    .get_mut(&peer.target)
                    .unwrap()
                    .process
                    .workspace = peer.fixture.0.path().join("different")
            }
            1 => peer
                .app
                .cache_account_host(peer.target.route, Uuid::new_v4()),
            _ => {
                peer.app
                    .views
                    .get_mut(&peer.target)
                    .unwrap()
                    .process
                    .incarnation = Uuid::new_v4()
            }
        };
        finish(&mut peer).await;
        assert!(
            peer.app
                .inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .name
                .is_none()
        );
        assert_eq!(
            peer.app.inference.profiles.unconfirmed_saves[0].command_id,
            id
        );
        assert_eq!(profile_changes(&peer).len(), 1);
        peer.unchanged();
    }
}

#[tokio::test]
async fn deactivated_profile_route_retains_unknown_save_and_cannot_restore_or_replay_it() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    peer.host.lock().unwrap().refuse_profile_change = true;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Disconnected review".into()));
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    let id = peer
        .app
        .inference
        .profiles
        .pending_save
        .as_ref()
        .unwrap()
        .command_id;
    peer.app.clients.mark_unavailable(peer.target.route);
    finish(&mut peer).await;
    assert!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    assert_eq!(
        peer.app.inference.profiles.unconfirmed_saves[0].command_id,
        id
    );
    assert!(peer.app.inference.profiles.job.is_none());
    peer.unchanged();
}

#[tokio::test]
async fn profile_host_pin_refuses_changed_capabilities_before_a_reviewed_save_effect() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Pinned host review".into()));
    let original_host = peer.host.lock().unwrap().host;
    peer.host.lock().unwrap().host = Uuid::new_v4();
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    finish(&mut peer).await;
    assert!(profile_changes(&peer).is_empty());
    let retained = &peer.app.inference.profiles.unconfirmed_saves[0];
    assert_eq!(retained.origin.host, original_host);
    assert_eq!(retained.profile.name, "Pinned host review");
    peer.unchanged();
}

#[tokio::test]
async fn socket_generation_change_retains_save_id_without_restoring_review_or_resending() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Lost socket review".into()));
    peer.host.lock().unwrap().refuse_profile_change = true;
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    let original = peer
        .app
        .inference
        .profiles
        .pending_save
        .as_ref()
        .unwrap()
        .clone();
    peer.app.clients[peer.target.route].disconnect();
    finish(&mut peer).await;
    assert!(
        peer.app.clients[peer.target.route]
            .connection_state()
            .borrow()
            .loss_generation
            > original.origin.loss_generation
    );
    assert!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    assert_eq!(
        peer.app.inference.profiles.unconfirmed_saves[0].command_id,
        original.command_id
    );
    assert!(profile_changes(&peer).len() <= 1);
    peer.unchanged();
}

#[tokio::test]
async fn acknowledgement_from_wrong_save_id_cannot_consume_a_retained_original_review() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    let origin = peer
        .app
        .profile_save_origin(Destination::Live(peer.target))
        .unwrap();
    let original = Uuid::new_v4();
    peer.app.inference.profiles.pending_save = Some(SaveReview {
        origin,
        command_id: original,
        profile,
    });
    peer.app.inference.profiles.job_save_id = Some(Uuid::new_v4());
    let (sender, receiver) = tokio::sync::oneshot::channel();
    peer.app.inference.profiles.job = Some(receiver);
    assert!(
        sender
            .send(Err(anyhow::anyhow!("unrelated save response")))
            .is_ok()
    );
    peer.app.poll_profiles();
    assert_eq!(
        peer.app
            .inference
            .profiles
            .pending_save
            .as_ref()
            .unwrap()
            .command_id,
        original
    );
    assert!(peer.app.inference.profiles.unconfirmed_saves.is_empty());
    assert!(
        peer.app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .name
            .is_none()
    );
    peer.unchanged();
}

async fn queued_success(peer: &mut Peer) -> ProfileJobResult {
    // Wait for the real scripted transport response without publishing it to
    // the UI; a new one-shot lets the test control the later consumption point.
    let mut receiver = peer.app.inference.profiles.job.take().unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match receiver.try_recv() {
                Ok(value) => break value,
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    tokio::task::yield_now().await
                }
                Err(_) => panic!("scripted save reply must remain available"),
            }
        }
    })
    .await
    .unwrap();
    assert!(result.is_ok());
    result
}

#[tokio::test]
async fn delayed_success_settles_original_save_without_publishing_into_changed_context() {
    for field in 0..5 {
        let mut peer = Peer::new().await;
        open(&mut peer).await;
        let profile = peer.host.lock().unwrap().profile.clone();
        peer.app.inference.profiles.editing =
            Some((profile.id, "Acknowledged original review".into()));
        peer.app.save_profile_settings(settings(&profile)).unwrap();
        let original = peer
            .app
            .inference
            .profiles
            .pending_save
            .as_ref()
            .unwrap()
            .clone();
        let response = queued_success(&mut peer).await;
        assert!(
            response
                .as_ref()
                .unwrap()
                .1
                .profiles
                .iter()
                .any(|profile| profile == &original.profile)
        );
        match field {
            0 => {
                peer.app
                    .views
                    .get_mut(&peer.target)
                    .unwrap()
                    .process
                    .workspace = peer.fixture.0.path().join("new-workspace")
            }
            1 => {
                peer.app
                    .views
                    .get_mut(&peer.target)
                    .unwrap()
                    .process
                    .incarnation = Uuid::new_v4()
            }
            2 => peer
                .app
                .cache_account_host(peer.target.route, Uuid::new_v4()),
            3 => peer.app.clients.mark_unavailable(peer.target.route),
            _ => peer.app.clients[peer.target.route].disconnect(),
        }
        let host_before = peer.app.account_host(peer.target.route);
        let catalogue_before = peer
            .app
            .inference
            .profiles
            .panel
            .as_ref()
            .unwrap()
            .catalogue
            .clone();
        let labels_before = peer.app.inference.profiles.labels.clone();
        let accounts_before: Vec<_> = peer
            .app
            .inference
            .profiles
            .accounts
            .iter()
            .map(|account| (account.binding.clone(), account.label.clone()))
            .collect();
        let (sender, receiver) = tokio::sync::oneshot::channel();
        peer.app.inference.profiles.job = Some(receiver);
        assert!(sender.send(response).is_ok());
        peer.app.poll_profiles();
        assert!(peer.app.inference.profiles.pending_save.is_none());
        assert!(peer.app.inference.profiles.unconfirmed_saves.is_empty());
        assert!(peer.app.inference.profiles.job_save_id.is_none());
        assert_eq!(peer.app.account_host(peer.target.route), host_before);
        assert_eq!(
            peer.app
                .inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .catalogue,
            catalogue_before
        );
        assert_eq!(peer.app.inference.profiles.labels, labels_before);
        assert_eq!(
            peer.app
                .inference
                .profiles
                .accounts
                .iter()
                .map(|account| (account.binding.clone(), account.label.clone()))
                .collect::<Vec<_>>(),
            accounts_before
        );
        assert!(
            peer.app
                .inference
                .profiles
                .panel
                .as_ref()
                .unwrap()
                .name
                .is_none()
        );
        assert_eq!(profile_changes(&peer).len(), 1);
        peer.unchanged();
    }
}

#[tokio::test]
async fn delayed_success_after_panel_close_keeps_caches_and_never_reopens_review() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let profile = peer.host.lock().unwrap().profile.clone();
    peer.app.inference.profiles.editing = Some((profile.id, "Closed acknowledged review".into()));
    peer.app.save_profile_settings(settings(&profile)).unwrap();
    let response = queued_success(&mut peer).await;
    let labels_before = peer.app.inference.profiles.labels.clone();
    peer.app.inference.profiles.panel = None;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    peer.app.inference.profiles.job = Some(receiver);
    assert!(sender.send(response).is_ok());
    peer.app.poll_profiles();
    assert!(peer.app.inference.profiles.pending_save.is_none());
    assert!(peer.app.inference.profiles.unconfirmed_saves.is_empty());
    assert!(peer.app.inference.profiles.panel.is_none());
    assert_eq!(peer.app.inference.profiles.labels, labels_before);
    assert_eq!(profile_changes(&peer).len(), 1);
    peer.unchanged();
}
