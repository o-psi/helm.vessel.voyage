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

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (
        tempfile::TempDir,
        Journal,
        LocalActor,
        Session,
        RemoteBinding,
        WithdrawalRequest,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(dir.path().join("journal")).unwrap();
        let actor = LocalActor {
            installation_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
        };
        let session = Session::new(dir.path().into(), "fixture".into());
        let binding = RemoteBinding {
            origin: "http://127.0.0.1:9000".into(),
            machine_id: Uuid::new_v4(),
            owner_id: Uuid::new_v4(),
            epoch: 1,
            local_installation_id: actor.installation_id,
            local_principal_id: actor.principal_id,
        };
        journal.create_remote_session(&session, &binding).unwrap();
        let request = WithdrawalRequest {
            session_id: session.id,
            operation_id: Uuid::new_v4(),
            expected_revision: 0,
        };
        (dir, journal, actor, session, binding, request)
    }
    fn admission(session: &Session, binding: &RemoteBinding) -> TurnAdmission {
        TurnAdmission {
            command_id: Uuid::new_v4(),
            machine_id: binding.machine_id,
            principal_id: binding.owner_id,
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: 10000,
            prompt: "PRIVATE_TASK".into(),
        }
    }
    #[test]
    fn exact_confirmation_scope_permanent_retirement_and_restart_receipt() {
        let (dir, mut journal, actor, session, binding, request) = fixture();
        let original = journal.load_session(session.id).unwrap().session;
        let preview = journal.preview_remote_withdrawal(&actor, &request).unwrap();
        let wrong = LocalActor {
            principal_id: Uuid::new_v4(),
            ..actor
        };
        assert!(journal.preview_remote_withdrawal(&wrong, &request).is_err());
        assert!(journal.withdraw_remote(&actor, &request, "wrong").is_err());
        for changed in [
            WithdrawalRequest {
                session_id: Uuid::new_v4(),
                ..request.clone()
            },
            WithdrawalRequest {
                operation_id: Uuid::new_v4(),
                ..request.clone()
            },
            WithdrawalRequest {
                expected_revision: 1,
                ..request.clone()
            },
        ] {
            assert!(
                journal
                    .withdraw_remote(&actor, &changed, &preview.confirmation_digest)
                    .is_err()
            );
        }
        assert!(!journal.remote_consent_status(&actor).unwrap().withdrawn);
        let receipt = journal
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap();
        assert_eq!(receipt.cancellation_requested, None);
        assert!(journal.check_remote_grant(&binding, session.id).is_err());
        assert!(journal.remote_snapshot(&binding, session.id).is_err());
        assert!(journal.remote_replay(&binding, session.id, 0, 4).is_err());
        let guard = journal.acquire_execution(session.id).unwrap();
        assert!(
            journal
                .admit_turn(&guard, &admission(&session, &binding), 1)
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap(),
            serde_json::to_value(original).unwrap()
        );
        drop(guard);
        drop(journal);
        let mut reopened = Journal::open(dir.path().join("journal")).unwrap();
        assert_eq!(
            reopened
                .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                .unwrap(),
            receipt
        );
        assert!(
            reopened
                .preview_remote_withdrawal(&actor, &request)
                .unwrap()
                .already_withdrawn
        );
        assert!(
            reopened
                .preview_remote_withdrawal(
                    &actor,
                    &WithdrawalRequest {
                        operation_id: Uuid::new_v4(),
                        ..request
                    }
                )
                .is_err()
        );
    }
    #[test]
    fn active_withdrawal_fences_publication_but_preserves_canonical_and_cleanup() {
        let (_dir, mut journal, actor, session, binding, request) = fixture();
        let guard = journal.acquire_execution(session.id).unwrap();
        let command = admission(&session, &binding);
        let run = journal.admit_turn(&guard, &command, 1).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        let before: i64 = journal
            .connection
            .query_row("SELECT next_sequence FROM remote_session", [], |r| r.get(0))
            .unwrap();
        let preview = journal.preview_remote_withdrawal(&actor, &request).unwrap();
        let receipt = journal
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap();
        assert_eq!(receipt.cancellation_requested, Some(run.id));
        assert!(journal.local_cancel_requested(session.id, run.id).unwrap());
        assert!(
            journal.lookup_command(&command).is_err(),
            "remote duplicate receipt also requires current disclosure authority"
        );
        let mut messages = journal.load_session(session.id).unwrap().session.messages;
        messages.push(Message::new(Role::Assistant, "AFTER_WITHDRAWAL_PRIVATE"));
        // Completion cancellation may refuse acceptance, but failure/recovery must retain canonical text.
        journal
            .checkpoint_canonical(
                &guard,
                run.id,
                &messages,
                &Usage {
                    input_tokens: 4,
                    output_tokens: 2,
                },
            )
            .unwrap();
        let after: i64 = journal
            .connection
            .query_row("SELECT next_sequence FROM remote_session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            journal
                .load_session(session.id)
                .unwrap()
                .session
                .messages
                .last()
                .unwrap()
                .content,
            "AFTER_WITHDRAWAL_PRIVATE"
        );
        assert_eq!(
            journal
                .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                .unwrap(),
            receipt
        );
    }
    #[test]
    fn failed_transaction_does_not_withdraw_or_leave_cancellation() {
        let (_dir, mut journal, actor, session, binding, request) = fixture();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal
            .admit_turn(&guard, &admission(&session, &binding), 1)
            .unwrap()
            .run;
        let preview = journal.preview_remote_withdrawal(&actor, &request).unwrap();
        journal.connection.execute_batch("CREATE TRIGGER fail_withdraw BEFORE INSERT ON remote_withdrawal BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(
            journal
                .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                .is_err()
        );
        assert!(!journal.remote_consent_status(&actor).unwrap().withdrawn);
        assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
        journal.check_remote_grant(&binding, session.id).unwrap();
        journal
            .connection
            .execute_batch("DROP TRIGGER fail_withdraw")
            .unwrap();
        journal
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap();
    }
    #[test]
    fn malformed_evidence_fails_closed_and_is_preserved() {
        let (_dir, mut journal, actor, session, binding, request) = fixture();
        journal
            .connection
            .execute("INSERT INTO remote_withdrawal VALUES(1,?1)", ["{broken"])
            .unwrap();
        assert!(journal.remote_consent_status(&actor).is_err());
        assert!(journal.check_remote_grant(&binding, session.id).is_err());
        assert!(journal.withdraw_remote(&actor, &request, "x").is_err());
        let stored: String = journal
            .connection
            .query_row("SELECT receipt FROM remote_withdrawal", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, "{broken");
    }
    #[test]
    fn schema_seven_upgrade_is_quiescent_and_preserves_binding_and_history() {
        let (dir, journal, actor, session, binding, _request) = fixture();
        journal
            .connection
            .execute_batch("DROP TABLE remote_withdrawal; UPDATE attachment_schema SET version=7")
            .unwrap();
        drop(journal);
        let mut old = Journal::open(dir.path().join("journal")).unwrap();
        assert!(old.check_remote_grant(&binding, session.id).is_err());
        let guard = old.acquire_execution(session.id).unwrap();
        let mut current = Journal::open(dir.path().join("journal")).unwrap();
        assert!(current.upgrade_quiescent().is_err());
        drop(guard);
        current.upgrade_quiescent().unwrap();
        assert!(
            old.create_session(&Session::new(dir.path().into(), "x".into()))
                .is_err()
        );
        assert!(!current.remote_consent_status(&actor).unwrap().withdrawn);
        current.check_remote_grant(&binding, session.id).unwrap();
    }
    #[test]
    fn competing_transaction_cannot_split_withdrawal_from_admission_or_publication() {
        let (dir, mut journal, actor, session, binding, request) = fixture();
        let mut other = Journal::open(dir.path().join("journal")).unwrap();
        let preview = other.preview_remote_withdrawal(&actor, &request).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        let command = admission(&session, &binding);
        let run = journal
            .admit_turn_with_clock(&guard, &command, || {
                // Admission already owns SQLite's write transaction. Withdrawal cannot partly commit.
                assert!(
                    other
                        .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                        .is_err()
                );
                Ok(1)
            })
            .unwrap()
            .run;
        journal.mark_running(&guard, run.id).unwrap();
        let before: i64 = journal
            .connection
            .query_row("SELECT next_sequence FROM remote_session", [], |r| r.get(0))
            .unwrap();
        other
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap();
        journal
            .append_text(&guard, run.id, "LATE_PRIVATE_TEXT")
            .unwrap();
        assert_eq!(
            journal.run(run.id).unwrap().partial_text,
            "LATE_PRIVATE_TEXT"
        );
        let after: i64 = journal
            .connection
            .query_row("SELECT next_sequence FROM remote_session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        assert!(journal.admit_turn(&guard, &command, 1).is_err());
        assert!(journal.check_remote_grant(&binding, session.id).is_err());
        assert!(journal.remote_replay(&binding, session.id, 0, 4).is_err());
    }
    #[test]
    fn terminal_busy_retry_after_withdrawal_keeps_public_cursor_frozen() {
        let (dir, mut journal, actor, session, binding, request) = fixture();
        let other = Journal::open(dir.path().join("journal")).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal
            .admit_turn(&guard, &admission(&session, &binding), 1)
            .unwrap()
            .run;
        journal.mark_running(&guard, run.id).unwrap();
        let preview = journal.preview_remote_withdrawal(&actor, &request).unwrap();
        let receipt = journal
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap();
        let before: i64 = journal
            .connection
            .query_row("SELECT next_sequence FROM remote_session", [], |r| r.get(0))
            .unwrap();
        other
            .connection
            .execute_batch("BEGIN; SELECT * FROM runs")
            .unwrap();
        let error = journal
            .finish_classified(
                &guard,
                run.id,
                RunState::Failed,
                Some("provider failed"),
                None,
                None,
            )
            .unwrap_err();
        assert!(journal.terminal_retry_safe(&error));
        assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
        other.connection.execute_batch("ROLLBACK").unwrap();
        let actual = journal
            .finish_classified(
                &guard,
                run.id,
                RunState::Failed,
                Some("provider failed"),
                None,
                None,
            )
            .unwrap();
        assert_eq!(actual.state, RunState::Cancelled);
        let after: i64 = journal
            .connection
            .query_row("SELECT next_sequence FROM remote_session", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            journal
                .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                .unwrap(),
            receipt
        );
        assert!(journal.remote_replay(&binding, session.id, 0, 4).is_err());
    }

    #[test]
    fn commit_busy_rolls_back_then_original_confirmation_recovers_once() {
        let (dir, mut journal, actor, session, binding, request) = fixture();
        let other = Journal::open(dir.path().join("journal")).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal
            .admit_turn(&guard, &admission(&session, &binding), 1)
            .unwrap()
            .run;
        let preview = journal.preview_remote_withdrawal(&actor, &request).unwrap();
        other
            .connection
            .execute_batch("BEGIN; SELECT * FROM remote_session;")
            .unwrap();
        let error = journal
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap_err();
        assert!(error.chain().any(|cause| matches!(cause.downcast_ref::<rusqlite::Error>(),Some(rusqlite::Error::SqliteFailure(code,_)) if code.code==rusqlite::ErrorCode::DatabaseBusy)));
        other.connection.execute_batch("ROLLBACK").unwrap();
        assert!(!journal.remote_consent_status(&actor).unwrap().withdrawn);
        assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
        let receipt = journal
            .withdraw_remote(&actor, &request, &preview.confirmation_digest)
            .unwrap();
        assert_eq!(
            journal
                .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                .unwrap(),
            receipt
        );
        let count: i64 = journal
            .connection
            .query_row("SELECT count(*) FROM remote_withdrawal", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn oversized_run_cleanup_and_receipt_metadata_fail_before_materialization_without_repair() {
        for field in ["run", "cleanup", "pending", "receipt"] {
            let (_dir, mut journal, actor, session, binding, request) = fixture();
            let guard = journal.acquire_execution(session.id).unwrap();
            let run = journal
                .admit_turn(&guard, &admission(&session, &binding), 1)
                .unwrap()
                .run;
            journal.register_local_cleanup(&guard, run.id).unwrap();
            let preview = journal.preview_remote_withdrawal(&actor, &request).unwrap();
            let oversized = "x".repeat(65536);
            journal
                .connection
                .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
                .unwrap();
            let (table, column) = match field {
                "run" => ("runs", "id"),
                "cleanup" => ("local_cleanup_obligations", "confirmation"),
                "pending" => ("local_cleanup_obligations", "run_id"),
                _ => {
                    journal
                        .connection
                        .execute("INSERT INTO remote_withdrawal VALUES(1,?1)", [&oversized])
                        .unwrap();
                    ("remote_withdrawal", "receipt")
                }
            };
            if field != "receipt" {
                journal
                    .connection
                    .execute(&format!("UPDATE {table} SET {column}=?1"), [&oversized])
                    .unwrap();
            }
            assert!(journal.remote_consent_status(&actor).is_err(), "{field}");
            // Withdrawal must fail on malformed active scope/evidence, not overwrite it.
            if field == "run" || field == "receipt" {
                assert!(
                    journal
                        .withdraw_remote(&actor, &request, &preview.confirmation_digest)
                        .is_err()
                );
            }
            let retained: String = journal
                .connection
                .query_row(&format!("SELECT {column} FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(retained, oversized);
        }
    }
    #[test]
    fn grant_observer_cannot_make_its_own_owner_commit_fail_busy() {
        observer_commit_case(false);
        observer_commit_case(true);
    }
    fn observer_commit_case(large: bool) {
        use std::sync::mpsc;
        let (_dir, mut journal, _actor, session, binding, _request) = fixture();
        if large {
            journal
                .connection
                .pragma_update(None, "cache_size", 1)
                .unwrap();
            journal
                .connection
                .execute_batch("PRAGMA main.cache_spill=1;")
                .unwrap();
        }
        let observer = journal
            .remote_grant_observer(binding.clone(), session.id)
            .unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        let mut command = admission(&session, &binding);
        if large {
            command.prompt = "x".repeat(MAX_PROMPT);
        }
        let (observed_tx, observed_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            observer.check_with(|| {
                observed_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            })
        });
        observed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (committing_tx, committing_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            let result = journal.admit_turn_with_clock(&guard, &command, || {
                committing_tx.send(()).unwrap();
                Ok(1)
            });
            result_tx
                .send(result.map(|admission| admission.run.id))
                .unwrap();
        });
        committing_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let early = result_rx.recv_timeout(Duration::from_millis(50));
        release_tx.send(()).unwrap();
        reader.join().unwrap().unwrap();
        writer.join().unwrap();
        assert!(
            matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
            "owner COMMIT must wait for its own observer, not fail: {early:?}"
        );
        assert!(
            result_rx
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .is_ok()
        );
    }
    #[test]
    fn large_owner_write_does_not_deny_its_current_grant_before_commit() {
        let (_dir, mut journal, _actor, session, binding, _request) = fixture();
        journal
            .connection
            .execute_batch(
                "PRAGMA cache_size=1; CREATE TABLE observer_spill_fixture(payload BLOB);",
            )
            .unwrap();
        let observer = journal.remote_grant_observer(binding, session.id).unwrap();
        let tx = journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        tx.execute(
            "INSERT INTO observer_spill_fixture VALUES(zeroblob(4194304))",
            [],
        )
        .unwrap();
        let observed = observer.check();
        tx.rollback().unwrap();
        assert!(
            observed.is_ok(),
            "unchanged grant denied while its owner has dirty pages: {observed:?}"
        );
    }
}
