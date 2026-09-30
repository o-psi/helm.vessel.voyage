use super::*;
use std::{num::NonZeroU64, path::PathBuf};
fn fixture() -> (rusqlite::Connection, Owner, ExecutionReview) {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("database.sql")).unwrap();
    db.execute_batch(include_str!("database_v3_migration.sql"))
        .unwrap();
    let owner = Owner {
        principal: Uuid::new_v4(),
        connection: Uuid::new_v4(),
        connection_revision: 3,
        vessel: Uuid::new_v4(),
    };
    let identity = ConfiguredExecutionIdentity {
        identity: IdentityRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        label: "Administrator fixture".into(),
        user_name: "root".into(),
        uid: 0,
        gid: 0,
        supplementary_groups: vec![],
        home: PathBuf::from("/root"),
        account_context: AccountContextRef {
            id: Uuid::new_v4(),
            revision: NonZeroU64::new(1).unwrap(),
        },
        authority: AuthorityClass::Administrator,
        enabled: true,
    };
    let tx = db.transaction().unwrap();
    let change = AdministratorOwnerChange {
        command_id: Uuid::new_v4(),
        principal_id: owner.principal,
        expected_authority_revision: 0,
        enabled: true,
    };
    owner_change_tx(&tx, owner.vessel, &change, 1000).unwrap();
    tx.execute(
        "INSERT INTO execution_identities VALUES(?1,1,?2)",
        params![
            identity.identity.id.to_string(),
            serde_json::to_string(&identity).unwrap()
        ],
    )
    .unwrap();
    tx.commit().unwrap();
    let facts = ReviewFacts {
        vessel_id: owner.vessel,
        session_id: Uuid::new_v4(),
        run_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        requester_id: owner.principal,
        connection_id: owner.connection,
        connection_revision: NonZeroU64::new(3).unwrap(),
        administrative_owner_id: owner.principal,
        authority_revision: NonZeroU64::new(1).unwrap(),
        expected_session_revision: 0,
        change: ExecutionChange::Start,
        previous_incarnation: None,
        identity: identity.identity,
        account_context: identity.account_context,
        account: voyage_protocol::accounts::AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 1,
            connection_revision: 1,
            transport: voyage_protocol::accounts::Transport::ChatgptOauth,
        },
        account_capability_revision: 1,
        workspace: PathBuf::from("/work"),
        host_identity_digest: "a".repeat(64),
        policy_digest: "b".repeat(64),
        release_digest: "c".repeat(64),
        pending_work_digest: "d".repeat(64),
    };
    let mut review = ExecutionReview {
        schema: EXECUTION_SCHEMA,
        review_id: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        created_at_ms: 1000,
        expires_at_ms: 3000,
        facts,
        digest: String::new(),
    };
    review.digest = review.calculated_digest().unwrap();
    (db, owner, review)
}
fn approval(review: &ExecutionReview) -> ReviewApproval {
    ReviewApproval {
        review_id: review.review_id,
        command_id: review.command_id,
        digest: review.digest.clone(),
    }
}
fn prepared(db: &mut rusqlite::Connection, owner: &Owner, review: &ExecutionReview) {
    let tx = db.transaction().unwrap();
    prepare_tx(&tx, owner, review, &review.facts, 1100).unwrap();
    tx.commit().unwrap();
}
#[test]
fn owner_enrollment_is_explicit_exact_and_revision_pinned() {
    let (mut db, owner, _) = fixture();
    let tx = db.transaction().unwrap();
    let change = AdministratorOwnerChange {
        command_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        expected_authority_revision: 1,
        enabled: true,
    };
    let result = owner_change_tx(&tx, owner.vessel, &change, 1200).unwrap();
    assert_eq!(result.authority_revision, 2);
    assert_eq!(
        owner_change_tx(&tx, owner.vessel, &change, 5000).unwrap(),
        result
    );
    let mut conflicting = change.clone();
    conflicting.enabled = false;
    assert!(owner_change_tx(&tx, owner.vessel, &conflicting, 1300).is_err());
    let mut stale = change;
    stale.command_id = Uuid::new_v4();
    assert!(owner_change_tx(&tx, owner.vessel, &stale, 1300).is_err());
    let mut unregistered = owner.clone();
    unregistered.principal = Uuid::new_v4();
    assert!(authorize(&tx, &unregistered).is_err());
}
#[test]
fn approved_review_is_exact_retained_and_never_launches() {
    let (mut db, owner, review) = fixture();
    prepared(&mut db, &owner, &review);
    let tx = db.transaction().unwrap();
    let first = approve_tx(&tx, &owner, &approval(&review), &review.facts, 1200).unwrap();
    assert_eq!(first.receipt.outcome, ExecutionOutcome::Approved);
    assert!(first.administrator_grant_id.is_some());
    assert_eq!(
        approve_tx(&tx, &owner, &approval(&review), &review.facts, 5000).unwrap(),
        first
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM administrator_grants", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM voyages", [], |r| r.get::<_, u64>(0))
            .unwrap(),
        0
    );
    let mut wrong = approval(&review);
    wrong.digest = "e".repeat(64);
    assert!(approve_tx(&tx, &owner, &wrong, &review.facts, 1200).is_err());
}
#[test]
fn stale_expired_and_changed_identity_approvals_create_no_grant() {
    let (mut db, owner, review) = fixture();
    prepared(&mut db, &owner, &review);
    let tx = db.transaction().unwrap();
    assert!(approve_tx(&tx, &owner, &approval(&review), &review.facts, 3000).is_err());
    let mut changed = review.facts.clone();
    changed.policy_digest = "e".repeat(64);
    assert!(approve_tx(&tx, &owner, &approval(&review), &changed, 1200).is_err());
    tx.execute(
        "INSERT INTO execution_identities SELECT identity_id,2,record FROM execution_identities",
        [],
    )
    .unwrap();
    assert!(approve_tx(&tx, &owner, &approval(&review), &review.facts, 1200).is_err());
    assert_eq!(
        tx.query_row("SELECT count(*) FROM administrator_grants", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn cancel_is_durable_and_late_approval_does_not_authorize() {
    let (mut db, owner, review) = fixture();
    prepared(&mut db, &owner, &review);
    let tx = db.transaction().unwrap();
    let control = ExecutionReviewControl {
        command_id: Uuid::new_v4(),
        review_id: review.review_id,
        digest: review.digest.clone(),
        action: ExecutionReviewControlAction::Cancel,
    };
    let cancelled = control_tx(&tx, &owner, &control, 1200).unwrap();
    assert_eq!(cancelled.receipt.outcome, ExecutionOutcome::Cancelled);
    assert_eq!(control_tx(&tx, &owner, &control, 5000).unwrap(), cancelled);
    assert_eq!(
        approve_tx(&tx, &owner, &approval(&review), &review.facts, 1300).unwrap(),
        cancelled
    );
    let mut conflicting = control;
    conflicting.action = ExecutionReviewControlAction::Revoke;
    assert!(control_tx(&tx, &owner, &conflicting, 1300).is_err());
    assert_eq!(
        tx.query_row("SELECT count(*) FROM administrator_grants", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn explicit_revoke_fences_grant_and_does_not_claim_cleanup() {
    let (mut db, owner, review) = fixture();
    prepared(&mut db, &owner, &review);
    let tx = db.transaction().unwrap();
    let approved = approve_tx(&tx, &owner, &approval(&review), &review.facts, 1200).unwrap();
    let mut control = ExecutionReviewControl {
        command_id: Uuid::new_v4(),
        review_id: review.review_id,
        digest: review.digest.clone(),
        action: ExecutionReviewControlAction::Cancel,
    };
    assert!(control_tx(&tx, &owner, &control, 1250).is_err());
    control.action = ExecutionReviewControlAction::Revoke;
    let revoked = control_tx(&tx, &owner, &control, 1300).unwrap();
    assert_eq!(
        revoked.receipt.outcome,
        ExecutionOutcome::RevocationRequested {
            administrator_grant_id: approved.administrator_grant_id.unwrap()
        }
    );
    assert_eq!(control_tx(&tx, &owner, &control, 1400).unwrap(), revoked);
    assert_eq!(
        approve_tx(&tx, &owner, &approval(&review), &review.facts, 1400).unwrap(),
        revoked
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM administrator_revocations", [], |r| {
            r.get::<_, u64>(0)
        })
        .unwrap(),
        1
    );
}
#[test]
fn removing_owner_fences_all_grants_atomically_and_invalidates_old_reviews() {
    let (mut db, owner, review) = fixture();
    prepared(&mut db, &owner, &review);
    let mut second = review.clone();
    second.review_id = Uuid::new_v4();
    second.command_id = Uuid::new_v4();
    second.facts.session_id = Uuid::new_v4();
    second.digest = second.calculated_digest().unwrap();
    prepared(&mut db, &owner, &second);
    let tx = db.transaction().unwrap();
    let a = approve_tx(&tx, &owner, &approval(&review), &review.facts, 1200).unwrap();
    let b = approve_tx(&tx, &owner, &approval(&second), &second.facts, 1200).unwrap();
    let change = AdministratorOwnerChange {
        command_id: Uuid::new_v4(),
        principal_id: owner.principal,
        expected_authority_revision: 1,
        enabled: false,
    };
    let result = owner_change_tx(&tx, owner.vessel, &change, 1300).unwrap();
    assert_eq!(result.grants_fenced.len(), 2);
    assert!(
        result
            .grants_fenced
            .contains(&a.administrator_grant_id.unwrap())
    );
    assert!(
        result
            .grants_fenced
            .contains(&b.administrator_grant_id.unwrap())
    );
    assert!(authorize(&tx, &owner).is_err());
    assert!(matches!(
        load(&tx, review.review_id).unwrap().receipt.outcome,
        ExecutionOutcome::RevocationRequested { .. }
    ));
    assert_eq!(
        owner_change_tx(&tx, owner.vessel, &change, 1400).unwrap(),
        result
    );
}
#[test]
fn ordinary_operator_refuses_before_mutation() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = crate::process::test_support::Fixture::new();
    assert!(protected(&root.0).is_err());
    assert!(!root.0.join("catalogue.sqlite3").exists());
}

#[test]
fn normal_catalogue_open_retains_schema_two_and_root_migration_is_explicit() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = crate::process::test_support::Fixture::new();
    let mut db = crate::process::database::open(&root.0).unwrap();
    let version: i64 = db
        .query_row("SELECT version FROM schema_version WHERE id=1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(version, 2);
    assert!(schema(&db).is_err());
    let tx = db.transaction().unwrap();
    migrate_tx(&tx).unwrap();
    tx.commit().unwrap();
    assert_eq!(
        db.query_row("SELECT version FROM schema_version WHERE id=1", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        3
    );
    drop(db);
    assert!(crate::process::database::open(&root.0).is_ok());
}

#[test]
fn rejected_first_enrollment_rolls_back_schema_and_authority() {
    let mut db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch(include_str!("database.sql")).unwrap();
    let tx = db.transaction().unwrap();
    let invalid = AdministratorOwnerChange {
        command_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        expected_authority_revision: 7,
        enabled: true,
    };
    assert!(owner_change_tx(&tx, Uuid::new_v4(), &invalid, 1200).is_err());
    drop(tx);
    assert_eq!(
        db.query_row("SELECT version FROM schema_version WHERE id=1", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        2
    );
    assert!(
        !db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='administrator_authority')",
            [],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
}

#[test]
fn independently_revoked_grant_is_visible_on_approval_reconciliation() {
    let (mut db, owner, review) = fixture();
    prepared(&mut db, &owner, &review);
    let tx = db.transaction().unwrap();
    let approved = approve_tx(&tx, &owner, &approval(&review), &review.facts, 1200).unwrap();
    let grant = approved.administrator_grant_id.unwrap();
    revoke(&tx, grant, Uuid::new_v4(), 1300).unwrap();
    let result = approve_tx(&tx, &owner, &approval(&review), &review.facts, 1400).unwrap();
    assert_eq!(
        result.receipt.outcome,
        ExecutionOutcome::RevocationRequested {
            administrator_grant_id: grant
        }
    );
    assert_eq!(
        tx.query_row("SELECT count(*) FROM administrator_grants", [], |r| r
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
}
