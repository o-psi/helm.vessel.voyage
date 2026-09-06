//! Private provenance and transactional, content-minimized remote observations.
//! Callers authenticate current authority separately; stored bindings are not grants.
use super::*;
use voyage_protocol::{
    attachment::{Command, Operation},
    events::{EventCursor, RunEvent, SequencedEvent, TerminalState, ToolOutcome},
};

pub(super) const SCHEMA: &str = "CREATE TABLE remote_session(slot INTEGER PRIMARY KEY CHECK(slot=1),session_id TEXT NOT NULL UNIQUE REFERENCES sessions(id),binding TEXT NOT NULL,next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence>0));
CREATE TABLE remote_cleanup_attestations(run_id TEXT PRIMARY KEY REFERENCES runs(id),installation_id TEXT NOT NULL,principal_id TEXT NOT NULL);
CREATE TABLE remote_text(run_id TEXT PRIMARY KEY REFERENCES runs(id),raw_offset INTEGER NOT NULL CHECK(raw_offset>=0));
CREATE TABLE remote_receipts(id TEXT PRIMARY KEY,digest BLOB NOT NULL,result TEXT NOT NULL);
CREATE TABLE remote_events(sequence INTEGER PRIMARY KEY,event TEXT NOT NULL);
CREATE TABLE remote_tools(run_id TEXT NOT NULL REFERENCES runs(id),native_id TEXT NOT NULL,logical_id TEXT PRIMARY KEY,finished INTEGER NOT NULL DEFAULT 0 CHECK(finished IN (0,1))); CREATE UNIQUE INDEX remote_active_tool ON remote_tools(run_id,native_id) WHERE finished=0;";
const MAX_EVENTS: i64 = 1024;
const MAX_PUBLIC_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteBinding {
    pub origin: String,
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
    pub local_installation_id: Uuid,
    pub local_principal_id: Uuid,
}
impl RemoteBinding {
    fn validate(&self) -> Result<()> {
        ensure!(
            crate::attachment::client::validate_origin(&self.origin, true)
                .is_ok_and(|origin| origin == self.origin),
            "invalid remote origin"
        );
        ensure!(
            self.epoch > 0 && self.epoch <= i64::MAX as u64,
            "invalid remote epoch"
        );
        ensure!(
            [
                self.machine_id,
                self.owner_id,
                self.local_installation_id,
                self.local_principal_id
            ]
            .iter()
            .all(|id| !id.is_nil()),
            "invalid remote binding"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteCancelReceipt {
    pub duplicate: bool,
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub state: RunState,
}
pub enum RemoteReplay {
    /// A bounded caller cursor is ahead of this authorized public journal.
    InvalidCursor,
    Events {
        events: Vec<SequencedEvent>,
        latest: u64,
    },
    SnapshotRequired {
        latest: u64,
    },
}

fn binding(db: &Connection) -> Result<Option<(Uuid, RemoteBinding)>> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='remote_session')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        return Ok(None);
    }
    let row: Option<(String, Option<String>)> = db
        .query_row(
            "SELECT substr(session_id,1,37),CASE WHEN length(CAST(binding AS BLOB))<=8192 THEN binding END FROM remote_session WHERE slot=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(id, value)| {
        let value = value.context("invalid remote binding size")?;
        let value: RemoteBinding = serde_json::from_str(&value)?;
        value.validate()?;
        let id = Uuid::parse_str(&id)?;
        ensure!(!id.is_nil(), "invalid remote session");
        Ok((id, value))
    })
    .transpose()
}
pub(super) fn require(db: &Connection, expected: &RemoteBinding, session: Uuid) -> Result<()> {
    expected.validate()?;
    let (id, actual) = binding(db)?.context("remote session unavailable")?;
    ensure!(
        id == session && actual == *expected,
        "remote scope unavailable"
    );
    Ok(())
}
pub(super) fn receipt_exists(db: &Connection, id: Uuid) -> Result<bool> {
    if binding(db)?.is_none() {
        return Ok(false);
    }
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM remote_receipts WHERE id=?1)",
        [id.to_string()],
        |r| r.get(0),
    )?)
}
pub(super) fn validate_admission(db: &Connection, request: &TurnAdmission) -> Result<()> {
    ensure!(
        !receipt_exists(db, request.command_id)?,
        "command identity already reserved"
    );
    if let Some((id, binding)) = binding(db)?
        && id == request.session_id
    {
        ensure!(
            request.machine_id == binding.machine_id && request.principal_id == binding.owner_id,
            "remote run attribution mismatch"
        );
    }
    Ok(())
}
fn publish(tx: &Transaction<'_>, run: Uuid, event: RunEvent) -> Result<()> {
    let sequence: i64 = tx.query_row(
        "SELECT next_sequence FROM remote_session WHERE slot=1",
        [],
        |r| r.get(0),
    )?;
    let next = sequence.checked_add(1).context("remote cursor overflow")?;
    let event = SequencedEvent {
        cursor: EventCursor::new(sequence.try_into()?).map_err(anyhow::Error::msg)?,
        run_id: run,
        event,
    };
    event.validate().map_err(anyhow::Error::msg)?;
    let encoded = serde_json::to_string(&event)?;
    ensure!(encoded.len() <= 128 * 1024, "public event too large");
    tx.execute(
        "INSERT INTO remote_events VALUES(?1,?2)",
        params![sequence, encoded],
    )?;
    tx.execute(
        "UPDATE remote_session SET next_sequence=?1 WHERE slot=1",
        [next],
    )?;
    tx.execute(
        "DELETE FROM remote_events WHERE sequence<=?1",
        [sequence.saturating_sub(MAX_EVENTS)],
    )?;
    // Keep a contiguous suffix when byte pressure evicts more than the count cap.
    loop {
        let bytes: i64 = tx.query_row(
            "SELECT coalesce(sum(length(CAST(event AS BLOB))),0) FROM remote_events",
            [],
            |r| r.get(0),
        )?;
        if bytes <= MAX_PUBLIC_BYTES as i64 {
            break;
        }
        tx.execute(
            "DELETE FROM remote_events WHERE sequence=(SELECT min(sequence) FROM remote_events)",
            [],
        )?;
    }
    Ok(())
}
pub(super) fn observe(
    tx: &Transaction<'_>,
    run: &RunRecord,
    kind: &EventKind,
    redactor: Option<&crate::tools::Redactor>,
) -> Result<()> {
    if !binding(tx)?.is_some_and(|(id, _)| id == run.session_id) {
        return Ok(());
    }
    let revision = || -> Result<u64> {
        let r: i64 = tx.query_row(
            "SELECT revision FROM sessions WHERE id=?1",
            [run.session_id.to_string()],
            |r| r.get(0),
        )?;
        Ok(r.try_into()?)
    };
    let event = match kind {
        EventKind::Accepted => Some(RunEvent::Accepted {
            command_id: run.command_id,
            revision: revision()?,
        }),
        EventKind::Started => Some(RunEvent::Running {}),
        EventKind::CanonicalCheckpoint => Some(RunEvent::CanonicalCheckpoint {
            revision: revision()?,
        }),
        EventKind::TextDelta(_) => {
            if let Some(redactor) = redactor {
                project_text(tx, run, redactor, false)?;
            }
            None
        }
        // Private receipt bookkeeping has no public event. This outbox has its
        // own contiguous session cursor; private Journal cursors never cross wire.
        EventKind::SteeringQueued(_)
        | EventKind::SteeringApplied(_)
        | EventKind::SteeringRejected { .. } => None,
        EventKind::Terminal(state) => {
            if let Some(redactor) = redactor {
                project_text(tx, run, redactor, true)?;
            }
            Some(RunEvent::Terminal {
                state: match state {
                    RunState::Completed => TerminalState::Completed,
                    RunState::Incomplete => TerminalState::Incomplete,
                    RunState::Cancelled => TerminalState::Cancelled,
                    RunState::Failed => TerminalState::Failed,
                    RunState::Interrupted => TerminalState::Interrupted,
                    _ => anyhow::bail!("nonterminal public outcome"),
                },
            })
        }
    };
    if let Some(event) = event {
        publish(tx, run.id, event)?;
    }
    Ok(())
}
fn project_text(
    tx: &Transaction<'_>,
    run: &RunRecord,
    redactor: &crate::tools::Redactor,
    flush: bool,
) -> Result<()> {
    let offset: Option<i64> = tx
        .query_row(
            "SELECT raw_offset FROM remote_text WHERE run_id=?1",
            [run.id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let offset: usize = offset.unwrap_or(0).try_into()?;
    let pending = run
        .partial_text
        .get(offset..)
        .context("invalid public text offset")?;
    let consumed = redactor.stable_prefix(pending, flush);
    let output = redactor.redact_public_prefix(&pending[..consumed]);
    let mut remaining = output.as_str();
    while !remaining.is_empty() {
        let mut end = remaining.len().min(16384);
        while !remaining.is_char_boundary(end) {
            end -= 1;
        }
        publish(
            tx,
            run.id,
            RunEvent::TextDelta {
                text: remaining[..end].to_owned(),
            },
        )?;
        remaining = &remaining[end..];
    }
    let next: i64 = offset
        .checked_add(consumed)
        .context("public text offset overflow")?
        .try_into()?;
    tx.execute("INSERT INTO remote_text(run_id,raw_offset) VALUES(?1,?2) ON CONFLICT(run_id) DO UPDATE SET raw_offset=excluded.raw_offset",params![run.id.to_string(),next])?;
    Ok(())
}
pub(super) fn cleanup(tx: &Transaction<'_>, run_id: Uuid, confirmation: &str) -> Result<()> {
    let run = read_run(tx, run_id)?;
    if !binding(tx)?.is_some_and(|(id, _)| id == run.session_id) {
        return Ok(());
    }
    let state = match confirmation {
        "observed" => voyage_protocol::events::CleanupState::Observed,
        "operator_attested" => voyage_protocol::events::CleanupState::OperatorAttested,
        _ => anyhow::bail!("invalid cleanup confirmation"),
    };
    publish(tx, run_id, RunEvent::Cleanup { state })
}
pub(super) fn canonical(
    tx: &Transaction<'_>,
    run: &RunRecord,
    new: &[Message],
    redactor: Option<&crate::tools::Redactor>,
) -> Result<()> {
    if !binding(tx)?.is_some_and(|(id, _)| id == run.session_id) {
        return Ok(());
    }
    for message in new {
        for call in &message.tool_calls {
            let count: i64 = tx.query_row("SELECT count(*) FROM remote_tools", [], |r| r.get(0))?;
            ensure!(
                count < 100_000 && call.id.len() <= 1024 && !call.id.is_empty(),
                "remote tool identity capacity exceeded"
            );
            let id = Uuid::new_v4();
            tx.execute(
                "INSERT INTO remote_tools(run_id,native_id,logical_id) VALUES(?1,?2,?3)",
                params![run.id.to_string(), call.id, id.to_string()],
            )?;
            publish(
                tx,
                run.id,
                RunEvent::ToolStarted {
                    tool_call_id: id,
                    name: redactor.map_or_else(
                        || "tool".into(),
                        |redactor| redactor.redact_public_prefix(&call.name),
                    ),
                },
            )?;
        }
        if message.role == Role::Tool {
            let native = message
                .tool_call_id
                .as_ref()
                .context("missing tool identity")?;
            let (id, finished): (String, bool) = tx.query_row(
                "SELECT logical_id,finished FROM remote_tools WHERE run_id=?1 AND native_id=?2 AND finished=0",
                params![run.id.to_string(), native],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            ensure!(!finished, "duplicate public tool result");
            tx.execute(
                "UPDATE remote_tools SET finished=1 WHERE run_id=?1 AND native_id=?2 AND finished=0",
                params![run.id.to_string(), native],
            )?;
            publish(
                tx,
                run.id,
                RunEvent::ToolFinished {
                    tool_call_id: Uuid::parse_str(&id)?,
                    outcome: match message.tool_success {
                        Some(true) => ToolOutcome::Succeeded,
                        Some(false) => ToolOutcome::Failed,
                        None => ToolOutcome::Interrupted,
                    },
                },
            )?;
        }
    }
    publish(
        tx,
        run.id,
        RunEvent::Usage {
            input_tokens: run.usage.input_tokens,
            output_tokens: run.usage.output_tokens,
        },
    )?;
    Ok(())
}
impl Journal {
    pub fn remote_local_binding(
        &self,
        actor: &super::super::local_actor::LocalActor,
    ) -> Result<(Uuid, RemoteBinding)> {
        self.check_schema()?;
        let (session, binding) =
            binding(&self.connection)?.context("not a dedicated remote journal")?;
        ensure!(
            binding.local_installation_id == actor.installation_id
                && binding.local_principal_id == actor.principal_id,
            "local installation attribution mismatch"
        );
        Ok((session, binding))
    }
    pub fn attest_remote_cleanup(
        &mut self,
        guard: &ExecutionGuard,
        expected: &RemoteBinding,
        run_id: Uuid,
        actor: &super::super::local_actor::LocalActor,
    ) -> Result<()> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        require(&tx, expected, run.session_id)?;
        ensure!(
            expected.local_installation_id == actor.installation_id
                && expected.local_principal_id == actor.principal_id,
            "local installation attribution mismatch"
        );
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.machine_id == expected.machine_id
                && run.principal_id == expected.owner_id
                && !matches!(run.state, RunState::Accepted | RunState::Running),
            "local attestation requires matching terminal remote run"
        );
        let confirmation: Option<String> = tx.query_row(
            "SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?1",
            [run_id.to_string()],
            |r| r.get(0),
        )?;
        if let Some(confirmation) = confirmation {
            ensure!(
                confirmation == "operator_attested",
                "cleanup evidence is immutable"
            );
            let prior:(String,String)=tx.query_row("SELECT installation_id,principal_id FROM remote_cleanup_attestations WHERE run_id=?1",[run_id.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
            ensure!(
                prior
                    == (
                        actor.installation_id.to_string(),
                        actor.principal_id.to_string()
                    ),
                "attestation provenance mismatch"
            );
            return Ok(());
        }
        tx.execute(
            "INSERT INTO remote_cleanup_attestations VALUES(?1,?2,?3)",
            params![
                run_id.to_string(),
                actor.installation_id.to_string(),
                actor.principal_id.to_string()
            ],
        )?;
        tx.execute("UPDATE local_cleanup_obligations SET confirmation='operator_attested' WHERE run_id=?1 AND confirmation IS NULL",[run_id.to_string()])?;
        cleanup(&tx, run_id, "operator_attested")?;
        tx.commit()?;
        Ok(())
    }
    pub fn configure_remote_redaction(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        redactor: std::sync::Arc<crate::tools::Redactor>,
    ) -> Result<()> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        ensure!(
            run.state == RunState::Accepted,
            "redaction must precede dispatch"
        );
        ensure!(
            binding(&self.connection)?.is_some_and(|(id, _)| id == run.session_id),
            "not a dedicated remote session"
        );
        self.remote_redactor = Some(redactor);
        Ok(())
    }
    pub fn create_remote_session(
        &mut self,
        session: &Session,
        expected: &RemoteBinding,
    ) -> Result<()> {
        ensure!(
            self.opened_schema >= 7,
            "explicit quiescent journal upgrade required"
        );
        expected.validate()?;
        ensure!(
            !session.id.is_nil()
                && session.messages.is_empty()
                && session.completion_runs.is_empty(),
            "remote session must be new and empty"
        );
        let encoded = snapshot(session)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        ensure!(binding(&tx)?.is_none(), "remote session already exists");
        // Dedicated means a pre-existing private journal can never be relabelled.
        let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
        ensure!(
            count == 0,
            "remote session requires an empty dedicated journal"
        );
        tx.execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,0,?2)",
            params![session.id.to_string(), encoded],
        )?;
        tx.execute(
            "INSERT INTO remote_session(slot,session_id,binding) VALUES(1,?1,?2)",
            params![session.id.to_string(), serde_json::to_string(expected)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn remote_session(&self, expected: &RemoteBinding) -> Result<Option<Uuid>> {
        self.check_schema()?;
        expected.validate()?;
        let Some((id, actual)) = binding(&self.connection)? else {
            return Ok(None);
        };
        ensure!(actual == *expected, "remote binding mismatch");
        Ok(Some(id))
    }
    /// Current metadata and run counters from one read transaction. No history or provider payload.
    pub fn remote_snapshot(
        &self,
        expected: &RemoteBinding,
        session: Uuid,
    ) -> Result<voyage_protocol::stream::Reply> {
        use voyage_protocol::{
            events::CleanupState,
            stream::{ExecutionView, Reply, SessionView},
        };
        let tx = self.connection.unchecked_transaction()?;
        check_transaction_schema(&tx, self.opened_schema)?;
        require(&tx, expected, session)?;
        let saved = read_session(&tx, session)?;
        let latest: i64 = tx.query_row(
            "SELECT next_sequence-1 FROM remote_session WHERE slot=1",
            [],
            |r| r.get(0),
        )?;
        let latest_run: Option<String> = tx
            .query_row(
                "SELECT id FROM runs WHERE session_id=?1 ORDER BY rowid DESC LIMIT 1",
                [session.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let run = latest_run
            .map(|id| -> Result<ExecutionView> {
                let run = read_run(&tx, Uuid::parse_str(&id)?)?;
                ensure!(
                    run.session_id == session
                        && run.machine_id == expected.machine_id
                        && run.principal_id == expected.owner_id,
                    "remote run binding mismatch"
                );
                let confirmation: Option<Option<String>> = tx
                    .query_row(
                        "SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?1",
                        [&id],
                        |r| r.get(0),
                    )
                    .optional()?;
                let active = matches!(run.state, RunState::Accepted | RunState::Running);
                let cleanup = match confirmation.as_ref().and_then(|c| c.as_deref()) {
                    Some("observed") => CleanupState::Observed,
                    Some("operator_attested") => CleanupState::OperatorAttested,
                    Some(_) => anyhow::bail!("invalid cleanup confirmation"),
                    None if active => CleanupState::Pending,
                    None => CleanupState::Unconfirmed,
                };
                let state = match run.state {
                    RunState::Accepted => voyage_protocol::stream::RunState::Accepted,
                    RunState::Running => voyage_protocol::stream::RunState::Running,
                    RunState::Completed => voyage_protocol::stream::RunState::Completed,
                    RunState::Incomplete => voyage_protocol::stream::RunState::Incomplete,
                    RunState::Cancelled => voyage_protocol::stream::RunState::Cancelled,
                    RunState::Failed => voyage_protocol::stream::RunState::Failed,
                    RunState::Interrupted => voyage_protocol::stream::RunState::Interrupted,
                };
                Ok(ExecutionView {
                    run_id: run.id,
                    state,
                    cleanup,
                    input_tokens: run.usage.input_tokens,
                    output_tokens: run.usage.output_tokens,
                })
            })
            .transpose()?;
        let reply = Reply::ExecutionSnapshot {
            session: SessionView {
                id: session,
                revision: saved.revision,
                name: saved
                    .session
                    .name
                    .unwrap_or_else(|| "Remote session".into()),
                model: saved.session.model,
                sharing: voyage_protocol::attachment::SharingMode::LiveEvents,
                archived: false,
            },
            run,
            latest: EventCursor::new(latest.try_into()?).map_err(anyhow::Error::msg)?,
        };
        tx.commit()?;
        Ok(reply)
    }
    pub fn remote_replay(
        &self,
        expected: &RemoteBinding,
        session: Uuid,
        after: u64,
        limit: usize,
    ) -> Result<RemoteReplay> {
        ensure!(
            (1..=128).contains(&limit) && after <= i64::MAX as u64,
            "invalid remote replay bounds"
        );
        let tx = self.connection.unchecked_transaction()?;
        check_transaction_schema(&tx, self.opened_schema)?;
        require(&tx, expected, session)?;
        let next: i64 = tx.query_row(
            "SELECT next_sequence FROM remote_session WHERE slot=1",
            [],
            |r| r.get(0),
        )?;
        ensure!(next > 0, "invalid public cursor");
        let latest = next as u64 - 1;
        if after > latest {
            return Ok(RemoteReplay::InvalidCursor);
        }
        let rows=tx.prepare("SELECT sequence,CASE WHEN length(CAST(event AS BLOB))<=131072 THEN event END FROM remote_events WHERE sequence>?1 ORDER BY sequence LIMIT ?2")?.query_map(params![after as i64,limit],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,Option<String>>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        let mut events = Vec::new();
        let mut bytes = 0;
        for (sequence, text) in rows {
            let text = text.context("invalid public event size")?;
            if sequence as u64 != after + events.len() as u64 + 1 {
                return Ok(RemoteReplay::SnapshotRequired { latest });
            }
            let event: SequencedEvent = serde_json::from_str(&text)?;
            event.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                event.cursor.get() == sequence as u64,
                "public cursor mismatch"
            );
            // Leave room for the frame envelope and JSON escaping already encoded.
            if bytes + text.len() > 192 * 1024 {
                break;
            }
            bytes += text.len();
            events.push(event);
        }
        if events.is_empty() && after != latest {
            return Ok(RemoteReplay::SnapshotRequired { latest });
        }
        tx.commit()?;
        Ok(RemoteReplay::Events { events, latest })
    }
    pub fn remote_cancel(
        &mut self,
        expected: &RemoteBinding,
        command: &Command,
    ) -> Result<RemoteCancelReceipt> {
        self.remote_cancel_with_clock(expected, command, || {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .try_into()?)
        })
    }
    pub(crate) fn remote_cancel_with_clock(
        &mut self,
        expected: &RemoteBinding,
        command: &Command,
        clock: impl FnOnce() -> Result<i64>,
    ) -> Result<RemoteCancelReceipt> {
        ensure!(self.opened_schema >= 7, "remote schema unavailable");
        command.validate_structure().map_err(anyhow::Error::msg)?;
        let Operation::Cancel { session_id, run_id } = command.operation else {
            anyhow::bail!("unsupported remote operation")
        };
        let digest = Sha256::digest(command.admission_bytes()?).to_vec();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        require(&tx, expected, session_id)?;
        ensure!(
            command.machine_id == expected.machine_id && command.principal_id == expected.owner_id,
            "remote actor mismatch"
        );
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.session_id == session_id
                && run.machine_id == expected.machine_id
                && run.principal_id == expected.owner_id,
            "remote run unavailable"
        );
        let previous: Option<(Vec<u8>, Option<String>)> = tx
            .query_row(
                "SELECT substr(digest,1,33),CASE WHEN length(CAST(result AS BLOB))<=4096 THEN result END FROM remote_receipts WHERE id=?1",
                [command.command_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old, encoded)) = previous {
            let encoded = encoded.context("remote receipt exceeds limit")?;
            ensure!(
                old == digest && encoded.len() <= 4096,
                "remote receipt mismatch"
            );
            let mut receipt: RemoteCancelReceipt = serde_json::from_str(&encoded)?;
            ensure!(
                receipt.session_id == session_id && receipt.run_id == run_id,
                "remote receipt scope mismatch"
            );
            receipt.duplicate = true;
            return Ok(receipt);
        }
        ensure!(
            !steering::reserved_receipt_exists(&tx, command.command_id)?,
            "remote receipt collision"
        );
        let collision: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM commands WHERE id=?1)",
            [command.command_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(!collision, "remote receipt collision");
        let now = clock()?;
        command.validate(now).map_err(anyhow::Error::msg)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM remote_receipts", [], |r| r.get(0))?;
        ensure!(count < MAX_COMMANDS, "remote receipt capacity reached");
        if matches!(run.state, RunState::Accepted | RunState::Running)
            && !catalogue::pending(&tx, session_id, run_id)?
        {
            tx.execute(
                "INSERT INTO local_cancel_intents VALUES(?1,?2,?3,?4,?5,?6)",
                params![
                    run_id.to_string(),
                    session_id.to_string(),
                    expected.machine_id.to_string(),
                    expected.owner_id.to_string(),
                    now,
                    command.expires_at_ms
                ],
            )?;
            publish(&tx, run_id, RunEvent::CancellationRequested {})?;
        }
        let receipt = RemoteCancelReceipt {
            duplicate: false,
            session_id,
            run_id,
            state: run.state,
        };
        tx.execute(
            "INSERT INTO remote_receipts VALUES(?1,?2,?3)",
            params![
                command.command_id.to_string(),
                digest,
                serde_json::to_string(&receipt)?
            ],
        )?;
        tx.commit()?;
        Ok(receipt)
    }
}
#[cfg(test)]
mod tests;
