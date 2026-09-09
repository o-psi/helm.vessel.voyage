//! Local installation metadata and cancellation intent. IDs are attribution, not
//! authentication. Callers must hold verified local installation authority; these
//! methods must not be exposed as remote authorization or command receipts.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const SCHEMA: &str = "CREATE TABLE local_cancel_intents(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), installation_id TEXT NOT NULL, principal_id TEXT NOT NULL, requested_at_ms INTEGER NOT NULL CHECK(requested_at_ms>=0), expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>requested_at_ms)); CREATE TABLE local_cleanup_obligations(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), installation_id TEXT NOT NULL, principal_id TEXT NOT NULL, confirmation TEXT CHECK(confirmation IN ('observed','operator_attested'))); CREATE UNIQUE INDEX one_pending_cleanup ON local_cleanup_obligations(session_id) WHERE confirmation IS NULL;";

#[derive(Debug, Serialize)]
pub struct SessionPage {
    pub sessions: Vec<SessionSummary>,
    pub next_after: Option<Uuid>,
}
#[derive(Debug, Serialize)]
pub struct SessionSummary {
    pub id: Uuid,
    pub revision: u64,
    pub name: Option<String>,
    pub model: String,
    pub workspace: PathBuf,
    pub active_run: Option<RunSummary>,
    pub pending_cleanup_run: Option<Uuid>,
}
#[derive(Debug, Serialize)]
pub struct RunSummary {
    pub id: Uuid,
    pub state: RunState,
}
#[derive(Clone, Debug)]
pub struct LocalCancelRequest {
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub installation_id: Uuid,
    pub principal_id: Uuid,
    pub expires_at_ms: i64,
}
#[derive(Debug, PartialEq, Eq)]
pub enum CancelRequestOutcome {
    Requested { duplicate: bool },
    AlreadyTerminal { state: RunState },
}

struct Target {
    session_id: Uuid,
    installation_id: Uuid,
    principal_id: Uuid,
    state: RunState,
}
// SQL projects only binding metadata, never provider history or partial text.
fn target(db: &Connection, id: Uuid) -> Result<Target> {
    let (session, active, command, fields): (String, i64, String, String) = db.query_row(
        "SELECT r.session_id,r.active,c.id,json_object('id',json_extract(r.record,'$.id'),'session_id',json_extract(r.record,'$.session_id'),'command_id',json_extract(r.record,'$.command_id'),'machine_id',json_extract(r.record,'$.machine_id'),'principal_id',json_extract(r.record,'$.principal_id'),'state',json_extract(r.record,'$.state')) FROM runs r JOIN commands c ON c.run_id=r.id WHERE r.id=?1 AND length(CAST(r.record AS BLOB))<=?2",
        params![id.to_string(), MAX_PARTIAL * 6 + 4096],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    #[derive(Deserialize)]
    struct Binding {
        id: Uuid,
        session_id: Uuid,
        command_id: Uuid,
        machine_id: Uuid,
        principal_id: Uuid,
        state: RunState,
    }
    ensure!(fields.len() <= 4096, "invalid run metadata");
    let binding: Binding = serde_json::from_str(&fields)?;
    ensure!(
        binding.id == id
            && binding.session_id.to_string() == session
            && binding.command_id.to_string() == command
            && !binding.machine_id.is_nil()
            && !binding.principal_id.is_nil(),
        "invalid run binding"
    );
    ensure!(
        active
            == i64::from(matches!(
                binding.state,
                RunState::Accepted | RunState::Running
            )),
        "invalid run activity"
    );
    Ok(Target {
        session_id: binding.session_id,
        installation_id: binding.machine_id,
        principal_id: binding.principal_id,
        state: binding.state,
    })
}
fn requested(db: &Connection, run_id: Uuid, target: &Target) -> Result<bool> {
    let row: Option<(String, String, String, i64, i64)> = db.query_row(
        "SELECT session_id,installation_id,principal_id,requested_at_ms,expires_at_ms FROM local_cancel_intents WHERE run_id=?1", [run_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    ).optional()?;
    if let Some((session, installation, principal, at, expiry)) = row {
        ensure!(
            session == target.session_id.to_string()
                && installation == target.installation_id.to_string()
                && principal == target.principal_id.to_string()
                && at >= 0
                && expiry > at
                && expiry.checked_sub(at).is_some_and(|ttl| ttl <= 300_000),
            "invalid cancellation binding"
        );
        return Ok(true);
    }
    Ok(false)
}
pub(super) fn pending(db: &Connection, session_id: Uuid, run_id: Uuid) -> Result<bool> {
    let target = target(db, run_id)?;
    ensure!(
        target.session_id == session_id,
        "cancellation session mismatch"
    );
    requested(db, run_id, &target)
}

impl Journal {
    /// Bounded UUID-ordered metadata page. Each page is a consistent read; later
    /// pages may reflect concurrent creation. No transcript is returned or decoded.
    pub fn list_session_summaries(&self, after: Option<Uuid>, limit: usize) -> Result<SessionPage> {
        ensure!((1..=100).contains(&limit), "invalid catalogue page limit");
        let tx = self.connection.unchecked_transaction()?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut ids = tx
            .prepare("SELECT id FROM sessions WHERE (?1 IS NULL OR id>?1) AND coalesce(json_extract(state,'$.format'),'') != 'voyage.managed-retired' ORDER BY id LIMIT ?2")?
            .query_map(params![after.map(|id| id.to_string()), limit + 1], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = ids.len() > limit;
        ids.truncate(limit);
        let mut sessions = Vec::with_capacity(ids.len());
        for id in ids {
            let (revision, metadata): (i64, String) = tx.query_row(
                "SELECT revision,metadata FROM (SELECT revision,json_object('id',json_extract(state,'$.id'),'name',json_extract(state,'$.name'),'model',json_extract(state,'$.model'),'workspace',json_extract(state,'$.workspace')) AS metadata FROM sessions WHERE id=?1 AND length(CAST(state AS BLOB))<=?2) WHERE length(CAST(metadata AS BLOB))<=65536",
                params![id, MAX_SNAPSHOT], |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            #[derive(Deserialize)]
            struct Metadata {
                id: Uuid,
                name: Option<String>,
                model: String,
                workspace: PathBuf,
            }
            ensure!(
                metadata.len() <= 64 * 1024,
                "session metadata exceeds limit"
            );
            let metadata: Metadata = serde_json::from_str(&metadata)?;
            ensure!(
                metadata.id.to_string() == id && !metadata.id.is_nil(),
                "invalid session metadata"
            );
            let active: Option<String> = tx
                .query_row(
                    "SELECT id FROM runs WHERE session_id=?1 AND active=1",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?;
            let active_run = active
                .map(|id| -> Result<RunSummary> {
                    let id = Uuid::parse_str(&id)?;
                    let target = target(&tx, id)?;
                    ensure!(
                        target.session_id == metadata.id,
                        "invalid active run binding"
                    );
                    Ok(RunSummary {
                        id,
                        state: target.state,
                    })
                })
                .transpose()?;
            let pending_cleanup_run = if self.opened_schema >= 5 {
                pending_cleanup(&tx, metadata.id)?
            } else {
                None
            };
            sessions.push(SessionSummary {
                id: metadata.id,
                revision: revision.try_into()?,
                name: metadata.name,
                model: metadata.model,
                workspace: metadata.workspace,
                active_run,
                pending_cleanup_run,
            });
        }
        let next_after = more.then(|| sessions.last().expect("nonempty bounded page").id);
        commit(tx, &self.commit_fence)?;
        Ok(SessionPage {
            sessions,
            next_after,
        })
    }

    /// Commits a local request, not proof that execution stopped. First admission
    /// requires an unexpired <=5 minute deadline. Existing intent is immutable and
    /// remains effective after expiry; retries neither refresh nor resample time.
    pub fn request_cancel_local(
        &mut self,
        request: &LocalCancelRequest,
    ) -> Result<CancelRequestOutcome> {
        self.request_cancel_local_with_clock(request, || {
            Ok(SystemTime::now()
                .duration_since(UNIX_EPOCH)?
                .as_millis()
                .try_into()?)
        })
    }
    pub(crate) fn request_cancel_local_with_clock(
        &mut self,
        request: &LocalCancelRequest,
        clock: impl FnOnce() -> Result<i64>,
    ) -> Result<CancelRequestOutcome> {
        ensure!(
            self.opened_schema >= 5,
            "explicit quiescent journal upgrade required"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let target = target(&tx, request.run_id)?;
        ensure!(
            target.session_id == request.session_id
                && target.installation_id == request.installation_id
                && target.principal_id == request.principal_id,
            "cancellation actor or run mismatch"
        );
        let duplicate = requested(&tx, request.run_id, &target)?;
        if !matches!(target.state, RunState::Accepted | RunState::Running) {
            return Ok(CancelRequestOutcome::AlreadyTerminal {
                state: target.state,
            });
        }
        if duplicate {
            return Ok(CancelRequestOutcome::Requested { duplicate: true });
        }
        let now = clock()?;
        ensure!(
            now >= 0
                && request.expires_at_ms > now
                && request
                    .expires_at_ms
                    .checked_sub(now)
                    .is_some_and(|ttl| ttl <= 300_000),
            "invalid cancellation deadline"
        );
        tx.execute(
            "INSERT INTO local_cancel_intents VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                request.run_id.to_string(),
                request.session_id.to_string(),
                request.installation_id.to_string(),
                request.principal_id.to_string(),
                now,
                request.expires_at_ms
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(CancelRequestOutcome::Requested { duplicate: false })
    }
    /// Observation only. The owning coordinator must cancel and await cleanup;
    /// requested does not mean stopped and this does not transfer its fence.
    pub fn local_cancel_requested(&self, session_id: Uuid, run_id: Uuid) -> Result<bool> {
        ensure!(
            self.opened_schema >= 5,
            "explicit quiescent journal upgrade required"
        );
        let tx = self.connection.unchecked_transaction()?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let result = pending(&tx, session_id, run_id)?;
        commit(tx, &self.commit_fence)?;
        Ok(result)
    }
}

// Validate retained scope even for completed evidence; never silently adopt
// tampered rows as authority to clear or ignore a cleanup obligation.
fn cleanup_confirmation(
    db: &Connection,
    run_id: Uuid,
    target: &Target,
) -> Result<Option<Option<String>>> {
    let row: Option<(String, String, String, Option<String>)> = db.query_row(
        "SELECT session_id,installation_id,principal_id,confirmation FROM local_cleanup_obligations WHERE run_id=?1", [run_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    ).optional()?;
    row.map(|(session, installation, principal, confirmation)| {
        ensure!(
            session == target.session_id.to_string()
                && installation == target.installation_id.to_string()
                && principal == target.principal_id.to_string(),
            "invalid cleanup binding"
        );
        ensure!(
            confirmation
                .as_deref()
                .is_none_or(|value| matches!(value, "observed" | "operator_attested")),
            "invalid cleanup confirmation"
        );
        Ok(confirmation)
    })
    .transpose()
}
pub(super) fn pending_cleanup(db: &Connection, session_id: Uuid) -> Result<Option<Uuid>> {
    let id: Option<Option<String>> = db.query_row("SELECT CASE WHEN length(CAST(run_id AS BLOB))=36 THEN run_id END FROM local_cleanup_obligations WHERE session_id=?1 AND confirmation IS NULL", [session_id.to_string()], |r| r.get(0)).optional()?;
    id.map(|id| {
        let id = Uuid::parse_str(&id.context("invalid pending cleanup identity size")?)?;
        let target = target(db, id)?;
        ensure!(
            target.session_id == session_id && cleanup_confirmation(db, id, &target)? == Some(None),
            "invalid pending cleanup"
        );
        Ok(id)
    })
    .transpose()
}
impl Journal {
    /// Opt in before constructing execution resources. A committed obligation
    /// survives terminalization, recovery, and process death until acknowledged.
    pub fn register_local_cleanup(&mut self, guard: &ExecutionGuard, run_id: Uuid) -> Result<()> {
        ensure!(
            self.opened_schema >= 5,
            "explicit quiescent journal upgrade required"
        );
        let initial_target = target(&self.connection, run_id)?;
        self.check_guard(guard, initial_target.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let target = target(&tx, run_id)?;
        ensure!(
            target.state == RunState::Accepted,
            "cleanup must be registered before dispatch"
        );
        if let Some(confirmation) = cleanup_confirmation(&tx, run_id, &target)? {
            ensure!(confirmation.is_none(), "cleanup evidence is immutable");
            return Ok(());
        }
        ensure!(
            pending_cleanup(&tx, target.session_id)?.is_none(),
            "session cleanup remains unconfirmed"
        );
        tx.execute(
            "INSERT INTO local_cleanup_obligations VALUES(?1,?2,?3,?4,NULL)",
            params![
                run_id.to_string(),
                target.session_id.to_string(),
                target.installation_id.to_string(),
                target.principal_id.to_string()
            ],
        )?;
        commit(tx, &self.commit_fence)?;
        Ok(())
    }
    /// Call only after observing complete provider/tool/child/PTY cleanup. Storage
    /// cannot observe OS effects itself; this API records the guarded owner's claim.
    pub fn confirm_local_cleanup_observed(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
    ) -> Result<()> {
        self.confirm_local_cleanup(guard, run_id, None)
    }
    /// Explicit human attestation that prior effects were independently stopped.
    /// This is retained distinctly from observed cleanup and never implied by recovery.
    pub fn attest_local_cleanup(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        installation_id: Uuid,
        principal_id: Uuid,
    ) -> Result<()> {
        self.confirm_local_cleanup(guard, run_id, Some((installation_id, principal_id)))
    }
    fn confirm_local_cleanup(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        actor: Option<(Uuid, Uuid)>,
    ) -> Result<()> {
        ensure!(
            self.opened_schema >= 5,
            "explicit quiescent journal upgrade required"
        );
        let initial_target = target(&self.connection, run_id)?;
        self.check_guard(guard, initial_target.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let target = target(&tx, run_id)?;
        ensure!(
            !matches!(target.state, RunState::Accepted | RunState::Running),
            "cleanup confirmation requires terminal run"
        );
        if let Some((installation, principal)) = actor {
            ensure!(
                installation == target.installation_id && principal == target.principal_id,
                "cleanup attestation actor mismatch"
            );
        }
        if super::assignments::pending(&tx, run_id)? > 0 {
            if actor.is_none() {
                tx.execute(
                    "INSERT OR IGNORE INTO process_assignment_local_cleanup VALUES(?1)",
                    [run_id.to_string()],
                )?;
                commit(tx, &self.commit_fence)?;
            }
            anyhow::bail!("participant cleanup remains pending; local observation retained");
        }
        let expected = if actor.is_some() {
            "operator_attested"
        } else {
            "observed"
        };
        let confirmation = cleanup_confirmation(&tx, run_id, &target)?
            .context("cleanup obligation not registered")?;
        if let Some(prior) = confirmation {
            ensure!(prior == expected, "cleanup evidence is immutable");
            return Ok(());
        }
        tx.execute("UPDATE local_cleanup_obligations SET confirmation=?1 WHERE run_id=?2 AND confirmation IS NULL", params![expected, run_id.to_string()])?;
        commit(tx, &self.commit_fence)?;
        Ok(())
    }
}
