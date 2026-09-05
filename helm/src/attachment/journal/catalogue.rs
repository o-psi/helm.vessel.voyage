//! Local installation metadata and cancellation intent. IDs are attribution, not
//! authentication. Callers must hold verified local installation authority; these
//! methods must not be exposed as remote authorization or command receipts.
use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const SCHEMA: &str = "CREATE TABLE local_cancel_intents(run_id TEXT PRIMARY KEY REFERENCES runs(id), session_id TEXT NOT NULL REFERENCES sessions(id), installation_id TEXT NOT NULL, principal_id TEXT NOT NULL, requested_at_ms INTEGER NOT NULL CHECK(requested_at_ms>=0), expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>requested_at_ms));";

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
            .prepare("SELECT id FROM sessions WHERE (?1 IS NULL OR id>?1) ORDER BY id LIMIT ?2")?
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
            sessions.push(SessionSummary {
                id: metadata.id,
                revision: revision.try_into()?,
                name: metadata.name,
                model: metadata.model,
                workspace: metadata.workspace,
                active_run,
            });
        }
        let next_after = more.then(|| sessions.last().expect("nonempty bounded page").id);
        tx.commit()?;
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
        tx.commit()?;
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
        tx.commit()?;
        Ok(result)
    }
}
