//! Complete account request pipelines over a scripted loopback host. No provider,
//! real credentials, browser, execution service or model invocation is used.
use super::super::account_socket_support_tests::{CODE, Peer};
use super::*;

async fn finish(peer: &mut Peer) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            peer.app.account_tick();
            if peer.app.accounts.reply.is_none()
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
    finish(peer).await;
}
async fn finish_usage(peer: &mut Peer) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            peer.app.account_tick();
            if peer.app.accounts.usage_reply.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
fn reviewed(peer: &Peer) -> Settings {
    Settings {
        account: Some(peer.host.lock().unwrap().account.clone()),
        provider: "openai-responses".into(),
        model: "reviewed-model".into(),
        ..Default::default()
    }
}
fn count(peer: &Peer, predicate: impl Fn(&VesselCommand) -> bool) -> usize {
    peer.host
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter(|c| predicate(c))
        .count()
}
fn intent_id(intent: &storage::Intent) -> Uuid {
    match &intent.command {
        VesselCommand::EnrollAccount { command_id, .. } => *command_id,
        _ => panic!("retained account intent must be an exact enrollment command"),
    }
}

#[tokio::test]
async fn full_account_load_binds_host_labels_and_capabilities_without_selecting_defaults() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let p = peer.app.accounts.picker.as_ref().unwrap();
    assert_eq!(p.host, Some(peer.host.lock().unwrap().host));
    assert!(p.enroll);
    assert!(p._lock.is_some());
    assert_eq!(p.catalogue.accounts.len(), 1);
    assert_eq!(p.original.model, "original-model");
    assert!(p.catalogue.default_account.is_none());
    assert!(!p.busy);
    finish_usage(&mut peer).await;
    assert_eq!(
        count(&peer, |c| matches!(c, VesselCommand::Capabilities)),
        1
    );
    assert_eq!(
        count(&peer, |c| matches!(c, VesselCommand::Accounts { .. })),
        1
    );
    assert_eq!(
        count(&peer, |c| matches!(
            c,
            VesselCommand::AccountDefaults { .. }
        )),
        1
    );
    assert_eq!(
        peer.app.account_host(peer.target.route),
        Some(peer.host.lock().unwrap().host)
    );
    peer.unchanged();
}

#[tokio::test]
async fn absent_capability_or_nil_host_refuses_before_account_observation() {
    for nil in [false, true] {
        let mut peer = Peer::new().await;
        {
            let mut host = peer.host.lock().unwrap();
            host.nil_host = nil;
            host.capabilities = nil;
        }
        open(&mut peer).await;
        let p = peer.app.accounts.picker.as_ref().unwrap();
        assert!(p.host.is_none());
        assert!(!p.busy);
        assert!(p.notice.contains("unavailable"));
        assert_eq!(
            count(&peer, |c| matches!(
                c,
                VesselCommand::Accounts { .. } | VesselCommand::AccountDefaults { .. }
            )),
            0
        );
        peer.unchanged();
    }
}

#[tokio::test]
async fn refused_catalogue_or_defaults_cannot_cache_a_host_or_echo_remote_private_diagnostics() {
    for defaults in [false, true] {
        let mut peer = Peer::new().await;
        {
            let mut host = peer.host.lock().unwrap();
            host.refuse_accounts = !defaults;
            host.refuse_defaults = defaults;
        }
        open(&mut peer).await;
        let p = peer.app.accounts.picker.as_ref().unwrap();
        assert!(p.host.is_none());
        assert!(!p.notice.contains(CODE));
        assert!(peer.app.accounts.labels.is_empty());
        assert!(peer.app.account_host(peer.target.route).is_none());
        peer.unchanged();
    }
}

#[tokio::test]
async fn private_start_is_followed_by_exact_resolution_and_human_only_status_not_replay() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    peer.app.accounts.picker.as_mut().unwrap().query = "personal".into();
    peer.app.begin_enrollment(Uuid::from_u128(353)).unwrap();
    finish(&mut peer).await;
    if peer.app.accounts.picker.as_ref().unwrap().private.is_none() {
        peer.app.poll_account_enrollment().unwrap();
        finish(&mut peer).await;
    }
    let p = peer.app.accounts.picker.as_ref().unwrap();
    let intent = p.intent.as_ref().unwrap();
    assert_eq!(p.private.as_ref().unwrap().user_code.as_deref(), Some(CODE));
    assert_eq!(p.outcome, Some(EnrollmentState::Pending));
    assert_eq!(
        intent_id(
            storage::load(intent.host, &intent.workspace)
                .unwrap()
                .enrollment
                .as_ref()
                .unwrap()
        ),
        intent_id(intent)
    );
    assert_eq!(
        count(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
    assert!(
        count(&peer, |c| matches!(
            c,
            VesselCommand::ResolveAccountEnrollment { .. }
        )) >= 1
    );
    assert!(!p.notice.contains(CODE));
    peer.unchanged();
}

#[tokio::test]
async fn changed_host_prevents_signin_effect_but_preserves_original_durable_intent() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let host = peer.app.accounts.picker.as_ref().unwrap().host.unwrap();
    peer.host.lock().unwrap().host = Uuid::new_v4();
    peer.app.accounts.picker.as_mut().unwrap().query = "retained".into();
    peer.app.begin_enrollment(Uuid::from_u128(353)).unwrap();
    finish(&mut peer).await;
    let p = peer.app.accounts.picker.as_ref().unwrap();
    assert_eq!(p.host, Some(host));
    assert!(p.intent.is_some());
    assert!(p.private.is_none());
    assert!(p.notice.contains("Original identity retained"));
    assert_eq!(
        count(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        0
    );
    assert!(
        storage::load(host, &p.workspace)
            .unwrap()
            .enrollment
            .is_some()
    );
    peer.unchanged();
}

#[tokio::test]
async fn private_status_failure_scrubs_code_and_cannot_erase_or_reissue_enrollment() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    peer.app.accounts.picker.as_mut().unwrap().query = "retained".into();
    peer.app.begin_enrollment(Uuid::from_u128(353)).unwrap();
    finish(&mut peer).await;
    let original = intent_id(
        peer.app
            .accounts
            .picker
            .as_ref()
            .unwrap()
            .intent
            .as_ref()
            .unwrap(),
    );
    peer.host.lock().unwrap().refuse_private = true;
    peer.app.poll_account_enrollment().unwrap();
    finish(&mut peer).await;
    let p = peer.app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none());
    assert_eq!(intent_id(p.intent.as_ref().unwrap()), original);
    assert!(!p.notice.contains(CODE));
    assert_eq!(
        count(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
    peer.unchanged();
}

#[tokio::test]
async fn uncertain_explicit_cancellation_reuses_its_exact_id_without_new_signin() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    peer.app.accounts.picker.as_mut().unwrap().query = "cancelled".into();
    peer.app.begin_enrollment(Uuid::from_u128(353)).unwrap();
    finish(&mut peer).await;
    peer.host.lock().unwrap().refuse_cancel = true;
    peer.app.cancel_enrollment().unwrap();
    finish(&mut peer).await;
    let id = peer
        .app
        .accounts
        .picker
        .as_ref()
        .unwrap()
        .intent
        .as_ref()
        .unwrap()
        .cancel
        .unwrap();
    peer.app.cancel_enrollment().unwrap();
    finish(&mut peer).await;
    let calls: Vec<_> = peer
        .host
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter_map(|command| {
            if let VesselCommand::CancelAccountEnrollment { command_id, .. } = command {
                Some(*command_id)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(calls, vec![id, id]);
    assert_eq!(
        count(&peer, |c| matches!(c, VesselCommand::EnrollAccount { .. })),
        1
    );
    let p = peer.app.accounts.picker.as_ref().unwrap();
    assert!(p.private.is_none());
    assert!(p.intent.is_some());
    peer.unchanged();
}

#[tokio::test]
async fn usage_failure_or_changed_account_is_a_private_observation_not_account_selection() {
    for changed in [false, true] {
        let mut peer = Peer::new().await;
        {
            let mut host = peer.host.lock().unwrap();
            host.refuse_usage = !changed;
            host.wrong_usage = changed;
        }
        open(&mut peer).await;
        finish_usage(&mut peer).await;
        let p = peer.app.accounts.picker.as_ref().unwrap();
        assert!(p.usage.is_empty());
        assert!(!p.notice.contains(CODE));
        peer.unchanged();
        assert_eq!(
            count(&peer, |c| matches!(c, VesselCommand::AccountUsage { .. })),
            1
        );
    }
}

#[tokio::test]
async fn explicit_usage_refresh_has_one_read_and_keeps_the_composer_and_settings() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    finish_usage(&mut peer).await;
    peer.app.refresh_account_usage(true).unwrap();
    finish_usage(&mut peer).await;
    let refreshes: Vec<_> = peer
        .host
        .lock()
        .unwrap()
        .calls
        .iter()
        .filter_map(|c| {
            if let VesselCommand::AccountUsage { refresh, .. } = c {
                Some(*refresh)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(refreshes, vec![false, true]);
    let account = peer.host.lock().unwrap().account.account_id;
    assert_eq!(
        peer.app.accounts.picker.as_ref().unwrap().usage[&account].refresh_status,
        AccountUsageRefreshStatus::Unsupported
    );
    peer.unchanged();
}

#[tokio::test]
async fn unconfirmed_or_mismatched_default_ack_retains_review_and_refuses_second_effect() {
    for wrong in [false, true] {
        let mut peer = Peer::new().await;
        open(&mut peer).await;
        {
            let mut host = peer.host.lock().unwrap();
            host.refuse_default_change = !wrong;
            host.wrong_default_ack = wrong;
        }
        let settings = reviewed(&peer);
        peer.app.accounts.picker.as_mut().unwrap().mode = Mode::DefaultConsent(settings);
        peer.app.set_default_account().unwrap();
        finish(&mut peer).await;
        let p = peer.app.accounts.picker.as_ref().unwrap();
        assert!(p.default_change_pending);
        assert!(matches!(p.mode, Mode::DefaultConsent(_)));
        assert!(p.notice.contains("not confirmed"));
        assert!(peer.app.set_default_account().is_err());
        assert_eq!(
            count(&peer, |c| matches!(
                c,
                VesselCommand::AccountSetDefault { .. }
            )),
            1
        );
        peer.unchanged();
    }
}

#[tokio::test]
async fn invalid_or_changed_settings_refuse_before_any_live_mutation() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    for field in 0..5 {
        let mut settings = reviewed(&peer);
        match field {
            0 => settings.model = "two models".into(),
            1 => settings.model = "x".repeat(257),
            2 => settings.account = None,
            3 => settings.reasoning_effort = Some("not valid".into()),
            _ => settings.service_tier = Some("x".repeat(65)),
        };
        assert!(peer.app.apply_account(settings).is_err());
        peer.unchanged();
    }
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
        .model = "changed-between-review".into();
    let settings = reviewed(&peer);
    assert!(peer.app.apply_account(settings).is_err());
    assert_eq!(count(&peer, |c| matches!(c, VesselCommand::Voyage(_))), 0);
    assert_eq!(
        peer.app.views[&peer.target].draft.text,
        "Unsent local prompt Δ"
    );
}

#[tokio::test]
async fn approved_live_account_change_owns_exact_pending_receipt_and_preserves_unsent_draft() {
    let mut peer = Peer::new().await;
    open(&mut peer).await;
    let settings = reviewed(&peer);
    let expected_account = settings.account.clone().unwrap();
    peer.app.apply_account(settings).unwrap();
    let pending = peer.app.views[&peer.target]
        .pending
        .as_ref()
        .unwrap()
        .clone();
    assert!(pending.preserve_draft);
    assert_eq!(pending.draft, "Unsent local prompt Δ");
    let Some(command) = pending.original.as_ref() else {
        panic!("exact mutation must be retained");
    };
    let VoyageCommand::SetAccountInference {
        command_id,
        account,
        expected_revision,
        model,
        ..
    } = command.as_ref()
    else {
        panic!("unexpected mutation");
    };
    assert_eq!(*command_id, pending.command_id);
    assert_eq!(*expected_revision, 17);
    assert_eq!(account, &expected_account);
    assert_eq!(model, "reviewed-model");
    let update = tokio::time::timeout(Duration::from_secs(3), peer.updates.recv())
        .await
        .unwrap()
        .unwrap();
    peer.app.update(update);
    assert_eq!(
        count(
            &peer,
            |c| matches!(c,VesselCommand::Voyage(v) if matches!(v.command,VoyageCommand::SetAccountInference{..}))
        ),
        1
    );
    assert_eq!(
        count(
            &peer,
            |c| matches!(c,VesselCommand::Voyage(v) if matches!(v.command,VoyageCommand::Submit{..}|VoyageCommand::SubmitContent{..}))
        ),
        0
    );
    assert_eq!(
        peer.app.views[&peer.target].draft.text,
        "Unsent local prompt Δ"
    );
    assert!(
        peer.app.views[&peer.target]
            .snapshot
            .as_ref()
            .unwrap()
            .messages
            .is_empty()
    );
    assert!(peer.app.accounts.picker.is_none());
}
