//! Run-bound steering evidence, separate from the immutable canonical transcript.
use super::*;
use crate::model::{SteeringReceipt, SteeringStatus};

pub const MAX_PENDING_STEERING: usize = 64;
pub const MAX_STEERING_PER_RUN: usize = 1024;
pub(super) const SCHEMA: &str = "CREATE TABLE steering(id TEXT PRIMARY KEY,run_id TEXT NOT NULL REFERENCES runs(id),session_id TEXT NOT NULL REFERENCES sessions(id),ordinal INTEGER NOT NULL CHECK(ordinal>0),machine_id TEXT NOT NULL,principal_id TEXT NOT NULL,digest BLOB NOT NULL,record TEXT NOT NULL,status TEXT NOT NULL CHECK(status IN ('queued','applied','not_applied')),UNIQUE(run_id,ordinal));";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteeringActor {
    pub machine_id: Uuid,
    pub principal_id: Uuid,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteeringAdmission {
    /// Stable idempotency key AND receipt identity. Cannot collide with turn IDs.
    pub receipt_id: Uuid,
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub actor: SteeringActor,
    pub expected_revision: u64,
    pub expires_at_ms: i64,
    pub text: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SteeringRejection {
    QueueFull,
    Closed,
    Cancelled,
    Interrupted,
    RunFailed,
    RunCompleted,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SteeringRecord {
    pub request: SteeringAdmission,
    pub ordinal: u64,
    /// Session revision which last changed this receipt, not an execution claim.
    pub revision: u64,
    pub status: SteeringStatus,
    pub reason: Option<SteeringRejection>,
    pub canonical_index: Option<usize>,
}
impl std::fmt::Debug for SteeringRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SteeringRecord")
            .field("id", &self.request.receipt_id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
impl SteeringRecord {
    fn message(&self, status: SteeringStatus) -> Message {
        let mut message = Message::new(Role::User, &self.request.text);
        message.steering = Some(SteeringReceipt {
            id: self.request.receipt_id,
            status,
        });
        message
    }
    pub fn applied_message(&self) -> Message {
        self.message(SteeringStatus::Applied)
    }
    pub(crate) fn queued_message(&self) -> Message {
        self.message(SteeringStatus::Queued)
    }
}
pub struct SteeringOutcome {
    pub duplicate: bool,
    pub record: SteeringRecord,
}
fn status(value: &SteeringStatus) -> Result<&'static str> {
    match value {
        SteeringStatus::Queued => Ok("queued"),
        SteeringStatus::Applied => Ok("applied"),
        SteeringStatus::NotApplied => Ok("not_applied"),
        SteeringStatus::UnknownAfterRestart => anyhow::bail!("invalid journal steering status"),
    }
}
fn validate(request: &SteeringAdmission) -> Result<()> {
    ensure!(
        !request.receipt_id.is_nil()
            && !request.session_id.is_nil()
            && !request.run_id.is_nil()
            && !request.actor.machine_id.is_nil()
            && !request.actor.principal_id.is_nil(),
        "nil steering identity"
    );
    ensure!(
        !request.text.trim().is_empty() && request.text.len() <= crate::agent::MAX_STEERING_BYTES,
        "invalid steering size"
    );
    Ok(())
}
fn read(db: &Connection, id: Uuid) -> Result<SteeringRecord> {
    let (json, stored_status, run, session, ordinal, machine, principal, digest): (String,String,String,String,i64,String,String,Vec<u8>) = db.query_row(
        "SELECT record,status,run_id,session_id,ordinal,machine_id,principal_id,digest FROM steering WHERE id=?1", [id.to_string()],
        |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?)))?;
    let record: SteeringRecord = serde_json::from_str(&json)?;
    validate(&record.request)?;
    ensure!(
        record.request.receipt_id == id
            && record.request.run_id.to_string() == run
            && record.request.session_id.to_string() == session
            && record.request.actor.machine_id.to_string() == machine
            && record.request.actor.principal_id.to_string() == principal
            && i64::try_from(record.ordinal)? == ordinal
            && record.ordinal > 0
            && record.revision > 0
            && status(&record.status)? == stored_status
            && Sha256::digest(serde_json::to_vec(&record.request)?).as_slice() == digest,
        "corrupt steering evidence"
    );
    ensure!(
        match record.status {
            SteeringStatus::Queued => record.reason.is_none() && record.canonical_index.is_none(),
            SteeringStatus::Applied => record.reason.is_none() && record.canonical_index.is_some(),
            SteeringStatus::NotApplied =>
                record.reason.is_some() && record.canonical_index.is_none(),
            _ => false,
        },
        "corrupt steering transition"
    );
    Ok(record)
}
fn write(tx: &Transaction<'_>, record: &SteeringRecord) -> Result<()> {
    ensure!(
        tx.execute(
            "UPDATE steering SET record=?1,status=?2 WHERE id=?3",
            params![
                serde_json::to_string(record)?,
                status(&record.status)?,
                record.request.receipt_id.to_string()
            ]
        )? == 1,
        "missing steering receipt"
    );
    Ok(())
}
pub(super) fn receipt_exists(db: &Connection, id: Uuid) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM steering WHERE id=?1)",
        [id.to_string()],
        |r| r.get(0),
    )?)
}
impl Journal {
    /// Caller must freshly authorize the actor for this run before each call.
    pub fn queue_steering(
        &mut self,
        guard: &ExecutionGuard,
        request: &SteeringAdmission,
        now_ms: i64,
    ) -> Result<SteeringOutcome> {
        self.queue_steering_with_clock(guard, request, || Ok(now_ms))
    }
    pub(crate) fn queue_steering_with_clock(
        &mut self,
        guard: &ExecutionGuard,
        request: &SteeringAdmission,
        clock: impl FnOnce() -> Result<i64>,
    ) -> Result<SteeringOutcome> {
        self.check_guard(guard, request.session_id)?;
        ensure!(
            self.opened_schema == SCHEMA_VERSION,
            "steering requires quiescent journal upgrade"
        );
        validate(request)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let command: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1)",
            [request.receipt_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!command, "receipt identity collides with a turn command");
        if receipt_exists(&tx, request.receipt_id)? {
            let record = read(&tx, request.receipt_id)?;
            ensure!(
                serde_json::to_vec(&record.request)? == serde_json::to_vec(request)?,
                "steering identity reused with changed payload or authority"
            );
            return Ok(SteeringOutcome {
                duplicate: true,
                record,
            });
        }
        let now_ms = clock()?;
        ensure!(
            now_ms >= 0
                && request.expires_at_ms > now_ms
                && request.expires_at_ms
                    <= now_ms.checked_add(300_000).context("clock overflow")?,
            "steering expired or deadline invalid"
        );
        let run = read_run(&tx, request.run_id)?;
        ensure!(
            run.session_id == request.session_id
                && matches!(run.state, RunState::Accepted | RunState::Running),
            "steering requires active matching run"
        );
        let current = read_session(&tx, request.session_id)?;
        ensure!(
            current.revision == request.expected_revision,
            "stale session revision"
        );
        let (total, pending): (i64, i64) = tx.query_row(
            "SELECT count(*),coalesce(sum(status='queued'),0) FROM steering WHERE run_id=?1",
            [run.id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            total < MAX_STEERING_PER_RUN as i64 && pending < MAX_PENDING_STEERING as i64,
            "steering queue/evidence capacity reached"
        );
        let count: i64 = tx.query_row("SELECT count(*) FROM steering", [], |r| r.get(0))?;
        ensure!(count < MAX_COMMANDS, "steering evidence capacity reached");
        let record = SteeringRecord {
            request: request.clone(),
            ordinal: u64::try_from(total)?
                .checked_add(1)
                .context("ordinal overflow")?,
            revision: current
                .revision
                .checked_add(1)
                .context("revision overflow")?,
            status: SteeringStatus::Queued,
            reason: None,
            canonical_index: None,
        };
        tx.execute(
            "INSERT INTO steering VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'queued')",
            params![
                request.receipt_id.to_string(),
                run.id.to_string(),
                run.session_id.to_string(),
                record.ordinal,
                request.actor.machine_id.to_string(),
                request.actor.principal_id.to_string(),
                Sha256::digest(serde_json::to_vec(request)?).to_vec(),
                serde_json::to_string(&record)?
            ],
        )?;
        update_session(&tx, &current)?;
        append_event(&tx, &run, EventKind::SteeringQueued(request.receipt_id))?;
        tx.commit()?;
        Ok(SteeringOutcome {
            duplicate: false,
            record,
        })
    }
    pub fn steering_record(&self, id: Uuid) -> Result<SteeringRecord> {
        self.check_schema()?;
        ensure!(
            self.opened_schema == SCHEMA_VERSION,
            "steering requires quiescent journal upgrade"
        );
        read(&self.connection, id)
    }
    /// Bounded local receipt projection. This is not an outbound authorization API.
    pub fn steering_page(
        &self,
        run_id: Uuid,
        after: u64,
        limit: usize,
    ) -> Result<Vec<SteeringRecord>> {
        self.check_schema()?;
        ensure!(
            self.opened_schema == SCHEMA_VERSION && (1..=MAX_PENDING_STEERING).contains(&limit),
            "invalid steering projection bound/schema"
        );
        let ids = self
            .connection
            .prepare(
                "SELECT id FROM steering WHERE run_id=?1 AND ordinal>?2 ORDER BY ordinal LIMIT ?3",
            )?
            .query_map(
                params![run_id.to_string(), i64::try_from(after)?, limit as i64],
                |r| r.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| read(&self.connection, Uuid::parse_str(&id)?))
            .collect()
    }
    pub fn reject_steering(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        id: Uuid,
        reason: SteeringRejection,
    ) -> Result<SteeringRecord> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        ensure!(
            self.opened_schema == SCHEMA_VERSION,
            "steering requires quiescent journal upgrade"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut record = read(&tx, id)?;
        ensure!(
            record.request.run_id == run_id && record.request.session_id == run.session_id,
            "receipt belongs to another run"
        );
        if record.status == SteeringStatus::NotApplied {
            ensure!(
                record.reason == Some(reason),
                "rejection outcome is immutable"
            );
            return Ok(record);
        }
        ensure!(
            record.status == SteeringStatus::Queued
                && matches!(run.state, RunState::Accepted | RunState::Running),
            "receipt cannot be rejected"
        );
        let current = read_session(&tx, run.session_id)?;
        record.status = SteeringStatus::NotApplied;
        record.reason = Some(reason);
        record.revision = current
            .revision
            .checked_add(1)
            .context("revision overflow")?;
        write(&tx, &record)?;
        update_session(&tx, &current)?;
        append_event(&tx, &run, EventKind::SteeringRejected { id, reason })?;
        tx.commit()?;
        Ok(record)
    }
    pub(crate) fn steering_actors(&self, run_id: Uuid) -> Result<Vec<SteeringActor>> {
        if self.opened_schema < SCHEMA_VERSION {
            return Ok(Vec::new());
        }
        let actors = self.connection.prepare("SELECT DISTINCT machine_id,principal_id FROM steering WHERE run_id=?1 AND status IN ('queued','applied')")?.query_map([run_id.to_string()], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        actors
            .into_iter()
            .map(|(machine, principal)| {
                Ok(SteeringActor {
                    machine_id: Uuid::parse_str(&machine)?,
                    principal_id: Uuid::parse_str(&principal)?,
                })
            })
            .collect()
    }
}
pub(super) fn apply(
    tx: &Transaction<'_>,
    run: &RunRecord,
    appended: &[Message],
    offset: usize,
    revision: u64,
    now_ms: i64,
) -> Result<()> {
    for (index, message) in appended.iter().enumerate() {
        let Some(receipt) = &message.steering else {
            continue;
        };
        let mut record = read(tx, receipt.id)?;
        ensure!(
            record.request.run_id == run.id
                && record.request.session_id == run.session_id
                && record.status == SteeringStatus::Queued,
            "unknown, stale or duplicate steering receipt"
        );
        ensure!(
            now_ms >= 0 && record.request.expires_at_ms > now_ms,
            "steering expired before application"
        );
        ensure!(
            serde_json::to_vec(message)? == serde_json::to_vec(&record.applied_message())?,
            "steering canonical message changed"
        );
        let first: String = tx.query_row(
            "SELECT id FROM steering WHERE run_id=?1 AND status='queued' ORDER BY ordinal LIMIT 1",
            [run.id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            first == receipt.id.to_string(),
            "steering applied out of FIFO order"
        );
        record.status = SteeringStatus::Applied;
        record.canonical_index = Some(
            offset
                .checked_add(index)
                .context("canonical index overflow")?,
        );
        record.revision = revision;
        write(tx, &record)?;
        append_event(tx, run, EventKind::SteeringApplied(receipt.id))?;
    }
    Ok(())
}
pub(super) fn ensure_no_pending(db: &Connection, run_id: Uuid) -> Result<()> {
    let pending: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM steering WHERE run_id=?1 AND status='queued')",
        [run_id.to_string()],
        |r| r.get(0),
    )?;
    ensure!(!pending, "cannot accept completion with unapplied steering");
    Ok(())
}
pub(super) fn settle(
    tx: &Transaction<'_>,
    run: &RunRecord,
    state: &RunState,
    revision: u64,
) -> Result<()> {
    let reason = match state {
        RunState::Cancelled => SteeringRejection::Cancelled,
        RunState::Interrupted => SteeringRejection::Interrupted,
        RunState::Completed | RunState::Incomplete => SteeringRejection::RunCompleted,
        _ => SteeringRejection::RunFailed,
    };
    let ids = tx
        .prepare("SELECT id FROM steering WHERE run_id=?1 AND status='queued' ORDER BY ordinal")?
        .query_map([run.id.to_string()], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for id in ids {
        let id = Uuid::parse_str(&id)?;
        let mut record = read(tx, id)?;
        record.status = SteeringStatus::NotApplied;
        record.reason = Some(reason);
        record.revision = revision;
        write(tx, &record)?;
        append_event(tx, run, EventKind::SteeringRejected { id, reason })?;
    }
    Ok(())
}
