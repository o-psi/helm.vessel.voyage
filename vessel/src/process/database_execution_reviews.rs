//! Protected review/grant transactions, before public administrator launch exists.
//! Callers must independently reconstruct current host/account/work/cleanup facts.
#![allow(dead_code)] // Public launch and owner review transports are still gated.
use super::{OptionalExtension, Transaction, TransactionBehavior, blocking, params};
use anyhow::{Result, ensure};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::{
    execution_identity::*, execution_review_control::*, process::ConnectionGrant,
};

#[derive(Clone)]
struct Owner {
    principal: Uuid,
    connection: Uuid,
    connection_revision: u64,
    vessel: Uuid,
}
fn protected(root: &Path) -> Result<()> {
    ensure!(
        unsafe { libc::getuid() } == 0 && unsafe { libc::geteuid() } == 0,
        "administrator authority requires explicit root operator access"
    );
    let _control = voyage_storage::protected_linux::RootDirectory::open(root)?;
    Ok(())
}
fn owner(root: &Path, connection: &ConnectionGrant) -> Result<Owner> {
    protected(root)?;
    ensure!(
        connection.full_access,
        "administrator review requires an enrolled owner connection"
    );
    crate::process::access::store::current_connection(root, connection)?;
    let vessel = crate::process::identity::public(root)?.vessel_id;
    ensure!(
        connection.vessel_id == vessel,
        "administrator connection belongs to another Vessel"
    );
    Ok(Owner {
        principal: connection.principal_id,
        connection: connection.grant_id,
        connection_revision: connection.revision,
        vessel,
    })
}
fn schema(db: &rusqlite::Connection) -> Result<()> {
    let version: i64 = db.query_row("SELECT version FROM schema_version WHERE id=1", [], |r| {
        r.get(0)
    })?;
    ensure!(version == 3, "administrator owners are not provisioned");
    Ok(())
}
fn migrate_tx(tx: &Transaction<'_>) -> Result<()> {
    let version: i64 = tx.query_row("SELECT version FROM schema_version WHERE id=1", [], |r| {
        r.get(0)
    })?;
    if version == 2 {
        tx.execute_batch(include_str!("database_v3_migration.sql"))?;
    } else {
        ensure!(version == 3, "unsupported administrator authority schema");
    }
    Ok(())
}
fn now() -> Result<u64> {
    Ok(chrono::Utc::now().timestamp_millis().try_into()?)
}
fn authorize(tx: &Transaction<'_>, owner: &Owner) -> Result<u64> {
    let (vessel, revision): (String, u64) = tx.query_row(
        "SELECT vessel_id,revision FROM administrator_authority WHERE id=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let enrolled: bool = tx.query_row(
        "SELECT enabled FROM administrative_owners WHERE principal_id=?1",
        [owner.principal.to_string()],
        |r| r.get(0),
    )?;
    ensure!(
        enrolled && vessel == owner.vessel.to_string(),
        "administrative owner is not enrolled for this Vessel"
    );
    Ok(revision)
}
fn check_facts(tx: &Transaction<'_>, owner: &Owner, facts: &ReviewFacts) -> Result<()> {
    let authority = authorize(tx, owner)?;
    ensure!(
        facts.vessel_id == owner.vessel
            && facts.administrative_owner_id == owner.principal
            && facts.connection_id == owner.connection
            && facts.connection_revision.get() == owner.connection_revision
            && facts.authority_revision.get() == authority,
        "administrator review authority changed"
    );
    let (revision,record):(u64,String)=tx.query_row("SELECT revision,record FROM execution_identities WHERE identity_id=?1 ORDER BY revision DESC LIMIT 1",[facts.identity.id.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let configured: ConfiguredExecutionIdentity = serde_json::from_str(&record)?;
    ensure!(
        configured.enabled
            && revision == facts.identity.revision.get()
            && configured.identity == facts.identity
            && configured.authority == AuthorityClass::Administrator
            && configured.account_context == facts.account_context,
        "administrator execution identity changed or unavailable"
    );
    Ok(())
}
fn receipt(review: &ExecutionReview, outcome: ExecutionOutcome) -> ExecutionReceipt {
    ExecutionReceipt {
        schema: EXECUTION_SCHEMA,
        vessel_id: review.facts.vessel_id,
        session_id: review.facts.session_id,
        run_id: review.facts.run_id,
        incarnation: review.facts.incarnation,
        command_id: review.command_id,
        review_id: review.review_id,
        review_digest: review.digest.clone(),
        outcome,
    }
}
fn load(tx: &Transaction<'_>, id: Uuid) -> Result<SavedExecutionReview> {
    let (review, receipt, grant): (String, String, Option<String>) = tx.query_row(
        "SELECT review,receipt,grant_id FROM execution_reviews WHERE review_id=?1",
        [id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let mut saved = SavedExecutionReview {
        review: serde_json::from_str(&review)?,
        receipt: serde_json::from_str(&receipt)?,
        administrator_grant_id: grant.map(|v| v.parse()).transpose()?,
    };
    ensure!(
        saved.review.review_id == id
            && saved.receipt.review_id == id
            && saved.receipt.command_id == saved.review.command_id
            && saved.receipt.review_digest == saved.review.digest
            && saved.receipt.vessel_id == saved.review.facts.vessel_id
            && saved.receipt.session_id == saved.review.facts.session_id
            && saved.receipt.incarnation == saved.review.facts.incarnation,
        "administrator review receipt identity mismatch"
    );
    if let Some(grant) = saved.administrator_grant_id {
        let revoked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM administrator_revocations WHERE grant_id=?1)",
            [grant.to_string()],
            |r| r.get(0),
        )?;
        if revoked {
            saved.receipt.outcome = ExecutionOutcome::RevocationRequested {
                administrator_grant_id: grant,
            };
        }
    }
    Ok(saved)
}
fn set_receipt(tx: &Transaction<'_>, saved: &SavedExecutionReview) -> Result<()> {
    tx.execute(
        "UPDATE execution_reviews SET receipt=?2,grant_id=?3 WHERE review_id=?1",
        params![
            saved.review.review_id.to_string(),
            serde_json::to_string(&saved.receipt)?,
            saved.administrator_grant_id.map(|id| id.to_string())
        ],
    )?;
    Ok(())
}
fn revoke(tx: &Transaction<'_>, grant: Uuid, command: Uuid, time: u64) -> Result<()> {
    let record: String = tx.query_row(
        "SELECT record FROM administrator_grants WHERE grant_id=?1",
        [grant.to_string()],
        |r| r.get(0),
    )?;
    let record: AdministratorGrant = serde_json::from_str(&record)?;
    ensure!(
        time >= record.created_at_ms,
        "revocation clock predates authorization"
    );
    // A different cancellation may observe an existing fence; it never removes it.
    tx.execute(
        "INSERT OR IGNORE INTO administrator_revocations VALUES(?1,?2,?3)",
        params![grant.to_string(), command.to_string(), time],
    )?;
    ensure!(
        tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM administrator_revocations WHERE grant_id=?1)",
            [grant.to_string()],
            |r| r.get::<_, bool>(0)
        )?,
        "administrator revocation was not retained"
    );
    Ok(())
}
fn owner_change_tx(
    tx: &Transaction<'_>,
    vessel: Uuid,
    change: &AdministratorOwnerChange,
    time: u64,
) -> Result<AdministratorOwnerReceipt> {
    ensure!(
        !vessel.is_nil()
            && !change.command_id.is_nil()
            && !change.principal_id.is_nil()
            && time > 0,
        "invalid administrator owner operation"
    );
    migrate_tx(tx)?;
    let request = serde_json::to_string(change)?;
    if let Some((original, saved)) = tx
        .query_row(
            "SELECT request,receipt FROM administrator_owner_commands WHERE command_id=?1",
            [change.command_id.to_string()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        ensure!(
            original == request,
            "administrator owner command identity conflict"
        );
        let saved: AdministratorOwnerReceipt = serde_json::from_str(&saved)?;
        ensure!(
            saved.vessel_id == vessel,
            "administrator owner receipt belongs to another Vessel"
        );
        return Ok(saved);
    }
    let current: Option<(String, u64)> = tx
        .query_row(
            "SELECT vessel_id,revision FROM administrator_authority WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    ensure!(
        current
            .as_ref()
            .is_none_or(|(id, _)| id == &vessel.to_string()),
        "administrator authority belongs to another Vessel"
    );
    let revision = current.map(|(_, revision)| revision).unwrap_or(0);
    ensure!(
        revision == change.expected_authority_revision,
        "administrator owner authority revision changed"
    );
    let revision = revision
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| anyhow::anyhow!("administrator authority revision exhausted"))?;
    if change.enabled {
        let owners: u64 = tx.query_row("SELECT count(*) FROM administrative_owners", [], |r| {
            r.get(0)
        })?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM administrative_owners WHERE principal_id=?1)",
            [change.principal_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            exists || owners < 256,
            "administrator owner storage is full"
        );
    }
    let mut grants_fenced = Vec::new();
    if !change.enabled {
        ensure!(
            tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM administrative_owners WHERE principal_id=?1)",
                [change.principal_id.to_string()],
                |r| r.get::<_, bool>(0)
            )?,
            "administrative owner was never enrolled"
        );
        let mut query=tx.prepare("SELECT g.grant_id,g.record FROM administrator_grants g LEFT JOIN administrator_revocations r USING(grant_id) WHERE r.grant_id IS NULL")?;
        let grants = query
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (id, record) in grants {
            let grant: AdministratorGrant = serde_json::from_str(&record)?;
            if grant.administrative_owner_id != change.principal_id {
                continue;
            }
            ensure!(
                grant.vessel_id == vessel,
                "administrator grant Vessel mismatch"
            );
            let id: Uuid = id.parse()?;
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(
                [
                    b"voyage/owner-revocation/v1".as_slice(),
                    change.command_id.as_bytes(),
                    id.as_bytes(),
                ]
                .concat(),
            );
            let command = Uuid::from_bytes(digest[..16].try_into()?);
            revoke(tx, id, command, time)?;
            grants_fenced.push(id);
            let mut query =
                tx.prepare("SELECT review_id FROM execution_reviews WHERE grant_id=?1")?;
            let reviews = query
                .query_map([id.to_string()], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for review in reviews {
                let mut saved = load(tx, review.parse()?)?;
                saved.receipt.outcome = ExecutionOutcome::RevocationRequested {
                    administrator_grant_id: id,
                };
                set_receipt(tx, &saved)?;
            }
        }
    }
    tx.execute("INSERT INTO administrator_authority VALUES(1,?1,?2) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision",params![vessel.to_string(),revision])?;
    tx.execute("INSERT INTO administrative_owners VALUES(?1,?2) ON CONFLICT(principal_id) DO UPDATE SET enabled=excluded.enabled",params![change.principal_id.to_string(),change.enabled])?;
    grants_fenced.sort();
    let receipt = AdministratorOwnerReceipt {
        command_id: change.command_id,
        vessel_id: vessel,
        principal_id: change.principal_id,
        authority_revision: revision,
        enabled: change.enabled,
        grants_fenced,
    };
    tx.execute(
        "INSERT INTO administrator_owner_commands VALUES(?1,?2,?3)",
        params![
            change.command_id.to_string(),
            request,
            serde_json::to_string(&receipt)?
        ],
    )?;
    Ok(receipt)
}
/// Separate root/operator enrollment. A paired full-access connection is insufficient.
pub async fn change_owner(
    root: &Path,
    change: &AdministratorOwnerChange,
) -> Result<AdministratorOwnerReceipt> {
    protected(root)?;
    let vessel = crate::process::identity::public(root)?.vessel_id;
    let change = change.clone();
    let time = now()?;
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receipt = owner_change_tx(&tx, vessel, &change, time)?;
        tx.commit()?;
        Ok(receipt)
    })
    .await
}
fn prepare_tx(
    tx: &Transaction<'_>,
    owner: &Owner,
    review: &ExecutionReview,
    current: &ReviewFacts,
    time: u64,
) -> Result<SavedExecutionReview> {
    check_facts(tx, owner, current)?;
    let approval = ReviewApproval {
        review_id: review.review_id,
        command_id: review.command_id,
        digest: review.digest.clone(),
    };
    review
        .check_current(&approval, current, time)
        .map_err(|reason| anyhow::anyhow!("invalid administrator review: {reason:?}"))?;
    ensure!(
        serde_json::to_vec(review)?.len() <= 32768,
        "administrator review exceeds limit"
    );
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM execution_reviews WHERE review_id=?1)",
        [review.review_id.to_string()],
        |r| r.get::<_, bool>(0),
    )? {
        let saved = load(tx, review.review_id)?;
        ensure!(
            saved.review == *review,
            "administrator review identity conflict"
        );
        return Ok(saved);
    }
    let count: u64 = tx.query_row("SELECT count(*) FROM execution_reviews", [], |r| r.get(0))?;
    ensure!(count < 4096, "administrator review receipt storage is full");
    let saved = SavedExecutionReview {
        review: review.clone(),
        receipt: receipt(review, ExecutionOutcome::AwaitingApproval),
        administrator_grant_id: None,
    };
    tx.execute(
        "INSERT INTO execution_reviews VALUES(?1,?2,?3,?4,?5,NULL)",
        params![
            review.review_id.to_string(),
            review.command_id.to_string(),
            owner.principal.to_string(),
            serde_json::to_string(review)?,
            serde_json::to_string(&saved.receipt)?
        ],
    )?;
    Ok(saved)
}
pub async fn prepare(
    root: &Path,
    connection: &ConnectionGrant,
    review: &ExecutionReview,
    current: &ReviewFacts,
) -> Result<SavedExecutionReview> {
    let owner = owner(root, connection)?;
    let review = review.clone();
    let current = current.clone();
    let time = now()?;
    blocking(root, move |db| {
        schema(db)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved = prepare_tx(&tx, &owner, &review, &current, time)?;
        tx.commit()?;
        Ok(saved)
    })
    .await
}
fn approve_tx(
    tx: &Transaction<'_>,
    owner: &Owner,
    approval: &ReviewApproval,
    current: &ReviewFacts,
    time: u64,
) -> Result<SavedExecutionReview> {
    authorize(tx, owner)?;
    let mut saved = load(tx, approval.review_id)?;
    ensure!(
        saved.review.facts.administrative_owner_id == owner.principal
            && saved.review.command_id == approval.command_id
            && saved.review.digest == approval.digest,
        "administrator approval identity conflict"
    );
    // Reconciliation returns retained state, including a revoked/cancelled result.
    if saved.receipt.outcome != ExecutionOutcome::AwaitingApproval {
        return Ok(saved);
    }
    check_facts(tx, owner, current)?;
    saved
        .review
        .check_current(approval, current, time)
        .map_err(|reason| anyhow::anyhow!("administrator approval refused: {reason:?}"))?;
    let facts = &saved.review.facts;
    let active:u64=tx.query_row("SELECT count(*) FROM administrator_grants g LEFT JOIN administrator_revocations r USING(grant_id) WHERE g.session_id=?1 AND r.grant_id IS NULL",[facts.session_id.to_string()],|r|r.get(0))?;
    ensure!(
        active == 0,
        "voyage already has administrator authorization"
    );
    let grant = AdministratorGrant {
        schema: EXECUTION_SCHEMA,
        grant_id: Uuid::new_v4(),
        vessel_id: facts.vessel_id,
        session_id: facts.session_id,
        administrative_owner_id: owner.principal,
        authority_revision: facts.authority_revision,
        identity: facts.identity.clone(),
        account_context: facts.account_context.clone(),
        host_identity_digest: facts.host_identity_digest.clone(),
        policy_digest: facts.policy_digest.clone(),
        created_at_ms: time,
        revoked_at_ms: None,
    };
    grant
        .check_current(&grant)
        .map_err(|reason| anyhow::anyhow!("invalid administrator authorization: {reason:?}"))?;
    tx.execute(
        "INSERT INTO administrator_grants VALUES(?1,?2,?3)",
        params![
            grant.grant_id.to_string(),
            facts.session_id.to_string(),
            serde_json::to_string(&grant)?
        ],
    )?;
    saved.administrator_grant_id = Some(grant.grant_id);
    saved.receipt.outcome = ExecutionOutcome::Approved;
    set_receipt(tx, &saved)?;
    Ok(saved)
}
pub async fn approve(
    root: &Path,
    connection: &ConnectionGrant,
    approval: &ReviewApproval,
    current: &ReviewFacts,
) -> Result<SavedExecutionReview> {
    let owner = owner(root, connection)?;
    let approval = approval.clone();
    let current = current.clone();
    let time = now()?;
    blocking(root, move |db| {
        schema(db)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved = approve_tx(&tx, &owner, &approval, &current, time)?;
        tx.commit()?;
        Ok(saved)
    })
    .await
}
fn control_tx(
    tx: &Transaction<'_>,
    owner: &Owner,
    control: &ExecutionReviewControl,
    time: u64,
) -> Result<SavedExecutionReview> {
    authorize(tx, owner)?;
    ensure!(
        !control.command_id.is_nil(),
        "invalid administrator review control command"
    );
    let request = serde_json::to_string(control)?;
    if let Some((original, receipt)) = tx
        .query_row(
            "SELECT request,receipt FROM execution_review_controls WHERE command_id=?1",
            [control.command_id.to_string()],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    {
        ensure!(
            original == request,
            "administrator review control command conflict"
        );
        let mut saved = load(tx, control.review_id)?;
        ensure!(
            saved.review.facts.administrative_owner_id == owner.principal,
            "review belongs to another administrative owner"
        );
        saved.receipt = serde_json::from_str(&receipt)?;
        return Ok(saved);
    }
    let mut saved = load(tx, control.review_id)?;
    ensure!(
        saved.review.facts.administrative_owner_id == owner.principal
            && saved.review.digest == control.digest
            && saved.review.command_id != control.command_id,
        "administrator control review mismatch"
    );
    match control.action {
        ExecutionReviewControlAction::Cancel => {
            ensure!(
                matches!(
                    saved.receipt.outcome,
                    ExecutionOutcome::AwaitingApproval | ExecutionOutcome::Cancelled
                ),
                "approved authorization requires explicit revoke"
            );
            saved.receipt.outcome = ExecutionOutcome::Cancelled;
        }
        ExecutionReviewControlAction::Revoke => {
            let grant = saved
                .administrator_grant_id
                .ok_or_else(|| anyhow::anyhow!("review has no administrator authorization"))?;
            revoke(tx, grant, control.command_id, time)?;
            saved.receipt.outcome = ExecutionOutcome::RevocationRequested {
                administrator_grant_id: grant,
            };
        }
    }
    set_receipt(tx, &saved)?;
    tx.execute(
        "INSERT INTO execution_review_controls VALUES(?1,?2,?3,?4)",
        params![
            control.command_id.to_string(),
            control.review_id.to_string(),
            request,
            serde_json::to_string(&saved.receipt)?
        ],
    )?;
    Ok(saved)
}
pub async fn control(
    root: &Path,
    connection: &ConnectionGrant,
    control: &ExecutionReviewControl,
) -> Result<SavedExecutionReview> {
    let owner = owner(root, connection)?;
    let control = control.clone();
    let time = now()?;
    blocking(root, move |db| {
        schema(db)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let saved = control_tx(&tx, &owner, &control, time)?;
        tx.commit()?;
        Ok(saved)
    })
    .await
}
pub async fn resolve(
    root: &Path,
    connection: &ConnectionGrant,
    review_id: Uuid,
) -> Result<SavedExecutionReview> {
    let owner = owner(root, connection)?;
    blocking(root, move |db| {
        schema(db)?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Deferred)?;
        authorize(&tx, &owner)?;
        let saved = load(&tx, review_id)?;
        ensure!(
            saved.review.facts.administrative_owner_id == owner.principal,
            "review belongs to another administrative owner"
        );
        Ok(saved)
    })
    .await
}
#[cfg(test)]
#[path = "database_execution_reviews_tests.rs"]
mod tests;
