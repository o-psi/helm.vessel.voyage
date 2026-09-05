//! Atomic admission and recovery for attachment-owned sessions.
//!
//! This is not an authorization layer or a replacement for SessionStore yet. The
//! caller must authorize the authenticated principal and sharing on every request.
//! Never import local sessions implicitly. Hold ExecutionGuard through provider /
//! tool execution and its cleanup. Commit accepted input before starting effects.
//! All mutators use SQLite transactions; partial output and command outcomes stay
//! recoverable after replay eviction. No function here dispatches or retries work.
use crate::{
    model::{Message, Role, Usage},
    session::Session,
};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 3;
const IMPORT_SCHEMA: &str = "CREATE TABLE imports(session_id TEXT PRIMARY KEY REFERENCES sessions(id), transfer_id TEXT NOT NULL UNIQUE, provenance TEXT NOT NULL);";
const MAX_DATABASE_BYTES: i64 = 256 * 1024 * 1024;
const MAX_SESSIONS: i64 = 4096;
const MAX_COMMANDS: i64 = 100_000;
const MAX_SNAPSHOT: usize = 16 * 1024 * 1024;
const MAX_PROMPT: usize = 64 * 1024;
const MAX_PARTIAL: usize = 1024 * 1024;
const REPLAY_LIMIT: i64 = 1024;

pub struct Journal {
    connection: Connection,
    opened_schema: i64,
    directory: PathBuf,
}

/// A stable OS-sidecar lock, not a lock on an atomically replaced session inode.
/// Never unlink its file. Process death releases ownership; PID values are not used.
pub struct ExecutionGuard {
    file: File,
    directory: PathBuf,
    session_id: Uuid,
}
impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Accepted,
    Running,
    Completed,
    Incomplete,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecord {
    pub id: Uuid,
    pub command_id: Uuid,
    pub machine_id: Uuid,
    pub principal_id: Uuid,
    pub session_id: Uuid,
    pub state: RunState,
    pub partial_text: String,
    pub terminal_reason: Option<String>,
    pub usage: Usage,
    pub final_checkpointed: bool,
}

impl std::fmt::Debug for RunRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunRecord")
            .field("id", &self.id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

pub struct VersionedSession {
    pub revision: u64,
    pub session: Session,
}

/// Transport adapters validate expiry/authentication before invoking the journal;
/// expiry is checked again here immediately before durable admission.
#[derive(Serialize)]
pub struct TurnAdmission {
    pub command_id: Uuid,
    pub machine_id: Uuid,
    pub principal_id: Uuid,
    pub session_id: Uuid,
    pub expected_revision: u64,
    pub expires_at_ms: i64,
    pub prompt: String,
}

pub struct Admission {
    pub duplicate: bool,
    pub run: RunRecord,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalEvent {
    pub sequence: u64,
    pub run_id: Uuid,
    pub kind: EventKind,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "data",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum EventKind {
    Accepted,
    Started,
    CanonicalCheckpoint,
    TextDelta(String),
    Terminal(RunState),
}

pub enum Replay {
    Events(Vec<JournalEvent>),
    /// Caller must authorize and construct a safe projection; do not send Session.
    SnapshotRequired {
        latest_sequence: u64,
    },
}

fn check_transaction_schema(tx: &Transaction<'_>, expected: i64) -> Result<()> {
    let actual: i64 = tx.query_row(
        "SELECT version FROM attachment_schema WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    ensure!(
        actual == expected,
        "journal schema changed; reopen after quiescent upgrade"
    );
    Ok(())
}

impl Journal {
    /// Dedicated local store under a trusted private parent. Network filesystems
    /// and hostile same-OS-user processes are outside the OS-lock trust boundary.
    pub fn open(directory: PathBuf) -> Result<Self> {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e).context("create attachment journal directory"),
        }
        let metadata = fs::symlink_metadata(&directory)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "journal directory is not a real directory"
        );
        private(&metadata)?;
        let directory = directory.canonicalize()?;
        let database_path = directory.join("journal.sqlite3");
        let file = open_private_file(&database_path)?;
        drop(file);
        let mut connection = Connection::open(&database_path)?;
        connection.busy_timeout(Duration::ZERO)?; // fail boundedly, never stall an async reactor
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS attachment_schema(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL);")?;
        let version: Option<i64> = tx
            .query_row(
                "SELECT version FROM attachment_schema WHERE id=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(version) = version {
            ensure!(
                matches!(version, 2 | SCHEMA_VERSION),
                "unsupported attachment journal schema"
            );
        } else {
            let other_tables: i64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT IN ('attachment_schema','sqlite_sequence')", [], |r| r.get(0))?;
            ensure!(other_tables == 0, "refusing to adopt an unrelated database");
            tx.execute_batch("CREATE TABLE sessions(id TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK(revision>=0), state TEXT NOT NULL, next_sequence INTEGER NOT NULL DEFAULT 1 CHECK(next_sequence>0));
                CREATE TABLE runs(id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id), record TEXT NOT NULL, active INTEGER NOT NULL CHECK(active IN (0,1)));
                CREATE UNIQUE INDEX one_active_run ON runs(session_id) WHERE active=1;
                CREATE TABLE commands(id TEXT PRIMARY KEY, digest BLOB NOT NULL, run_id TEXT NOT NULL UNIQUE REFERENCES runs(id));
                CREATE TABLE events(session_id TEXT NOT NULL REFERENCES sessions(id), sequence INTEGER NOT NULL, event TEXT NOT NULL, PRIMARY KEY(session_id,sequence));")?;
            tx.execute_batch(IMPORT_SCHEMA)?;
            tx.execute(
                "INSERT INTO attachment_schema VALUES(1, ?1)",
                [SCHEMA_VERSION],
            )?;
        }
        tx.commit()?;
        // Bound durable storage without pruning command evidence. SQLite FULL
        // rolls the transaction back; the caller must stop further dispatch.
        connection.pragma_update(None, "journal_mode", "DELETE")?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        ensure!(page_size > 0, "invalid SQLite page size");
        let page_count: i64 = connection.pragma_query_value(None, "page_count", |r| r.get(0))?;
        ensure!(
            page_count <= MAX_DATABASE_BYTES / page_size,
            "attachment database capacity exceeded"
        );
        connection.pragma_update(None, "max_page_count", MAX_DATABASE_BYTES / page_size)?;
        #[cfg(unix)]
        {
            File::open(&directory)?.sync_all()?;
            File::open(directory.parent().context("journal has no parent")?)?.sync_all()?;
        }
        Ok(Self {
            connection,
            opened_schema: version.unwrap_or(SCHEMA_VERSION),
            directory,
        })
    }

    fn check_schema(&self) -> Result<()> {
        let current: i64 = self.connection.query_row(
            "SELECT version FROM attachment_schema WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            current == self.opened_schema,
            "journal schema changed; reopen after quiescent upgrade"
        );
        Ok(())
    }

    /// Explicit process-quiescent upgrade. Legacy processes must be stopped first.
    /// SQLite excludes concurrent admissions/creation; sidecars exclude effects.
    pub fn upgrade_quiescent(&mut self) -> Result<()> {
        self.upgrade_with(|| Ok(()))
    }

    fn upgrade_with(&mut self, after_fencing: impl FnOnce() -> Result<()>) -> Result<()> {
        self.check_schema()?;
        if self.opened_schema == SCHEMA_VERSION {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let active: i64 =
            tx.query_row("SELECT count(*) FROM runs WHERE active=1", [], |r| r.get(0))?;
        ensure!(
            active == 0,
            "journal has active runs; recover or finish before upgrade"
        );
        let ids = tx
            .prepare("SELECT id FROM sessions")?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut guards = Vec::new();
        for id in ids {
            let session_id = Uuid::parse_str(&id)?;
            let file =
                open_private_file(&self.directory.join(format!("{session_id}.execution.lock")))?;
            file.try_lock()
                .context("session busy during journal upgrade")?;
            guards.push(ExecutionGuard {
                file,
                directory: self.directory.clone(),
                session_id,
            });
        }
        after_fencing()?;
        tx.execute_batch(IMPORT_SCHEMA)?;
        tx.execute(
            "UPDATE attachment_schema SET version=?1 WHERE id=1",
            [SCHEMA_VERSION],
        )?;
        tx.commit()?;
        self.opened_schema = SCHEMA_VERSION;
        drop(guards);
        Ok(())
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn preflight_import(
        &self,
        provenance: &super::migration::Provenance,
    ) -> Result<Option<u64>> {
        self.check_schema()?;
        ensure!(
            self.opened_schema == SCHEMA_VERSION,
            "explicit quiescent journal upgrade required"
        );
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT provenance FROM imports WHERE session_id=?1 OR transfer_id=?2",
                params![
                    provenance.session_id.to_string(),
                    provenance.transfer_id.to_string()
                ],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let existing: super::migration::Provenance = serde_json::from_str(&existing)?;
            ensure!(existing == *provenance, "transfer provenance conflict");
            return Ok(Some(self.load_session(provenance.session_id)?.revision));
        }
        let collision: i64 = self.connection.query_row(
            "SELECT count(*) FROM sessions WHERE id=?1",
            [provenance.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            collision == 0,
            "destination already owns this session without matching provenance"
        );
        let count: i64 = self
            .connection
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
        ensure!(count < MAX_SESSIONS, "attachment session capacity reached");
        Ok(None)
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn import_session(
        &mut self,
        session: &Session,
        provenance: &super::migration::Provenance,
    ) -> Result<(u64, bool)> {
        let encoded = snapshot(session)?;
        ensure!(
            session.id == provenance.session_id && session.revision == provenance.source_revision,
            "source identity or revision mismatch"
        );
        if let Some(revision) = self.preflight_import(provenance)? {
            return Ok((revision, true));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
        ensure!(count < MAX_SESSIONS, "attachment session capacity reached");
        let revision = i64::try_from(session.revision)?;
        tx.execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,?2,?3)",
            params![session.id.to_string(), revision, encoded],
        )?;
        tx.execute(
            "INSERT INTO imports VALUES(?1,?2,?3)",
            params![
                session.id.to_string(),
                provenance.transfer_id.to_string(),
                serde_json::to_string(provenance)?
            ],
        )?;
        tx.commit()?;
        Ok((session.revision, false))
    }

    /// Import/create is explicit and create-only. Caller controls consent. A UUID
    /// collision never replaces another canonical session or its command history.
    pub fn create_session(&mut self, session: &Session) -> Result<()> {
        self.check_schema()?;
        ensure!(!session.id.is_nil(), "nil session identity");
        let encoded = snapshot(session)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let count: i64 = tx.query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))?;
        ensure!(count < MAX_SESSIONS, "attachment session capacity reached");
        tx.execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,0,?2)",
            params![session.id.to_string(), encoded],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_session(&self, id: Uuid) -> Result<VersionedSession> {
        read_session(&self.connection, id)
    }

    pub fn acquire_execution(&self, session_id: Uuid) -> Result<ExecutionGuard> {
        self.check_schema()?;
        ensure!(!session_id.is_nil(), "nil session identity");
        self.load_session(session_id)?; // Unknown IDs must not allocate lock files.
        let file = open_private_file(&self.directory.join(format!("{session_id}.execution.lock")))?;
        file.try_lock()
            .context("session busy or execution locking unavailable")?;
        let guard = ExecutionGuard {
            file,
            directory: self.directory.clone(),
            session_id,
        };
        self.load_session(session_id)?;
        Ok(guard)
    }

    fn check_guard(&self, guard: &ExecutionGuard, session: Uuid) -> Result<()> {
        self.check_schema()?;
        ensure!(
            guard.directory == self.directory && guard.session_id == session,
            "execution guard belongs to another journal or session"
        );
        Ok(())
    }

    /// Retrieve an identical accepted command without taking execution ownership.
    /// Authentication/sharing must still be checked by the caller on every retry.
    pub fn lookup_command(&self, request: &TurnAdmission) -> Result<Option<RunRecord>> {
        ensure!(request.prompt.len() <= MAX_PROMPT, "invalid prompt size");
        let digest = Sha256::digest(serde_json::to_vec(request)?).to_vec();
        let existing: Option<(Vec<u8>, String)> = self
            .connection
            .query_row(
                "SELECT digest,run_id FROM commands WHERE id=?1",
                [request.command_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        existing
            .map(|(previous, id)| {
                ensure!(
                    previous == digest,
                    "command identity reused with different payload or authority"
                );
                read_run(&self.connection, Uuid::parse_str(&id)?)
            })
            .transpose()
    }

    /// Duplicate lookup precedes deadline/revision checking: an expired retry can
    /// inspect an already accepted result, but cannot create another execution.
    pub fn admit_turn(
        &mut self,
        guard: &ExecutionGuard,
        request: &TurnAdmission,
        now_ms: i64,
    ) -> Result<Admission> {
        self.check_guard(guard, request.session_id)?;
        ensure!(
            !request.command_id.is_nil()
                && !request.machine_id.is_nil()
                && !request.principal_id.is_nil(),
            "nil command authority identity"
        );
        ensure!(
            !request.prompt.trim().is_empty() && request.prompt.len() <= MAX_PROMPT,
            "invalid prompt size"
        );
        let digest = Sha256::digest(serde_json::to_vec(request)?).to_vec();
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let existing: Option<(Vec<u8>, String)> = tx
            .query_row(
                "SELECT digest,run_id FROM commands WHERE id=?1",
                [request.command_id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((old_digest, run_id)) = existing {
            ensure!(
                old_digest == digest,
                "command identity reused with different payload or authority"
            );
            let run = read_run(&tx, Uuid::parse_str(&run_id)?)?;
            return Ok(Admission {
                duplicate: true,
                run,
            });
        }
        ensure!(
            now_ms >= 0
                && request.expires_at_ms > now_ms
                && request.expires_at_ms
                    <= now_ms.checked_add(300_000).context("clock overflow")?,
            "command expired or deadline invalid"
        );
        let count: i64 = tx.query_row("SELECT count(*) FROM commands", [], |r| r.get(0))?;
        ensure!(count < MAX_COMMANDS, "command evidence capacity reached");
        let active: i64 = tx.query_row(
            "SELECT count(*) FROM runs WHERE session_id=?1 AND active=1",
            [request.session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(active == 0, "session has an unresolved active run");
        let mut current = read_session(&tx, request.session_id)?;
        ensure!(
            !has_pending_tools(&current.session.messages),
            "session has unresolved tool effects; explicit reconciliation is required before another turn"
        );
        ensure!(
            current.revision == request.expected_revision,
            "stale session revision"
        );
        current
            .session
            .messages
            .push(Message::new(Role::User, &request.prompt));
        update_session(&tx, &current)?;
        let run = RunRecord {
            id: Uuid::new_v4(),
            command_id: request.command_id,
            machine_id: request.machine_id,
            principal_id: request.principal_id,
            session_id: request.session_id,
            state: RunState::Accepted,
            partial_text: String::new(),
            terminal_reason: None,
            usage: Usage::default(),
            final_checkpointed: false,
        };
        tx.execute(
            "INSERT INTO runs VALUES(?1,?2,?3,1)",
            params![
                run.id.to_string(),
                run.session_id.to_string(),
                serde_json::to_string(&run)?
            ],
        )?;
        tx.execute(
            "INSERT INTO commands VALUES(?1,?2,?3)",
            params![request.command_id.to_string(), digest, run.id.to_string()],
        )?;
        append_event(&tx, &run, EventKind::Accepted)?;
        tx.commit()?;
        Ok(Admission {
            duplicate: false,
            run,
        })
    }

    /// Commit dispatch intent before starting effects. A crash after this commit
    /// is uncertain execution, never grounds to run the command again.
    pub fn mark_running(&mut self, guard: &ExecutionGuard, run_id: Uuid) -> Result<()> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut run = read_run(&tx, run_id)?;
        ensure!(
            run.state == RunState::Accepted,
            "run already dispatched or terminal"
        );
        run.state = RunState::Running;
        tx.execute(
            "UPDATE runs SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&run)?, run.id.to_string()],
        )?;
        append_event(&tx, &run, EventKind::Started)?;
        tx.commit()?;
        Ok(())
    }

    pub fn run(&self, run_id: Uuid) -> Result<RunRecord> {
        read_run(&self.connection, run_id)
    }

    /// Output is recorded before being advertised as durably replayable. This
    /// partial field is not a completed provider message. Size failure stops new
    /// output rather than dropping already acknowledged text.
    pub fn append_text(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        delta: &str,
    ) -> Result<u64> {
        ensure!(
            !delta.is_empty() && delta.len() <= MAX_PROMPT,
            "invalid output delta size"
        );
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut run = read_run(&tx, run_id)?;
        ensure!(run.state == RunState::Running, "run is terminal");
        ensure!(
            run.partial_text
                .len()
                .checked_add(delta.len())
                .is_some_and(|n| n <= MAX_PARTIAL),
            "partial output capacity reached"
        );
        run.partial_text.push_str(delta);
        tx.execute(
            "UPDATE runs SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&run)?, run.id.to_string()],
        )?;
        let sequence = append_event(&tx, &run, EventKind::TextDelta(delta.to_owned()))?;
        tx.commit()?;
        Ok(sequence)
    }

    /// Persist the complete canonical provider/tool history before further effects.
    /// History is append-only within a run; no compaction or private history rewrite.
    /// This local API is never an outbound projection.
    pub fn checkpoint_canonical(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        messages: &[Message],
        usage: &Usage,
    ) -> Result<()> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut run = read_run(&tx, run_id)?;
        ensure!(
            run.state == RunState::Running,
            "checkpoint requires a running run"
        );
        ensure!(
            messages.iter().all(|m| m.role != Role::System),
            "runtime guidance cannot enter canonical checkpoints"
        );
        let mut current = read_session(&tx, run.session_id)?;
        let previous = current.session.messages.len();
        ensure!(
            messages.len() >= previous
                && serde_json::to_vec(&messages[..previous])?
                    == serde_json::to_vec(&current.session.messages)?,
            "canonical checkpoint rewrites accepted history"
        );
        let input_delta = usage
            .input_tokens
            .checked_sub(run.usage.input_tokens)
            .context("run usage decreased")?;
        let output_delta = usage
            .output_tokens
            .checked_sub(run.usage.output_tokens)
            .context("run usage decreased")?;
        if messages.len() == previous && input_delta == 0 && output_delta == 0 {
            return Ok(());
        }
        current.session.replace_messages(messages.to_vec());
        current.session.refresh_active_run_summary();
        current.session.usage.input_tokens = current
            .session
            .usage
            .input_tokens
            .checked_add(input_delta)
            .context("session usage overflow")?;
        current.session.usage.output_tokens = current
            .session
            .usage
            .output_tokens
            .checked_add(output_delta)
            .context("session usage overflow")?;
        run.usage = usage.clone();
        // Canonical text is provisional. Only the runtime's explicit acceptance
        // hook may classify a final response; no-tool text alone is insufficient.
        run.final_checkpointed = false;
        update_session(&tx, &current)?;
        tx.execute(
            "UPDATE runs SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&run)?, run.id.to_string()],
        )?;
        append_event(&tx, &run, EventKind::CanonicalCheckpoint)?;
        tx.commit()?;
        Ok(())
    }

    /// Record trusted run ownership before any provider or tool dispatch.
    pub fn register_run_scope(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        reference: crate::completion::runtime::RunReference,
    ) -> Result<()> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        ensure!(
            reference.session_id == run.session_id && reference.run_id == run_id,
            "completion scope does not match admitted run"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let run = read_run(&tx, run_id)?;
        ensure!(
            run.state == RunState::Running,
            "scope requires running admission"
        );
        let mut current = read_session(&tx, run.session_id)?;
        ensure!(
            !current.session.completion_runs.contains(&reference),
            "scope already registered"
        );
        current.session.begin_run_summary(reference.run_id);
        current.session.completion_runs.push(reference);
        update_session(&tx, &current)?;
        tx.commit()?;
        Ok(())
    }

    /// Mark an already-durable canonical proposal accepted. Scoped callers invoke
    /// this only after sealing readiness. A failed write cannot become completion.
    pub fn accept_checkpoint(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        messages: &[Message],
        usage: &Usage,
    ) -> Result<()> {
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut run = read_run(&tx, run_id)?;
        ensure!(
            run.state == RunState::Running,
            "acceptance requires running admission"
        );
        let current = read_session(&tx, run.session_id)?;
        ensure!(
            serde_json::to_vec(messages)? == serde_json::to_vec(&current.session.messages)?
                && serde_json::to_vec(usage)? == serde_json::to_vec(&run.usage)?,
            "acceptance does not match canonical checkpoint"
        );
        ensure!(
            messages
                .last()
                .is_some_and(|m| m.role == Role::Assistant && m.tool_calls.is_empty())
                && !has_pending_tools(messages),
            "acceptance requires a final proposal without pending tools"
        );
        ensure!(!run.final_checkpointed, "proposal already accepted");
        run.final_checkpointed = true;
        tx.execute(
            "UPDATE runs SET record=?1 WHERE id=?2",
            params![serde_json::to_string(&run)?, run.id.to_string()],
        )?;
        append_event(&tx, &run, EventKind::CanonicalCheckpoint)?;
        tx.commit()?;
        Ok(())
    }

    /// Terminal outcome is immutable. A failed/interrupted run keeps partial text
    /// in its run record; it must not become an accepted assistant completion.
    pub fn finish(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        state: RunState,
        reason: Option<&str>,
        final_text: Option<&str>,
    ) -> Result<RunRecord> {
        self.finish_classified(guard, run_id, state, reason, final_text, None)
    }

    /// Commit terminal status and local transcript classification together.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_classified(
        &mut self,
        guard: &ExecutionGuard,
        run_id: Uuid,
        state: RunState,
        reason: Option<&str>,
        final_text: Option<&str>,
        classification: Option<&crate::agent::StopReason>,
    ) -> Result<RunRecord> {
        ensure!(
            !matches!(state, RunState::Accepted | RunState::Running),
            "finish requires terminal state"
        );
        ensure!(
            reason.is_none_or(|s| !s.trim().is_empty()
                && s.len() <= 1024
                && !s.chars().any(char::is_control)),
            "invalid terminal reason"
        );
        ensure!(
            state == RunState::Completed || (reason.is_some() && final_text.is_none()),
            "unsuccessful runs require a reason and cannot publish success"
        );
        ensure!(
            state != RunState::Completed || final_text.is_none_or(|s| s.len() <= MAX_PARTIAL),
            "completed run requires bounded final text"
        );
        let run = self.run(run_id)?;
        self.check_guard(guard, run.session_id)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_transaction_schema(&tx, self.opened_schema)?;
        let mut run = read_run(&tx, run_id)?;
        ensure!(
            matches!(run.state, RunState::Accepted | RunState::Running),
            "run is already terminal"
        );
        ensure!(
            state != RunState::Completed || run.state == RunState::Running,
            "undispatched run cannot complete"
        );
        ensure!(
            state != RunState::Completed || final_text.is_some() || run.final_checkpointed,
            "completed run requires a final checkpoint"
        );
        let mut current = read_session(&tx, run.session_id)?;
        ensure!(
            state != RunState::Completed || !has_pending_tools(&current.session.messages),
            "unresolved tool effects cannot be completed"
        );
        ensure!(
            !(final_text.is_some()
                && current
                    .session
                    .messages
                    .last()
                    .is_some_and(|m| m.role == Role::Assistant && m.tool_calls.is_empty())),
            "final answer already checkpointed"
        );
        if let Some(text) = final_text {
            current
                .session
                .messages
                .push(Message::new(Role::Assistant, text));
        }
        if state == RunState::Interrupted {
            for terminal in &mut current.session.terminals {
                terminal.state = crate::terminal::TerminalState::Disconnected;
            }
        }
        if let Some(classification) = classification {
            ensure!(
                matches!(
                    (&state, classification),
                    (RunState::Completed, crate::agent::StopReason::Completed)
                        | (
                            RunState::Incomplete,
                            crate::agent::StopReason::Incomplete { .. }
                        )
                ),
                "run state and transcript classification disagree"
            );
            current.session.finish_run_summary(classification);
        } else if state == RunState::Completed {
            current
                .session
                .finish_run_summary(&crate::agent::StopReason::Completed);
        } else {
            current
                .session
                .interrupt_run_summary(reason.unwrap_or("run interrupted").to_owned());
        }
        update_session(&tx, &current)?;
        run.state = state.clone();
        run.terminal_reason = reason.map(str::to_owned);
        tx.execute(
            "UPDATE runs SET record=?1,active=0 WHERE id=?2",
            params![serde_json::to_string(&run)?, run.id.to_string()],
        )?;
        append_event(&tx, &run, EventKind::Terminal(state))?;
        tx.commit()?;
        Ok(run)
    }

    /// Caller acquires ownership after the old process has exited. This never
    /// starts/retries tools; active ownership prevents premature recovery.
    pub fn recover_interrupted(&mut self, guard: &ExecutionGuard) -> Result<Option<RunRecord>> {
        self.check_guard(guard, guard.session_id)?;
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM runs WHERE session_id=?1 AND active=1",
                [guard.session_id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        id.map(|id| {
            self.finish(
                guard,
                Uuid::parse_str(&id)?,
                RunState::Interrupted,
                Some("execution owner exited before terminal checkpoint"),
                None,
            )
        })
        .transpose()
    }

    pub fn replay(&mut self, session_id: Uuid, after: u64) -> Result<Replay> {
        let after = i64::try_from(after).context("event cursor overflow")?;
        let tx = self.connection.transaction()?;
        let latest: i64 = tx.query_row(
            "SELECT next_sequence-1 FROM sessions WHERE id=?1",
            [session_id.to_string()],
            |r| r.get(0),
        )?;
        ensure!(after <= latest, "event cursor is ahead of journal");
        let earliest: Option<i64> = tx.query_row(
            "SELECT min(sequence) FROM events WHERE session_id=?1",
            [session_id.to_string()],
            |r| r.get(0),
        )?;
        if earliest.is_some_and(|first| after < first - 1) || (earliest.is_none() && after < latest)
        {
            return Ok(Replay::SnapshotRequired {
                latest_sequence: latest as u64,
            });
        }
        let mut statement = tx.prepare("SELECT sequence,event FROM events WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
        let rows = statement
            .query_map(params![session_id.to_string(), after, REPLAY_LIMIT], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?;
        let mut events = Vec::new();
        let mut previous = after;
        for row in rows {
            let (sequence, encoded) = row?;
            ensure!(
                encoded.len() <= MAX_PROMPT * 6 + 4096,
                "stored replay event capacity exceeded"
            );
            let event: JournalEvent = serde_json::from_str(&encoded)?;
            ensure!(
                sequence > 0 && event.sequence == sequence as u64,
                "stored replay event identity mismatch"
            );
            let belongs: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1 AND session_id=?2)",
                params![event.run_id.to_string(), session_id.to_string()],
                |r| r.get(0),
            )?;
            ensure!(belongs, "stored replay event belongs to another session");
            if sequence != previous + 1 {
                return Ok(Replay::SnapshotRequired {
                    latest_sequence: latest as u64,
                });
            }
            previous = sequence;
            events.push(event);
        }
        if previous != latest {
            return Ok(Replay::SnapshotRequired {
                latest_sequence: latest as u64,
            });
        }
        Ok(Replay::Events(events))
    }
}

// Preserve uncertain intent rather than inventing tool results or replaying an
// effect. Older orphan result messages may exist after legacy compaction; they
// cannot satisfy a different outstanding call. A future explicit reconciliation
// operation must account for unknown effects before reopening this session.
fn has_pending_tools(messages: &[Message]) -> bool {
    let mut pending = std::collections::BTreeSet::new();
    for message in messages {
        if message.role == Role::Assistant {
            for call in &message.tool_calls {
                if call.id.is_empty() || !pending.insert(call.id.as_str()) {
                    return true;
                }
            }
        } else if message.role == Role::Tool
            && let Some(id) = &message.tool_call_id
        {
            pending.remove(id.as_str());
        }
    }
    !pending.is_empty()
}

fn snapshot(session: &Session) -> Result<String> {
    let mut value = session.clone();
    value.messages.retain(|m| m.role != Role::System);
    let encoded = serde_json::to_string(&value)?;
    ensure!(
        encoded.len() <= MAX_SNAPSHOT,
        "session snapshot capacity reached"
    );
    Ok(encoded)
}
fn read_session(db: &Connection, id: Uuid) -> Result<VersionedSession> {
    let (revision, encoded): (i64, String) = db.query_row(
        "SELECT revision,state FROM sessions WHERE id=?1 AND length(CAST(state AS BLOB))<=?2",
        params![id.to_string(), MAX_SNAPSHOT],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        encoded.len() <= MAX_SNAPSHOT && revision >= 0,
        "invalid session record bounds"
    );
    let session: Session = serde_json::from_str(&encoded)?;
    ensure!(session.id == id, "session record identity mismatch");
    Ok(VersionedSession {
        revision: revision as u64,
        session,
    })
}
fn update_session(tx: &Transaction<'_>, current: &VersionedSession) -> Result<()> {
    let revision = i64::try_from(current.revision).context("revision overflow")?;
    let next = revision.checked_add(1).context("revision overflow")?;
    let mut session = current.session.clone();
    session.updated_at = chrono::Utc::now();
    ensure!(
        tx.execute(
            "UPDATE sessions SET state=?1,revision=?2 WHERE id=?3 AND revision=?4",
            params![snapshot(&session)?, next, session.id.to_string(), revision]
        )? == 1,
        "stale session revision"
    );
    Ok(())
}
fn read_run(db: &Connection, id: Uuid) -> Result<RunRecord> {
    let (encoded, session_id, active, command_id): (String, String, i64, String) = db.query_row(
        "SELECT r.record,r.session_id,r.active,c.id FROM runs r JOIN commands c ON c.run_id=r.id WHERE r.id=?1 AND length(CAST(r.record AS BLOB))<=?2",
        params![id.to_string(), MAX_PARTIAL * 6 + 4096],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let run: RunRecord = serde_json::from_str(&encoded)?;
    ensure!(
        run.id == id
            && run.session_id.to_string() == session_id
            && run.command_id.to_string() == command_id,
        "run record identity mismatch"
    );
    ensure!(
        !run.machine_id.is_nil() && !run.principal_id.is_nil(),
        "invalid stored run authority"
    );
    ensure!(
        (active == 1) == matches!(run.state, RunState::Accepted | RunState::Running),
        "inconsistent stored run lifecycle"
    );
    ensure!(
        run.partial_text.len() <= MAX_PARTIAL,
        "stored partial output capacity exceeded"
    );
    Ok(run)
}
fn append_event(tx: &Transaction<'_>, run: &RunRecord, kind: EventKind) -> Result<u64> {
    let sequence: i64 = tx.query_row(
        "SELECT next_sequence FROM sessions WHERE id=?1",
        [run.session_id.to_string()],
        |r| r.get(0),
    )?;
    ensure!(sequence > 0, "invalid event sequence");
    let next = sequence.checked_add(1).context("event sequence overflow")?;
    let event = JournalEvent {
        sequence: sequence as u64,
        run_id: run.id,
        kind,
    };
    tx.execute(
        "INSERT INTO events VALUES(?1,?2,?3)",
        params![
            run.session_id.to_string(),
            sequence,
            serde_json::to_string(&event)?
        ],
    )?;
    tx.execute(
        "UPDATE sessions SET next_sequence=?1 WHERE id=?2",
        params![next, run.session_id.to_string()],
    )?;
    tx.execute(
        "DELETE FROM events WHERE session_id=?1 AND sequence<=?2",
        params![
            run.session_id.to_string(),
            sequence.saturating_sub(REPLAY_LIMIT)
        ],
    )?;
    Ok(sequence as u64)
}
fn private(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.mode() & 0o077 == 0,
            "attachment storage must be owner-only"
        );
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "attachment storage has another owner"
        );
    }
    #[cfg(not(unix))]
    let _ = metadata; // Windows ACL verification is a delivery prerequisite.
    Ok(())
}
fn open_private_file(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            ensure!(
                m.is_file() && !m.file_type().is_symlink(),
                "attachment store path is not a regular file"
            );
            private(&m)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "attachment store is not a regular file"
    );
    private(&file.metadata()?)?;
    Ok(file)
}

#[cfg(test)]
mod tests;
