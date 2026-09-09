//! Private local inference admission. A permit counts a local dispatch boundary,
//! never a billed token, dollar, or proof that the server received a request.
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

pub mod cli;
pub mod history;
pub mod runtime;

const MAX_COUNT: u64 = i64::MAX as u64 - 1;
const MAX_RECORD: usize = 16 * 1024;
const MAX_DETAILS: i64 = 20_000;
const MAX_AUDIT: i64 = 20_000;
const MAX_DATABASE: i64 = 64 * 1024 * 1024;
const RESERVED_BYTES: i64 = 8 * 1024 * 1024;

/// Static frontend diagnostics never carry SQLite text, paths, or corrupt records.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub enum Failure {
    #[error(
        "local inference attempt allowance exhausted; inspect and explicitly revise the budget with an operator reason"
    )]
    Exhausted,
    #[error("local inference evidence capacity reached; no required evidence was pruned")]
    Capacity,
    #[error(
        "local inference state is busy; retry the identical operation after current work settles"
    )]
    Busy,
    #[error("local inference state unavailable; no further dispatch was authorized")]
    Unavailable,
    #[error("invalid inference configuration or attribution")]
    Invalid,
    #[error("stale inference budget revision; inspect current state before another edit")]
    Stale,
    #[error("inference operation identity conflicts with its immutable receipt")]
    Conflict,
    #[error("historical usage snapshot changed; refresh from its first page before continuing")]
    HistoryChanged,
}
impl Failure {
    pub fn from_error(error: &anyhow::Error) -> Self {
        if let Some(failure) = error.downcast_ref::<Self>() {
            return *failure;
        }
        if let Some(rusqlite::Error::SqliteFailure(error, _)) =
            error.downcast_ref::<rusqlite::Error>()
        {
            if matches!(
                error.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) {
                return Self::Busy;
            }
            if error.code == rusqlite::ErrorCode::DiskFull {
                return Self::Capacity;
            }
        }
        Self::Unavailable
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Project(Uuid),
    Session(Uuid),
}
impl Scope {
    fn key(self) -> String {
        match self {
            Self::Project(id) => format!("project:{id}"),
            Self::Session(id) => format!("session:{id}"),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Change {
    pub operation: Uuid,
    pub scope: Scope,
    pub expected_revision: u64,
    pub limit: Option<u64>,
    pub warning: Option<u64>,
    pub reason: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Status {
    pub scope: Scope,
    pub revision: u64,
    pub consumed: u64,
    pub omitted_attempt_details: u64,
    pub limit: Option<u64>,
    pub warning: Option<u64>,
}
impl Status {
    pub fn summary(&self) -> String {
        format!(
            "{}: {} local attempts, limit {}, omitted details {}",
            self.scope.key(),
            self.consumed,
            self.limit
                .map_or_else(|| "unlimited".into(), |value| value.to_string()),
            self.omitted_attempt_details
        )
    }
    pub fn remaining(&self) -> Option<u64> {
        self.limit.map(|limit| limit.saturating_sub(self.consumed))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub operation: Uuid,
    pub status: Status,
    pub reason: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    Conversation,
    Title,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attribution {
    pub session: Uuid,
    pub run: Uuid,
    pub agent: Option<Uuid>,
    pub provider: String,
    pub model: String,
    pub purpose: Purpose,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Unknown,
    Completed,
    Failed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Attempt {
    pub sequence: u64,
    pub id: Uuid,
    pub project: Uuid,
    pub attribution: Attribution,
    pub admitted_at: String,
    pub finished_at: Option<String>,
    pub outcome: AttemptOutcome,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Permit {
    pub id: Uuid,
    pub retained: bool,
    pub warnings: Vec<Status>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Audit {
    pub sequence: u64,
    pub at: String,
    pub scope: Scope,
    pub event: String,
    pub detail: serde_json::Value,
}

pub struct Store {
    connection: Connection,
    #[cfg(windows)]
    _private: std::sync::Arc<voyage_storage::PrivateDirectory>,
}
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn bounded_json<T: Serialize>(value: &T) -> Result<String> {
    let json = serde_json::to_string(value)?;
    ensure!(json.len() <= MAX_RECORD, "inference record too large");
    Ok(json)
}
fn decode<T: serde::de::DeserializeOwned>(value: Option<String>) -> Result<T> {
    let value = value.context("oversized inference record")?;
    ensure!(value.len() <= MAX_RECORD, "oversized inference record");
    Ok(serde_json::from_str(&value)?)
}
fn counter(value: i64) -> Result<u64> {
    let value = u64::try_from(value).context("invalid negative inference counter")?;
    ensure!(value <= MAX_COUNT, "inference counter exceeds capacity");
    Ok(value)
}
fn status(connection: &Connection, scope: Scope) -> Result<Status> {
    schema(connection)?;
    let raw = connection
        .query_row(
            "SELECT revision,consumed,hard_limit,warning,omitted FROM limits WHERE scope=?1",
            [scope.key()],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((revision, consumed, limit, warning, omitted)) = raw else {
        anyhow::bail!("unknown inference scope")
    };
    let result = Status {
        scope,
        revision: counter(revision)?,
        consumed: counter(consumed)?,
        omitted_attempt_details: counter(omitted)?,
        limit: limit.map(counter).transpose()?,
        warning: warning.map(counter).transpose()?,
    };
    ensure!(
        result
            .warning
            .zip(result.limit)
            .is_none_or(|(warning, limit)| warning <= limit),
        "invalid warning threshold"
    );
    ensure!(
        result.omitted_attempt_details <= result.consumed,
        Failure::Invalid
    );
    Ok(result)
}
fn schema(connection: &Connection) -> Result<()> {
    let version: i64 = connection.query_row(
        "SELECT version FROM inference_schema WHERE id=1",
        [],
        |row| row.get(0),
    )?;
    ensure!(version == 1, Failure::Unavailable);
    Ok(())
}
fn ordinary_capacity(connection: &Connection) -> Result<bool> {
    let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
    let pages: i64 = connection.pragma_query_value(None, "page_count", |r| r.get(0))?;
    ensure!(page_size > 0 && pages >= 0, Failure::Unavailable);
    Ok(pages <= (MAX_DATABASE - RESERVED_BYTES) / page_size)
}
fn audit(
    connection: &Connection,
    scope: Scope,
    event: &str,
    detail: serde_json::Value,
) -> Result<()> {
    let count: i64 = connection.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))?;
    ensure!(
        count < MAX_AUDIT - if event == "configured" { 0 } else { 100 },
        "inference audit capacity reached; no evidence was pruned"
    );
    ensure!(
        event == "configured" || ordinary_capacity(connection)?,
        Failure::Capacity
    );
    let item = Audit {
        sequence: 0,
        at: now(),
        scope,
        event: event.into(),
        detail,
    };
    connection.execute(
        "INSERT INTO audit(scope,record) VALUES(?1,?2)",
        params![scope.key(), bounded_json(&item)?],
    )?;
    Ok(())
}
impl Store {
    pub fn default_path() -> PathBuf {
        crate::config::default_data_dir().join("inference")
    }
    pub fn open(directory: PathBuf) -> Result<Self> {
        // Reuse the journal's native private directory/file checks without sharing
        // its schema, grants, authority, or database.
        #[cfg(windows)]
        let (directory, private) = crate::attachment::journal::storage::prepare(directory)?;
        #[cfg(not(windows))]
        let directory = crate::attachment::journal::prepare_directory(directory)?;
        let database = directory.join("journal.sqlite3");
        let file = crate::attachment::journal::open_private_file(&database)?;
        ensure!(
            file.metadata()?.len() <= MAX_DATABASE as u64,
            "inference database capacity exceeded"
        );
        drop(file);
        let mut connection = Connection::open(database)?;
        // Independent voyage processes share this host accounting database.
        // Let their short transactions serialize instead of failing otherwise
        // successful concurrent inference. SQLite retries lock acquisition, not
        // provider dispatch or an application operation with uncertain effects.
        // Keep the wait below the accounting worker's ten-second deadline.
        connection.busy_timeout(Duration::from_secs(2))?;
        #[cfg(windows)]
        crate::attachment::journal::storage::configure(&connection)?;
        connection.execute_batch(
            "PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;",
        )?;
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: i64 = connection.pragma_query_value(None, "page_count", |r| r.get(0))?;
        ensure!(
            page_size > 0 && pages <= MAX_DATABASE / page_size,
            "inference database capacity exceeded"
        );
        connection.pragma_update(None, "max_page_count", MAX_DATABASE / page_size)?;
        let has_schema: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='inference_schema')", [], |r| r.get(0))?;
        if has_schema {
            let version: i64 = connection.query_row(
                "SELECT version FROM inference_schema WHERE id=1",
                [],
                |r| r.get(0),
            )?;
            ensure!(version == 1, "unsupported inference schema");
        } else {
            let tables: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            ensure!(tables == 0, "refusing unrelated inference database");
        }
        if !has_schema {
            let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch("CREATE TABLE IF NOT EXISTS inference_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);")?;
            let version: Option<i64> = tx
                .query_row("SELECT version FROM inference_schema WHERE id=1", [], |r| {
                    r.get(0)
                })
                .optional()?;
            if let Some(version) = version {
                ensure!(version == 1, "unsupported inference schema");
            } else {
                let others: i64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT IN ('inference_schema','sqlite_sequence')", [], |r| r.get(0))?;
                ensure!(others == 0, "refusing unrelated inference database");
                tx.execute_batch("CREATE TABLE projects(id TEXT PRIMARY KEY,root BLOB NOT NULL UNIQUE);
                CREATE TABLE sessions(id TEXT PRIMARY KEY,project TEXT NOT NULL REFERENCES projects(id));
                CREATE TABLE limits(scope TEXT PRIMARY KEY,revision INTEGER NOT NULL DEFAULT 0,consumed INTEGER NOT NULL DEFAULT 0,hard_limit INTEGER,warning INTEGER,warned_revision INTEGER,omitted INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE receipts(id TEXT PRIMARY KEY,payload TEXT NOT NULL,record TEXT NOT NULL);
                CREATE TABLE attempts(sequence INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL UNIQUE,project TEXT NOT NULL,session TEXT NOT NULL,record TEXT NOT NULL);
                CREATE INDEX attempts_project ON attempts(project,sequence);
                CREATE INDEX attempts_session ON attempts(session,sequence);
                CREATE TABLE audit(sequence INTEGER PRIMARY KEY AUTOINCREMENT,scope TEXT NOT NULL,record TEXT NOT NULL);
                CREATE INDEX audit_scope ON audit(scope,sequence);
                INSERT INTO inference_schema VALUES(1,1);")?;
            }
            tx.commit()?;
        }
        let page_size: i64 = connection.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: i64 = connection.pragma_query_value(None, "page_count", |r| r.get(0))?;
        ensure!(
            page_size > 0 && pages <= MAX_DATABASE / page_size,
            "inference database capacity exceeded"
        );
        connection.pragma_update(None, "max_page_count", MAX_DATABASE / page_size)?;
        #[cfg(windows)]
        crate::attachment::journal::storage::verify(&private)?;
        Ok(Self {
            connection,
            #[cfg(windows)]
            _private: private,
        })
    }
    pub fn project(&mut self, root: &Path) -> Result<Uuid> {
        schema(&self.connection)?;
        let root = root.canonicalize()?;
        ensure!(root.is_dir(), "project root is not a directory");
        let bytes = root.as_os_str().as_encoded_bytes();
        ensure!(bytes.len() <= 4096, "project root too large");
        let read: Option<Option<String>> = self.connection.query_row("SELECT CASE WHEN length(CAST(id AS BLOB))=36 THEN id END FROM projects WHERE root=?1", [bytes], |r|r.get(0)).optional()?;
        if let Some(id) = read {
            let id = Uuid::parse_str(&id.context("invalid project identity")?)?;
            ensure!(!id.is_nil(), "invalid project identity");
            return Ok(id);
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let existing: Option<Option<String>> = tx.query_row("SELECT CASE WHEN length(CAST(id AS BLOB))=36 THEN id END FROM projects WHERE root=?1", [bytes], |r| r.get(0)).optional()?;
        let id = match existing {
            Some(id) => Uuid::parse_str(&id.context("invalid project identity")?)?,
            None => {
                let id = Uuid::new_v4();
                tx.execute(
                    "INSERT INTO projects VALUES(?1,?2)",
                    params![id.to_string(), bytes],
                )?;
                tx.execute(
                    "INSERT INTO limits(scope) VALUES(?1)",
                    [Scope::Project(id).key()],
                )?;
                id
            }
        };
        ensure!(!id.is_nil(), "invalid project identity");
        tx.commit()?;
        Ok(id)
    }
    pub fn bind_session(&mut self, project: Uuid, session: Uuid) -> Result<()> {
        ensure!(
            !project.is_nil() && !session.is_nil(),
            "invalid inference identity"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        status(&tx, Scope::Project(project))?;
        let inserted = tx.execute(
            "INSERT INTO sessions VALUES(?1,?2) ON CONFLICT(id) DO NOTHING",
            params![session.to_string(), project.to_string()],
        )?;
        let existing: Option<String> = tx.query_row("SELECT CASE WHEN length(CAST(project AS BLOB))=36 THEN project END FROM sessions WHERE id=?1", [session.to_string()], |r| r.get(0))?;
        ensure!(
            existing.as_deref() == Some(&project.to_string()),
            "session already belongs to another inference project"
        );
        if inserted == 1 {
            tx.execute(
                "INSERT INTO limits(scope) VALUES(?1)",
                [Scope::Session(session).key()],
            )?;
        } else {
            status(&tx, Scope::Session(session))?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn session_project(&self, session: Uuid) -> Result<Uuid> {
        schema(&self.connection)?;
        let value: Option<String> = self.connection.query_row("SELECT CASE WHEN length(CAST(project AS BLOB))=36 THEN project END FROM sessions WHERE id=?1", [session.to_string()], |r| r.get(0))?;
        let id = Uuid::parse_str(&value.context("invalid inference project")?)?;
        ensure!(!id.is_nil(), "invalid inference project");
        Ok(id)
    }
    pub fn inspect(&self, scope: Scope) -> Result<Status> {
        status(&self.connection, scope)
    }
    fn validate_change(change: &Change) -> Result<()> {
        ensure!(!change.operation.is_nil(), "operation identity is required");
        ensure!(
            !change.reason.trim().is_empty()
                && change.reason.len() <= 4096
                && !change.reason.chars().any(char::is_control),
            "a bounded plain-text operator reason is required"
        );
        ensure!(
            change
                .limit
                .into_iter()
                .chain(change.warning)
                .chain([change.expected_revision])
                .all(|value| value <= MAX_COUNT),
            "invalid inference limit"
        );
        ensure!(
            change
                .warning
                .zip(change.limit)
                .is_none_or(|(warning, limit)| warning <= limit),
            "warning exceeds hard limit"
        );
        Ok(())
    }
    pub fn preview(&self, change: &Change) -> Result<Status> {
        Self::validate_change(change).map_err(|_| Failure::Invalid)?;
        let current = self.inspect(change.scope)?;
        ensure!(current.revision == change.expected_revision, Failure::Stale);
        ensure!(current.revision < MAX_COUNT, "inference revision exhausted");
        Ok(current)
    }
    pub fn configure(&mut self, change: &Change) -> Result<Receipt> {
        Self::validate_change(change).map_err(|_| Failure::Invalid)?;
        let payload = bounded_json(change)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let existing: Option<(Option<String>, Option<String>)> = tx.query_row("SELECT CASE WHEN length(CAST(payload AS BLOB))<=16384 THEN payload END,CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM receipts WHERE id=?1", [change.operation.to_string()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((stored, record)) = existing {
            ensure!(stored.as_deref() == Some(&payload), Failure::Conflict);
            return decode(record);
        }
        let current = status(&tx, change.scope)?;
        ensure!(current.revision < MAX_COUNT, "inference revision exhausted");
        let audits: i64 = tx.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))?;
        let reserved_edit = change.limit.is_some() && current.limit.is_none()
            || change.limit.is_none() && change.warning.is_none()
            || change
                .limit
                .zip(current.limit)
                .is_some_and(|(next, old)| next <= old);
        ensure!(
            audits < MAX_AUDIT - if reserved_edit { 0 } else { 100 },
            "inference edit audit capacity reached; remaining space is reserved for lowering limits or explicitly removing configuration"
        );
        ensure!(current.revision == change.expected_revision, Failure::Stale);
        tx.execute("UPDATE limits SET revision=revision+1,hard_limit=?2,warning=?3,warned_revision=NULL WHERE scope=?1", params![change.scope.key(),change.limit.map(|n| n as i64),change.warning.map(|n| n as i64)])?;
        ensure!(reserved_edit || ordinary_capacity(&tx)?, Failure::Capacity);
        let receipt = Receipt {
            operation: change.operation,
            status: status(&tx, change.scope)?,
            reason: change.reason.clone(),
        };
        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3)",
            params![
                change.operation.to_string(),
                payload,
                bounded_json(&receipt)?
            ],
        )?;
        audit(
            &tx,
            change.scope,
            "configured",
            serde_json::to_value(&receipt)?,
        )?;
        tx.commit()?;
        Ok(receipt)
    }
    pub fn admit(&mut self, attribution: &Attribution) -> Result<Permit> {
        ensure!(
            !attribution.session.is_nil()
                && !attribution.run.is_nil()
                && attribution.agent.is_none_or(|id| !id.is_nil()),
            "invalid inference attribution"
        );
        ensure!(
            [&attribution.provider, &attribution.model]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)),
            "invalid inference provider/model label"
        );
        let project = self.session_project(attribution.session)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let scopes = [Scope::Project(project), Scope::Session(attribution.session)];
        let details: i64 = tx.query_row("SELECT count(*) FROM attempts", [], |r| r.get(0))?;
        let retained = details < MAX_DETAILS && ordinary_capacity(&tx)?;
        for scope in scopes {
            let state = status(&tx, scope)?;
            ensure!(
                retained || (state.limit.is_none() && state.warning.is_none()),
                Failure::Capacity
            );
            if state.consumed == MAX_COUNT || state.remaining() == Some(0) {
                audit(&tx, scope, "denied", serde_json::to_value(attribution)?)?;
                tx.commit()?;
                anyhow::bail!(Failure::Exhausted);
            }
        }
        let mut warnings = Vec::new();
        for scope in scopes {
            tx.execute(
                "UPDATE limits SET consumed=consumed+1,omitted=omitted+?2 WHERE scope=?1",
                params![scope.key(), i64::from(!retained)],
            )?;
            let state = status(&tx, scope)?;
            let warned: Option<i64> = tx.query_row(
                "SELECT warned_revision FROM limits WHERE scope=?1",
                [scope.key()],
                |r| r.get(0),
            )?;
            if state
                .warning
                .is_some_and(|threshold| state.consumed >= threshold)
                && warned != Some(state.revision as i64)
            {
                audit(&tx, scope, "warning", serde_json::to_value(&state)?)?;
                tx.execute(
                    "UPDATE limits SET warned_revision=revision WHERE scope=?1",
                    [scope.key()],
                )?;
                warnings.push(state);
            }
        }
        let attempt = Attempt {
            sequence: 0,
            id: Uuid::new_v4(),
            project,
            attribution: attribution.clone(),
            admitted_at: now(),
            finished_at: None,
            outcome: AttemptOutcome::Unknown,
            input_tokens: None,
            output_tokens: None,
        };
        if retained {
            tx.execute(
                "INSERT INTO attempts(id,project,session,record) VALUES(?1,?2,?3,?4)",
                params![
                    attempt.id.to_string(),
                    project.to_string(),
                    attribution.session.to_string(),
                    bounded_json(&attempt)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(Permit {
            id: attempt.id,
            retained,
            warnings,
        })
    }
    pub fn report(&mut self, id: Uuid, input: Option<u64>, output: Option<u64>) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let raw: Option<String> = tx.query_row("SELECT CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM attempts WHERE id=?1", [id.to_string()], |r| r.get(0))?;
        let mut record: Attempt = decode(raw)?;
        ensure!(
            record.id == id && record.outcome == AttemptOutcome::Unknown,
            "inference report is not for a current attempt"
        );
        // Reports are cumulative per attempt. Reject a provider counter regression.
        ensure!(
            input
                .zip(record.input_tokens)
                .is_none_or(|(next, old)| next >= old)
                && output
                    .zip(record.output_tokens)
                    .is_none_or(|(next, old)| next >= old),
            "provider usage regressed"
        );
        record.input_tokens = input.or(record.input_tokens);
        record.output_tokens = output.or(record.output_tokens);
        tx.execute(
            "UPDATE attempts SET record=?2 WHERE id=?1",
            params![id.to_string(), bounded_json(&record)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn finish_reported(&mut self, id: Uuid, outcome: AttemptOutcome) -> Result<()> {
        schema(&self.connection)?;
        let raw: Option<String> = self.connection.query_row("SELECT CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM attempts WHERE id=?1", [id.to_string()], |r| r.get(0))?;
        let record: Attempt = decode(raw)?;
        self.finish(id, outcome, record.input_tokens, record.output_tokens)
    }
    pub fn finish(
        &mut self,
        id: Uuid,
        outcome: AttemptOutcome,
        input: Option<u64>,
        output: Option<u64>,
    ) -> Result<()> {
        ensure!(
            outcome != AttemptOutcome::Unknown,
            "cannot erase an attempt outcome"
        );
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let raw: Option<String> = tx.query_row("SELECT CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM attempts WHERE id=?1", [id.to_string()], |r| r.get(0))?;
        let mut record: Attempt = decode(raw)?;
        ensure!(record.id == id, "inference attempt identity mismatch");
        if record.outcome != AttemptOutcome::Unknown {
            ensure!(
                record.outcome == outcome
                    && record.input_tokens == input
                    && record.output_tokens == output,
                "inference outcome conflict"
            );
            return Ok(());
        }
        record.outcome = outcome;
        record.input_tokens = input;
        record.output_tokens = output;
        record.finished_at = Some(now());
        tx.execute(
            "UPDATE attempts SET record=?2 WHERE id=?1",
            params![id.to_string(), bounded_json(&record)?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn attempts(&self, scope: Scope, after: u64, limit: u32) -> Result<Vec<Attempt>> {
        schema(&self.connection)?;
        ensure!(
            after <= MAX_COUNT && (1..=100).contains(&limit),
            "invalid inference page"
        );
        let (column, id) = match scope {
            Scope::Project(id) => ("project", id),
            Scope::Session(id) => ("session", id),
        };
        let sql = format!(
            "SELECT sequence,CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM attempts WHERE {column}=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let rows = statement.query_map(params![id.to_string(), after as i64, limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.map(|row| {
            let (sequence, record) = row?;
            let mut record: Attempt = decode(record)?;
            ensure!(
                match scope {
                    Scope::Project(id) => record.project == id,
                    Scope::Session(id) => record.attribution.session == id,
                },
                "inference attempt attribution mismatch"
            );
            record.sequence = counter(sequence)?;
            Ok(record)
        })
        .collect()
    }
    pub fn audit(&self, scope: Scope, after: u64, limit: u32) -> Result<Vec<Audit>> {
        schema(&self.connection)?;
        ensure!(
            after <= MAX_COUNT && (1..=100).contains(&limit),
            "invalid inference page"
        );
        let mut statement = self.connection.prepare("SELECT sequence,CASE WHEN length(CAST(record AS BLOB))<=16384 THEN record END FROM audit WHERE scope=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3")?;
        let rows = statement.query_map(params![scope.key(), after as i64, limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
        })?;
        rows.map(|row| {
            let (sequence, record) = row?;
            let mut record: Audit = decode(record)?;
            ensure!(record.scope == scope, "inference audit scope mismatch");
            record.sequence = counter(sequence)?;
            Ok(record)
        })
        .collect()
    }
}
