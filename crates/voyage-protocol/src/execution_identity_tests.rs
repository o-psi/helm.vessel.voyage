use super::*;
use serde_json::json;

fn review() -> ExecutionReview {
    let mut review = ExecutionReview {
        schema: EXECUTION_SCHEMA,
        review_id: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        created_at_ms: 100,
        expires_at_ms: 1000,
        facts: ReviewFacts {
            vessel_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            incarnation: Uuid::new_v4(),
            requester_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            connection_revision: NonZeroU64::new(1).unwrap(),
            administrative_owner_id: Uuid::new_v4(),
            authority_revision: NonZeroU64::new(1).unwrap(),
            expected_session_revision: 3,
            change: ExecutionChange::Start,
            previous_incarnation: None,
            identity: IdentityRef {
                id: Uuid::new_v4(),
                revision: NonZeroU64::new(1).unwrap(),
            },
            account_context: AccountContextRef {
                id: Uuid::new_v4(),
                revision: NonZeroU64::new(1).unwrap(),
            },
            account: crate::accounts::AccountBinding {
                account_id: Uuid::new_v4(),
                connection_id: Uuid::new_v4(),
                identity_generation: 1,
                connection_revision: 1,
                transport: crate::accounts::Transport::ChatgptOauth,
            },
            account_capability_revision: 1,
            workspace: "/workspace".into(),
            host_identity_digest: "a".repeat(64),
            policy_digest: "b".repeat(64),
            release_digest: "c".repeat(64),
            pending_work_digest: "d".repeat(64),
        },
        digest: "e".repeat(64),
    };
    review.digest = review.calculated_digest().unwrap();
    review
}
fn approval_for(review: &ExecutionReview) -> ReviewApproval {
    ReviewApproval {
        review_id: review.review_id,
        command_id: review.command_id,
        digest: review.digest.clone(),
    }
}

#[test]
fn administrator_grant_is_one_voyage_and_revocable_across_process_replacements() {
    let facts = review().facts;
    let mut grant = AdministratorGrant {
        schema: EXECUTION_SCHEMA,
        grant_id: Uuid::new_v4(),
        vessel_id: facts.vessel_id,
        session_id: facts.session_id,
        administrative_owner_id: facts.administrative_owner_id,
        authority_revision: facts.authority_revision,
        identity: facts.identity,
        account_context: facts.account_context,
        host_identity_digest: facts.host_identity_digest,
        policy_digest: facts.policy_digest,
        created_at_ms: 100,
        revoked_at_ms: None,
    };
    assert_eq!(grant.check_current(&grant), Ok(()));
    for (field, replacement) in [
        ("vessel_id", json!(Uuid::new_v4())),
        ("session_id", json!(Uuid::new_v4())),
        ("administrative_owner_id", json!(Uuid::new_v4())),
        ("authority_revision", json!(2)),
        ("host_identity_digest", json!("c".repeat(64))),
        ("policy_digest", json!("d".repeat(64))),
    ] {
        let mut changed = serde_json::to_value(&grant).unwrap();
        changed[field] = replacement;
        let changed = serde_json::from_value(changed).unwrap();
        assert_eq!(
            grant.check_current(&changed),
            Err(ExecutionFailure::StaleReview)
        );
    }
    grant.revoked_at_ms = Some(101);
    assert_eq!(
        grant.check_current(&grant),
        Err(ExecutionFailure::OwnerRequired)
    );
}

#[test]
fn execution_reviews_bind_every_authority_and_launch_fact() {
    let review = review();
    let approval = approval_for(&review);
    assert_eq!(review.check_current(&approval, &review.facts, 100), Ok(()));
    // A permission, account, policy, pending-work or identity edit invalidates a
    // displayed review. This also covers a new run in the same incarnation.
    let original = serde_json::to_value(&review.facts).unwrap();
    let replacements = [
        ("vessel_id", json!(Uuid::new_v4())),
        ("session_id", json!(Uuid::new_v4())),
        ("run_id", json!(Uuid::new_v4())),
        ("incarnation", json!(Uuid::new_v4())),
        ("requester_id", json!(Uuid::new_v4())),
        ("connection_id", json!(Uuid::new_v4())),
        ("connection_revision", json!(2)),
        ("administrative_owner_id", json!(Uuid::new_v4())),
        ("authority_revision", json!(2)),
        ("expected_session_revision", json!(4)),
        ("change", json!("transition")),
        ("previous_incarnation", json!(Uuid::new_v4())),
        (
            "identity",
            json!({"id": review.facts.identity.id, "revision":2}),
        ),
        (
            "account_context",
            json!({"id": review.facts.account_context.id, "revision":2}),
        ),
        ("account_capability_revision", json!(2)),
        ("workspace", json!("/another")),
        ("host_identity_digest", json!("f".repeat(64))),
        ("policy_digest", json!("f".repeat(64))),
        ("release_digest", json!("f".repeat(64))),
        ("pending_work_digest", json!("f".repeat(64))),
    ];
    for (field, value) in replacements {
        let mut changed = original.clone();
        changed[field] = value;
        let current = serde_json::from_value(changed).unwrap();
        assert_eq!(
            review.check_current(&approval, &current, 500),
            Err(ExecutionFailure::StaleReview),
            "{field}"
        );
    }
    for field in [
        "account_id",
        "connection_id",
        "identity_generation",
        "connection_revision",
        "transport",
    ] {
        let mut changed = original.clone();
        changed["account"][field] = match field {
            "account_id" | "connection_id" => json!(Uuid::new_v4()),
            "transport" => json!("anthropic"),
            _ => json!(2),
        };
        let current = serde_json::from_value(changed).unwrap();
        assert_eq!(
            review.check_current(&approval, &current, 500),
            Err(ExecutionFailure::StaleReview),
            "{field}"
        );
    }
    for field in ["review_id", "command_id", "digest"] {
        let mut changed = serde_json::to_value(&approval).unwrap();
        changed[field] = if field == "digest" {
            json!("f".repeat(64))
        } else {
            json!(Uuid::new_v4())
        };
        let changed = serde_json::from_value(changed).unwrap();
        assert_eq!(
            review.check_current(&changed, &review.facts, 500),
            Err(ExecutionFailure::StaleReview)
        );
    }
}

#[test]
fn execution_reviews_reject_expiry_clock_rollback_and_invalid_saved_records() {
    let mut review = review();
    let approval = approval_for(&review);
    for now in [0, 99, 1000, u64::MAX] {
        assert_eq!(
            review.check_current(&approval, &review.facts, now),
            Err(ExecutionFailure::ExpiredReview)
        );
    }
    let original = serde_json::to_value(&review).unwrap();
    for (field, value) in [
        ("schema", json!(2)),
        ("review_id", json!(Uuid::nil())),
        ("command_id", json!(Uuid::nil())),
        ("digest", json!("A".repeat(64))),
        ("expires_at_ms", json!(100)),
        ("expires_at_ms", json!(MAX_REVIEW_LIFETIME_MS + 101)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        let changed: ExecutionReview = serde_json::from_value(changed).unwrap();
        assert_eq!(
            changed.check_current(&approval, &changed.facts, 500),
            Err(ExecutionFailure::InvalidReview),
            "{field}"
        );
    }
    review.facts.change = ExecutionChange::Transition;
    assert_eq!(
        review.check_current(&approval, &review.facts, 500),
        Err(ExecutionFailure::InvalidReview)
    );
    review.facts.previous_incarnation = Some(review.facts.incarnation);
    assert_eq!(
        review.check_current(&approval, &review.facts, 500),
        Err(ExecutionFailure::InvalidReview)
    );
    review.facts.previous_incarnation = Some(Uuid::new_v4());
    // Even internally consistent modified facts cannot reuse the old digest.
    assert_eq!(
        review.check_current(&approval, &review.facts, 500),
        Err(ExecutionFailure::InvalidReview)
    );
    review.digest = review.calculated_digest().unwrap();
    assert_eq!(
        review.check_current(&approval_for(&review), &review.facts, 500),
        Ok(())
    );
    review.facts.workspace = "relative".into();
    assert_eq!(
        review.check_current(&approval, &review.facts, 500),
        Err(ExecutionFailure::InvalidReview)
    );
}

#[test]
fn execution_contracts_do_not_accept_ambient_or_client_supplied_os_authority() {
    assert_eq!(
        ExecutionCapability::default(),
        ExecutionCapability::Unavailable
    );
    let identity = json!({"id": Uuid::new_v4(), "revision": 1});
    for extra in [
        "uid",
        "gid",
        "groups",
        "home",
        "executable",
        "environment",
        "account_path",
    ] {
        let mut value = identity.clone();
        value[extra] = json!(0);
        assert!(
            serde_json::from_value::<IdentityRef>(value).is_err(),
            "{extra}"
        );
    }
    let mut zero = identity;
    zero["revision"] = json!(0);
    assert!(serde_json::from_value::<IdentityRef>(zero).is_err());
    for value in [
        json!(true),
        json!("approved"),
        json!({"root_grant":"approved"}),
    ] {
        assert!(serde_json::from_value::<ReviewApproval>(value).is_err());
    }
    let review = review();
    let mut value = serde_json::to_value(approval_for(&review)).unwrap();
    value["uid"] = json!(0);
    assert!(serde_json::from_value::<ReviewApproval>(value).is_err());
    let mut value = serde_json::to_value(review).unwrap();
    value["facts"]["environment"] = json!({"HOME":"/root"});
    assert!(serde_json::from_value::<ExecutionReview>(value).is_err());
}
