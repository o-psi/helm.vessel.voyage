//! Retired-transition metadata fences; no helper, chown, guardian or Root execution.
use super::*;
use crate::process::{identity_accounts::test_fixtures as f, test_support::Fixture};

fn fixture() -> (
    Intent,
    ConnectionGrant,
    SavedExecutionReview,
    RetiredJournalFacts,
) {
    let grant = f::connection();
    let source = f::registration();
    let source_identity = f::identity(AuthorityClass::Ordinary);
    let target = f::identity(AuthorityClass::Administrator);
    let saved = f::saved(
        f::facts(&target, &source, &grant),
        ExecutionOutcome::Launching,
    );
    let intent = Intent {
        schema: 1,
        review_id: saved.review.review_id,
        command_id: saved.review.command_id,
        principal: grant.principal_id,
        source: source.clone(),
        source_identity,
        target_facts: f::config_facts(&target),
        target,
        target_config: "/fixture/private-target-config".into(),
        target_incarnation: saved.review.facts.incarnation,
    };
    let retired = RetiredJournalFacts {
        session_id: source.session_id,
        source_incarnation: source.incarnation,
        revision: 17,
        frozen_config_digest: "b".repeat(64),
        history_digest: "c".repeat(64),
        pending_work_digest: "d".repeat(64),
        unresolved_cleanup: vec![Uuid::new_v4()],
    };
    (intent, grant, saved, retired)
}

#[test]
fn stopping_consent_and_original_ids_are_required_before_any_preparation() {
    let review = Uuid::new_v4();
    let command = Uuid::new_v4();
    preparation_consent(review, command, true).unwrap();
    for (review, command, stop) in [
        (review, command, false),
        (Uuid::nil(), command, true),
        (review, Uuid::nil(), true),
        (Uuid::nil(), Uuid::nil(), false),
    ] {
        assert!(preparation_consent(review, command, stop).is_err());
    }
}

#[test]
fn repeated_preparation_binds_original_command_principal_session_source_and_target_revision() {
    let (intent, grant, _, _) = fixture();
    preparation_current(
        &intent,
        intent.command_id,
        grant.principal_id,
        intent.source.session_id,
        intent.source.incarnation,
        &intent.target.identity,
    )
    .unwrap();
    for change in 0..6 {
        let mut command = intent.command_id;
        let mut principal = grant.principal_id;
        let mut session = intent.source.session_id;
        let mut source = intent.source.incarnation;
        let mut target = intent.target.identity.clone();
        match change {
            0 => command = Uuid::new_v4(),
            1 => principal = Uuid::new_v4(),
            2 => session = Uuid::new_v4(),
            3 => source = Uuid::new_v4(),
            4 => target.id = Uuid::new_v4(),
            _ => target.revision = NonZeroU64::new(2).unwrap(),
        };
        assert!(
            preparation_current(&intent, command, principal, session, source, &target).is_err()
        );
    }
}

#[test]
fn retired_source_identity_and_every_pending_work_fact_participate_in_review_digest() {
    let (intent, _, _, source) = fixture();
    source_facts_current(&intent, &source).unwrap();
    for change in 0..2 {
        let mut changed = source.clone();
        if change == 0 {
            changed.session_id = Uuid::new_v4();
        } else {
            changed.source_incarnation = Uuid::new_v4();
        }
        assert!(source_facts_current(&intent, &changed).is_err());
    }
    let original = digest(&source).unwrap();
    for change in 0..5 {
        let mut changed = source.clone();
        match change {
            0 => changed.revision += 1,
            1 => changed.frozen_config_digest = "0".repeat(64),
            2 => changed.history_digest = "0".repeat(64),
            3 => changed.pending_work_digest = "0".repeat(64),
            _ => changed.unresolved_cleanup.push(Uuid::new_v4()),
        };
        assert_ne!(digest(&changed).unwrap(), original);
    }
    let pending = source.unresolved_cleanup.clone();
    let _ = digest(&source).unwrap();
    assert_eq!(
        source.unresolved_cleanup, pending,
        "hashing cannot discharge unresolved effects"
    );
}

#[test]
fn approval_does_not_change_owner_command_or_saved_review_digest() {
    let (intent, grant, saved, _) = fixture();
    let approval = ReviewApproval {
        review_id: intent.review_id,
        command_id: intent.command_id,
        digest: saved.review.digest.clone(),
    };
    approval_current(&intent, grant.principal_id, &approval, &saved).unwrap();
    for change in 0..3 {
        let mut changed = approval.clone();
        let mut principal = grant.principal_id;
        match change {
            0 => principal = Uuid::new_v4(),
            1 => changed.command_id = Uuid::new_v4(),
            _ => changed.digest = "0".repeat(64),
        };
        assert!(approval_current(&intent, principal, &changed, &saved).is_err());
    }
}

#[test]
fn active_transition_authority_tracks_exact_saved_review_owner_connection_and_revision() {
    let (intent, grant, expected, _) = fixture();
    let revision = expected.review.facts.authority_revision.get();
    review_current(revision, &grant, &intent, &expected, &expected).unwrap();
    for change in 0..7 {
        let mut current = expected.clone();
        let mut current_grant = grant.clone();
        let mut current_intent = intent.clone();
        let mut authority = revision;
        match change {
            0 => authority += 1,
            1 => current_grant.grant_id = Uuid::new_v4(),
            2 => current_grant.revision += 1,
            3 => current_grant.principal_id = Uuid::new_v4(),
            4 => current_intent.principal = Uuid::new_v4(),
            5 => current.review.expires_at_ms += 1,
            _ => current.review.facts.pending_work_digest = "0".repeat(64),
        };
        assert!(
            review_current(
                authority,
                &current_grant,
                &current_intent,
                &expected,
                &current
            )
            .is_err()
        );
    }
    let mut unconfirmed = expected.clone();
    unconfirmed.receipt.outcome = ExecutionOutcome::Unconfirmed {
        cleanup_obligations: vec![intent.source.session_id],
    };
    review_current(revision, &grant, &intent, &expected, &unconfirmed).unwrap();
    for outcome in [
        ExecutionOutcome::AwaitingApproval,
        ExecutionOutcome::Approved,
        ExecutionOutcome::Cancelled,
        ExecutionOutcome::Refused {
            reason: ExecutionFailure::StaleReview,
        },
        ExecutionOutcome::RevocationRequested {
            administrator_grant_id: Uuid::new_v4(),
        },
        ExecutionOutcome::Ready {
            observed: f::observed(&intent.target, intent.target_incarnation),
        },
    ] {
        let mut current = expected.clone();
        current.receipt.outcome = outcome;
        assert!(review_current(revision, &grant, &intent, &expected, &current).is_err());
    }
}

#[test]
fn every_saved_review_fact_remains_exact_across_reconciliation() {
    let (intent, grant, expected, _) = fixture();
    let revision = expected.review.facts.authority_revision.get();
    let fields = [
        "vessel_id",
        "session_id",
        "run_id",
        "incarnation",
        "requester_id",
        "connection_id",
        "connection_revision",
        "administrative_owner_id",
        "authority_revision",
        "expected_session_revision",
        "change",
        "previous_incarnation",
        "identity",
        "account_context",
        "account",
        "account_capability_revision",
        "workspace",
        "host_identity_digest",
        "policy_digest",
        "release_digest",
        "pending_work_digest",
    ];
    for field in fields {
        let mut current = expected.clone();
        let mut value = serde_json::to_value(&current.review.facts).unwrap();
        let current_value = &value[field];
        value[field] = if current_value.is_number() {
            serde_json::json!(current_value.as_u64().unwrap() + 1)
        } else if current_value.is_object() {
            let mut replacement = current_value.clone();
            if let Some(id) = replacement.get_mut("id") {
                *id = serde_json::json!(Uuid::new_v4());
            } else {
                replacement["account_id"] = serde_json::json!(Uuid::new_v4());
            }
            replacement
        } else if field == "change" {
            serde_json::json!("replace_process")
        } else if field.ends_with("digest") {
            serde_json::json!("0".repeat(64))
        } else if field == "workspace" {
            serde_json::json!("/fixture/changed")
        } else {
            serde_json::json!(Uuid::new_v4())
        };
        current.review.facts = serde_json::from_value(value).unwrap();
        assert!(
            review_current(revision, &grant, &intent, &expected, &current).is_err(),
            "changed {field}"
        );
    }
}

#[test]
fn only_registration_process_state_can_change_during_retired_source_reconciliation() {
    let (intent, _, _, _) = fixture();
    for state in [
        ProcessState::Live,
        ProcessState::Suspended,
        ProcessState::Starting,
        ProcessState::Stopped,
        ProcessState::CleanupUnconfirmed,
        ProcessState::Unavailable,
        ProcessState::Relinquished,
    ] {
        let mut current = intent.source.clone();
        current.state = state;
        registration_current(&current, &intent.source, "fixture stale registration").unwrap();
    }
    for change in 0..11 {
        let mut current = intent.source.clone();
        match change {
            0 => current.session_id = Uuid::new_v4(),
            1 => current.incarnation = Uuid::new_v4(),
            2 => current.command_id = Uuid::new_v4(),
            3 => current.token = "different private process token".into(),
            4 => current.workspace = "/fixture/changed".into(),
            5 => current.config_path = None,
            6 => current.peer_uids = None,
            7 => current.executable = Some("/fixture/changed-binary".into()),
            8 => current.restart_from = Some(Uuid::new_v4()),
            9 => current.name = Some("Different canonical name".into()),
            _ => current.protocol += 1,
        };
        assert!(
            registration_current(&current, &intent.source, "fixture stale registration").is_err()
        );
    }
}

#[test]
fn phase_ids_are_stable_separate_and_not_new_effects_on_retry() {
    let review = Uuid::new_v4();
    let original = phase_command(review, "freeze");
    assert!(!original.is_nil());
    assert_eq!(phase_command(review, "freeze"), original);
    for phase in ["commit", "abort", "observe", "lookup"] {
        assert_ne!(phase_command(review, phase), original);
        assert_eq!(phase_command(review, phase), phase_command(review, phase));
    }
    assert_ne!(phase_command(Uuid::new_v4(), "freeze"), original);
}

#[test]
fn committed_config_and_history_must_match_reviewed_target_and_retired_source() {
    let (intent, _, saved, source) = fixture();
    let prepared = PreparedTransitionReceipt {
        command_id: phase_command(intent.review_id, "freeze"),
        transition_id: intent.review_id,
        session_id: intent.source.session_id,
        source_incarnation: intent.source.incarnation,
        target_incarnation: intent.target_incarnation,
        source_uid: intent.source_identity.uid,
        source_gid: intent.source_identity.gid,
        source_directory_device: 11,
        source_directory_inode: 12,
        target_uid: intent.target.uid,
        target_gid: intent.target.gid,
        previous_revision: source.revision,
        prepared_revision: source.revision + 1,
        previous_config_digest: source.frozen_config_digest.clone(),
        target_config_digest: intent.target_facts.config_digest.clone(),
        review_digest: saved.review.digest,
        history_digest: source.history_digest.clone(),
        pending_work_digest: source.pending_work_digest.clone(),
        retained_pending_work_digest: "e".repeat(64),
        unresolved_cleanup: source.unresolved_cleanup.clone(),
    };
    let receipt = TransitionReceipt {
        command_id: phase_command(intent.review_id, "commit"),
        prepared,
        resulting_revision: source.revision + 2,
        config_digest: intent.target_facts.config_digest.clone(),
        history_digest: source.history_digest.clone(),
        pending_work_digest: "e".repeat(64),
    };
    committed_current(&receipt, &intent, &source).unwrap();
    for change in 0..2 {
        let mut changed = receipt.clone();
        if change == 0 {
            changed.config_digest = "0".repeat(64);
        } else {
            changed.history_digest = "0".repeat(64);
        }
        assert!(committed_current(&changed, &intent, &source).is_err());
    }
    assert_eq!(
        receipt.prepared.unresolved_cleanup,
        source.unresolved_cleanup
    );
}

#[test]
fn retained_intent_is_private_strict_metadata_not_public_execution_authority() {
    let (intent, _, saved, _) = fixture();
    let private = serde_json::to_value(&intent).unwrap();
    assert_eq!(private["source"]["token"], intent.source.token);
    let public = serde_json::to_string(&saved).unwrap();
    assert!(!public.contains(&intent.source.token));
    assert!(!public.contains("private-target-config"));
    let mut injected = private;
    injected["credentials"] = serde_json::json!("not an accepted authority surface");
    assert!(serde_json::from_value::<Intent>(injected).is_err());
    use std::os::unix::ffi::OsStringExt;
    assert!(
        digest(&PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'/', 0xff
        ])))
        .is_err()
    );
}

#[tokio::test]
async fn invalid_stop_consent_returns_before_metadata_or_process_admission() {
    let root = Fixture::new();
    let supervisor = root.supervisor().await;
    let grant = root.connection();
    let identity = f::identity(AuthorityClass::Ordinary).identity;
    for (review, command, stop) in [
        (Uuid::new_v4(), Uuid::new_v4(), false),
        (Uuid::nil(), Uuid::new_v4(), true),
        (Uuid::new_v4(), Uuid::nil(), true),
    ] {
        assert!(
            supervisor
                .prepare_execution_transition(
                    &grant,
                    review,
                    command,
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    identity.clone(),
                    stop
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("source-stop consent")
        );
    }
    assert!(!root.0.join("execution-transitions").exists());
    assert!(!root.0.join("guardians").exists());
    let db = database::open(&root.0).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM voyages", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn unreadable_transition_state_is_not_retirement_or_a_writable_root_receipt() {
    let root = Fixture::new();
    let previous = root.registration();
    let mut next = previous.clone();
    next.incarnation = Uuid::new_v4();
    assert!(dormant(&root.0, previous.session_id, previous.incarnation).is_err());
    assert!(retained_digest(&root.0, &previous).is_err());
    carry_retained_digest(&root.0, &previous, &next).unwrap();
    assert!(!root.0.join("execution-transitions").exists());
    assert!(
        write(
            &root.0,
            Uuid::new_v4(),
            "intent",
            &serde_json::json!({"fixture":true})
        )
        .is_err()
    );
    assert!(!root.0.join("execution-transitions").exists());
}
