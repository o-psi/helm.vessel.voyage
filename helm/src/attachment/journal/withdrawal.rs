//! Permanent local retirement of a dedicated remote grant. No positive grant or network authority.
use super::super::local_actor::LocalActor;
use super::*;

pub(super) const SCHEMA: &str =
    "CREATE TABLE remote_withdrawal(slot INTEGER PRIMARY KEY CHECK(slot=1),receipt TEXT NOT NULL);";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalRequest {
    pub session_id: Uuid,
    pub operation_id: Uuid,
    pub expected_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithdrawalReceipt {
    pub request: WithdrawalRequest,
    pub binding: RemoteBinding,
    pub confirmation_digest: String,
    pub revision: u64,
    pub cancellation_requested: Option<Uuid>,
    pub last_public_cursor: u64,
}
#[derive(Debug, Serialize)]
pub struct WithdrawalPreview {
    pub request: WithdrawalRequest,
    pub binding: RemoteBinding,
    pub confirmation_digest: String,
    pub already_withdrawn: bool,
}
#[derive(Debug, Serialize)]
pub struct RemoteConsentStatus {
    pub session_id: Uuid,
    pub binding: RemoteBinding,
    pub revision: u64,
    pub withdrawn: bool,
    pub receipt: Option<WithdrawalReceipt>,
    pub pending_cleanup_run: Option<Uuid>,
    pub run: Option<RemoteConsentRun>,
}
#[derive(Debug, Serialize)]
pub struct RemoteConsentRun {
    pub run_id: Uuid,
    pub state: RunState,
    pub cleanup: voyage_protocol::events::CleanupState,
}
/// Read-only durable authority observation synchronized with its owner's commits.
/// The mutex is process-local scheduling only; SQLite remains the cross-process fence.
pub struct RemoteGrantObserver {
    journal: std::sync::Mutex<Journal>,
    fence: std::sync::Arc<std::sync::Mutex<()>>,
    binding: RemoteBinding,
    session: Uuid,
}
impl std::fmt::Debug for RemoteGrantObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteGrantObserver")
    }
}
impl RemoteGrantObserver {
    pub fn check(&self) -> Result<()> {
        self.check_with(|| {})
    }
    fn check_with(&self, checked: impl FnOnce()) -> Result<()> {
        let journal = self
            .journal
            .lock()
            .map_err(|_| anyhow::anyhow!("remote observer poisoned"))?;
        let _fence = self
            .fence
            .lock()
            .map_err(|_| anyhow::anyhow!("journal observer fence poisoned"))?;
        journal.check_remote_grant_with(&self.binding, self.session, checked)
    }
}
fn digest(request: &WithdrawalRequest, binding: &RemoteBinding) -> Result<String> {
    let bytes = serde_json::to_vec(&("voyage-dedicated-withdrawal-v1", request, binding))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn validate(request: &WithdrawalRequest) -> Result<()> {
    ensure!(
        !request.session_id.is_nil()
            && !request.operation_id.is_nil()
            && request.expected_revision == 0,
        "withdrawal requires exact identities and active grant revision zero"
    );
    Ok(())
}
// Older schemas have no retirement facility. Current remote entrypoints require schema8;
// old local recovery remains usable until an explicit quiescent upgrade.
pub(super) fn read(db: &Connection) -> Result<Option<WithdrawalReceipt>> {
    let version: i64 = db.query_row(
        "SELECT version FROM attachment_schema WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if version < 8 {
        return Ok(None);
    }
    let row: Option<Option<String>> = db.query_row(
        "SELECT CASE WHEN length(CAST(receipt AS BLOB))<=4096 THEN receipt END FROM remote_withdrawal WHERE slot=1", [], |r| r.get(0)).optional()?;
    row.map(|value| {
        let receipt: WithdrawalReceipt =
            serde_json::from_str(&value.context("invalid withdrawal receipt size")?)?;
        validate(&receipt.request)?;
        remote::require(db, &receipt.binding, receipt.request.session_id)?;
        ensure!(
            receipt.revision == 1
                && receipt.confirmation_digest == digest(&receipt.request, &receipt.binding)?
                && receipt.cancellation_requested.is_none_or(|id| !id.is_nil()),
            "invalid withdrawal evidence"
        );
        let latest: i64 = db.query_row(
            "SELECT next_sequence-1 FROM remote_session WHERE slot=1",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            u64::try_from(latest).ok() == Some(receipt.last_public_cursor),
            "public cursor changed after withdrawal"
        );
        if let Some(id) = receipt.cancellation_requested {
            let run = read_run(db, id)?;
            ensure!(
                run.session_id == receipt.request.session_id
                    && run.machine_id == receipt.binding.machine_id
                    && run.principal_id == receipt.binding.owner_id
                    && catalogue::pending(db, run.session_id, id)?,
                "withdrawal cancellation evidence mismatch"
            );
        }
        Ok(receipt)
    })
    .transpose()
}
pub(super) fn require_active(db: &Connection) -> Result<()> {
    ensure!(
        read(db)?.is_none(),
        "dedicated remote grant permanently withdrawn"
    );
    Ok(())
}
fn local_binding(db: &Connection, actor: &LocalActor) -> Result<(Uuid, RemoteBinding)> {
    let (session, binding) = remote::binding(db)?.context("not a dedicated remote journal")?;
    ensure!(
        binding.local_installation_id == actor.installation_id
            && binding.local_principal_id == actor.principal_id,
        "local installation attribution mismatch"
    );
    Ok((session, binding))
}
impl Journal {
    /// Construct while the owner is idle, before admitting any executor. The returned
    /// observer has no mutation API. Write transactions may call it before COMMIT;
    /// they never hold the process-local commit mutex while checking authority.
    pub(crate) fn remote_grant_observer(
        &mut self,
        expected: RemoteBinding,
        session: Uuid,
    ) -> Result<RemoteGrantObserver> {
        self.check_remote_grant(&expected, session)?;
        let observer = Journal::open(self.directory.clone())?;
        // A spill can take SQLite's EXCLUSIVE lock before COMMIT, making a current
        // observer deny this owner's unchanged grant. Defer dirty-page spill on this
        // connection only; commit() serializes the eventual write with the observer.
        // Transactional dirty memory may exceed the usual cache target; existing
        // snapshot/output/database bounds still apply. No retry or WAL change.
        self.connection.pragma_update(None, "cache_spill", false)?;
        let spill: i64 = self
            .connection
            .pragma_query_value(None, "cache_spill", |row| row.get(0))?;
        ensure!(
            spill == 0,
            "remote observer requires deferred dirty-page spill"
        );
        let fence = self
            .commit_fence
            .get_or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(())))
            .clone();
        Ok(RemoteGrantObserver {
            journal: std::sync::Mutex::new(observer),
            fence,
            binding: expected,
            session,
        })
    }
    pub fn remote_consent_status(&self, actor: &LocalActor) -> Result<RemoteConsentStatus> {
        self.check_schema()?;
        ensure!(
            self.opened_schema >= 8,
            "explicit quiescent journal upgrade required"
        );
        let tx = self.connection.unchecked_transaction()?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let (session_id, binding) = local_binding(&tx, actor)?;
        let receipt = read(&tx)?;
        let pending_cleanup_run = catalogue::pending_cleanup(&tx, session_id)?;
        let latest: Option<Option<String>> = tx
            .query_row(
                "SELECT CASE WHEN length(CAST(id AS BLOB))=36 THEN id END FROM runs WHERE session_id=?1 ORDER BY rowid DESC LIMIT 1",
                [session_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let run = latest
            .map(|id| -> Result<RemoteConsentRun> {
                let id = id.context("invalid remote run identity size")?;
                let run = read_run(&tx, Uuid::parse_str(&id)?)?;
                let confirmation: Option<(bool,Option<String>)> = tx.query_row(
                    "SELECT confirmation IS NULL,CASE WHEN length(CAST(confirmation AS BLOB))<=32 THEN confirmation END FROM local_cleanup_obligations WHERE run_id=?1", [&id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
                let confirmation = match confirmation {
                    Some((false,value)) => Some(value.context("invalid cleanup evidence size")?),
                    _ => None,
                };
                use voyage_protocol::events::CleanupState;
                let cleanup = match confirmation.as_deref() {
                    Some("observed") => CleanupState::Observed,
                    Some("operator_attested") => CleanupState::OperatorAttested,
                    Some(_) => anyhow::bail!("invalid cleanup evidence"),
                    None if matches!(run.state, RunState::Accepted | RunState::Running) => {
                        CleanupState::Pending
                    }
                    None => CleanupState::Unconfirmed,
                };
                Ok(RemoteConsentRun {
                    run_id: run.id,
                    state: run.state,
                    cleanup,
                })
            })
            .transpose()?;
        let result = RemoteConsentStatus {
            session_id,
            binding,
            revision: u64::from(receipt.is_some()),
            withdrawn: receipt.is_some(),
            receipt,
            pending_cleanup_run,
            run,
        };
        commit(tx, &self.commit_fence)?;
        Ok(result)
    }
    pub fn preview_remote_withdrawal(
        &self,
        actor: &LocalActor,
        request: &WithdrawalRequest,
    ) -> Result<WithdrawalPreview> {
        validate(request)?;
        let status = self.remote_consent_status(actor)?;
        ensure!(
            status.session_id == request.session_id,
            "withdrawal session mismatch"
        );
        if let Some(receipt) = &status.receipt {
            ensure!(
                receipt.request == *request,
                "dedicated grant already withdrawn by another operation"
            );
        }
        Ok(WithdrawalPreview {
            request: request.clone(),
            confirmation_digest: digest(request, &status.binding)?,
            binding: status.binding,
            already_withdrawn: status.withdrawn,
        })
    }
    /// Independent of the execution lease: an active owner cannot prevent local withdrawal.
    /// The transaction also requests cancellation. A receipt is never cleanup confirmation.
    pub fn withdraw_remote(
        &mut self,
        actor: &LocalActor,
        request: &WithdrawalRequest,
        confirmed: &str,
    ) -> Result<WithdrawalReceipt> {
        self.check_schema()?;
        ensure!(
            self.opened_schema >= 8,
            "explicit quiescent journal upgrade required"
        );
        validate(request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let (session, binding) = local_binding(&tx, actor)?;
        ensure!(session == request.session_id, "withdrawal session mismatch");
        let confirmation_digest = digest(request, &binding)?;
        ensure!(
            confirmed == confirmation_digest,
            "withdrawal confirmation mismatch"
        );
        if let Some(receipt) = read(&tx)? {
            ensure!(
                receipt.request == *request && receipt.binding == binding,
                "withdrawal operation conflict"
            );
            return Ok(receipt);
        }
        let active: Option<Option<String>> = tx
            .query_row(
                "SELECT CASE WHEN length(CAST(id AS BLOB))=36 THEN id END FROM runs WHERE session_id=?1 AND active=1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let cancellation_requested = active
            .map(|id| -> Result<Uuid> {
                Ok(Uuid::parse_str(
                    &id.context("invalid active run identity size")?,
                )?)
            })
            .transpose()?;
        if let Some(run) = cancellation_requested
            && !catalogue::pending(&tx, session, run)?
        {
            // Expiry bounds admission, never resurrects an already requested cancellation.
            let now: i64 = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .try_into()?;
            let expiry = now
                .checked_add(300_000)
                .context("cancellation clock overflow")?;
            tx.execute(
                "INSERT INTO local_cancel_intents VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    run.to_string(),
                    session.to_string(),
                    binding.machine_id.to_string(),
                    binding.owner_id.to_string(),
                    now,
                    expiry
                ],
            )?;
        }
        let latest: i64 = tx.query_row(
            "SELECT next_sequence-1 FROM remote_session WHERE slot=1",
            [],
            |r| r.get(0),
        )?;
        let receipt = WithdrawalReceipt {
            last_public_cursor: latest.try_into()?,
            request: request.clone(),
            binding,
            confirmation_digest,
            revision: 1,
            cancellation_requested,
        };
        tx.execute(
            "INSERT INTO remote_withdrawal VALUES(1,?1)",
            [serde_json::to_string(&receipt)?],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(receipt)
    }
    /// Current persisted grant, checked with the exact binding. Never a reusable permit.
    pub fn check_remote_grant(&self, expected: &RemoteBinding, session: Uuid) -> Result<()> {
        self.check_remote_grant_with(expected, session, || {})
    }
    fn check_remote_grant_with(
        &self,
        expected: &RemoteBinding,
        session: Uuid,
        checked: impl FnOnce(),
    ) -> Result<()> {
        self.check_schema()?;
        ensure!(
            self.opened_schema >= 8,
            "explicit quiescent journal upgrade required"
        );
        let tx = self.connection.unchecked_transaction()?;
        check_transaction_schema(&tx, self.opened_schema)?;
        remote::require(&tx, expected, session)?;
        require_active(&tx)?;
        checked();
        tx.commit()?;
        Ok(())
    }
}
