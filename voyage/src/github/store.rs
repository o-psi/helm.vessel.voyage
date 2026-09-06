//! Private exact-operation state. A committed Sending intent is never retried.
use super::publication::{Actor, Draft};
use anyhow::{Result, ensure};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

const MAX_OPERATIONS: u64 = 512;
const MAX_RECORD_BYTES: usize = 128 * 1024;
const MAX_AUDIT: u64 = 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audit {
    pub id: Uuid,
    pub digest: String,
    pub owner: Owner,
    pub object: super::repository::Object,
    pub former_state: State,
    pub receipt: Option<Receipt>,
    pub disposition: Option<String>,
    pub forgotten_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditSnapshot {
    pub digest: String,
    pub entries: Vec<Audit>,
}

fn audit_snapshot(connection: &Connection) -> Result<AuditSnapshot> {
    use sha2::{Digest, Sha256};
    schema(connection)?;
    let count: u64 = connection.query_row("SELECT count(*) FROM audit", [], |row| row.get(0))?;
    ensure!(count <= MAX_AUDIT, "GitHub audit exceeds capacity");
    let mut statement = connection.prepare("SELECT id,digest,CASE WHEN length(CAST(data AS BLOB))<=4096 THEN data ELSE NULL END FROM audit ORDER BY sequence")?;
    let mut rows = statement.query([])?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let digest: String = row.get(1)?;
        let text: String = row.get(2)?;
        let entry: Audit =
            serde_json::from_str(&text).map_err(|_| anyhow::anyhow!("GitHub audit is corrupt"))?;
        ensure!(
            entry.id.to_string() == id
                && entry.digest == digest
                && digest.len() == 64
                && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                && matches!(
                    entry.former_state,
                    State::Published | State::Cancelled | State::Disposed
                ),
            "GitHub audit identity is corrupt"
        );
        entry.owner.scope()?;
        entry.object.validate()?;
        ensure!(
            (entry.former_state == State::Published) == entry.receipt.is_some()
                && (entry.former_state == State::Disposed) == entry.disposition.is_some(),
            "GitHub audit terminal evidence is corrupt"
        );
        if let Some(note) = &entry.disposition {
            ensure!(
                !note.trim().is_empty() && note.len() <= 1024,
                "GitHub audit disposition is corrupt"
            );
        }
        if let Some(receipt) = &entry.receipt {
            let prefix = entry.object.url();
            ensure!(
                receipt.id > 0
                    && receipt.id <= i64::MAX as u64
                    && (receipt.url == format!("{prefix}#issuecomment-{}", receipt.id)
                        || (entry.object.kind == super::repository::ObjectKind::PullRequest
                            && receipt.url
                                == format!("{prefix}#pullrequestreview-{}", receipt.id))),
                "GitHub audit receipt is corrupt"
            );
        }
        entries.push(entry);
    }
    let digest = hex::encode(Sha256::digest(serde_json::to_vec(&entries)?));
    Ok(AuditSnapshot { digest, entries })
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Prepared,
    Sending,
    Published,
    Cancelled,
    Disposed,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub id: u64,
    pub url: String,
    pub evidence: ReceiptEvidence,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptEvidence {
    ApiResponse,
    OperatorAdopted,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Owner {
    pub workspace: String,
    pub session: Option<Uuid>,
    pub run: Option<Uuid>,
}
impl Owner {
    pub fn new(
        workspace: &std::path::Path,
        session: Option<Uuid>,
        run: Option<Uuid>,
    ) -> Result<Self> {
        use sha2::{Digest, Sha256};
        let workspace = workspace.canonicalize()?;
        ensure!(workspace.is_dir(), "GitHub workspace is unavailable");
        ensure!(
            run.is_none() || session.is_some(),
            "GitHub run requires an owning voyage"
        );
        Ok(Self {
            workspace: hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes())),
            session,
            run,
        })
    }
    fn scope(&self) -> Result<String> {
        ensure!(
            self.workspace.len() == 64
                && self.workspace.bytes().all(|byte| byte.is_ascii_hexdigit())
                && self.session.is_none_or(|id| !id.is_nil())
                && self.run.is_none_or(|id| !id.is_nil())
                && (self.run.is_none() || self.session.is_some()),
            "invalid GitHub operation owner"
        );
        Ok(serde_json::to_string(&(&self.workspace, self.session))?)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub id: Uuid,
    pub digest: String,
    pub policy_digest: String,
    pub owner: Owner,
    pub actor: Actor,
    pub draft: Draft,
    pub observed_head: Option<String>,
    pub observed_base: Option<super::context::Base>,
    pub state: State,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub receipt: Option<Receipt>,
    /// Explicit human disposition preserves the uncertain operation and is not a retry.
    pub disposition: Option<String>,
}
impl Operation {
    pub(super) fn snapshot_digest(&self) -> Result<String> {
        use sha2::{Digest, Sha256};
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(self)?)))
    }
}

pub struct Store {
    connection: Connection,
    #[cfg(windows)]
    _private: std::sync::Arc<voyage_storage::PrivateDirectory>,
}

fn decode(text: &str) -> Result<Operation> {
    ensure!(
        text.len() <= MAX_RECORD_BYTES,
        "GitHub operation record exceeds limit"
    );
    let operation: Operation = serde_json::from_str(text)
        .map_err(|_| anyhow::anyhow!("GitHub operation record is corrupt"))?;
    ensure!(
        !operation.id.is_nil()
            && operation.actor.id > 0
            && !operation.actor.login.is_empty()
            && operation.actor.login.len() <= 100,
        "GitHub operation identity is corrupt"
    );
    operation.owner.scope()?;
    ensure!(
        operation.digest
            == operation_digest(
                &operation.draft,
                &operation.actor,
                &operation.policy_digest,
                operation.observed_head.as_deref(),
                operation.observed_base.as_ref(),
                &operation.owner
            )?,
        "GitHub operation binding is corrupt"
    );
    ensure!(
        operation
            .created_at
            .checked_add_signed(chrono::Duration::minutes(15))
            == Some(operation.expires_at),
        "GitHub operation expiry is corrupt"
    );
    ensure!(
        (operation.state == State::Published) == operation.receipt.is_some(),
        "GitHub operation receipt state is corrupt"
    );
    ensure!(
        (operation.state == State::Disposed) == operation.disposition.is_some(),
        "GitHub operation disposition state is corrupt"
    );
    if let Some(note) = &operation.disposition {
        ensure!(
            !note.trim().is_empty() && note.len() <= 1024,
            "GitHub operation disposition is corrupt"
        );
    }
    if let Some(receipt) = &operation.receipt {
        let marker = match operation.draft.action {
            super::publication::Action::Comment { .. } => "issuecomment",
            super::publication::Action::Review { .. } => "pullrequestreview",
        };
        ensure!(
            receipt.id > 0
                && receipt.id <= i64::MAX as u64
                && receipt.url
                    == format!("{}#{marker}-{}", operation.draft.object.url(), receipt.id),
            "GitHub operation receipt identity is corrupt"
        );
    }
    Ok(operation)
}

fn schema(connection: &Connection) -> Result<()> {
    let version: i64 =
        connection.query_row("SELECT version FROM github_schema WHERE id=1", [], |row| {
            row.get(0)
        })?;
    ensure!(version == 1, "unsupported GitHub operation schema");
    for (table, expected) in [
        (
            "github_schema",
            vec![("id", "INTEGER", 0, 1), ("version", "INTEGER", 1, 0)],
        ),
        (
            "operations",
            vec![
                ("id", "TEXT", 0, 1),
                ("digest", "TEXT", 1, 0),
                ("owner", "TEXT", 1, 0),
                ("state", "TEXT", 1, 0),
                ("data", "TEXT", 1, 0),
            ],
        ),
        (
            "audit",
            vec![
                ("sequence", "INTEGER", 0, 1),
                ("id", "TEXT", 1, 0),
                ("digest", "TEXT", 1, 0),
                ("data", "TEXT", 1, 0),
            ],
        ),
    ] {
        let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
        let columns = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(5)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            columns.len() == expected.len()
                && columns
                    .iter()
                    .zip(expected)
                    .all(|(actual, expected)| actual.0 == expected.0
                        && actual.1 == expected.1
                        && actual.2 == expected.2
                        && actual.3 == expected.3),
            "GitHub journal table contract is corrupt; restore its private evidence"
        );
    }
    Ok(())
}

impl Store {
    pub fn default_path() -> PathBuf {
        crate::config::default_data_dir().join("github")
    }
    pub fn open(directory: PathBuf) -> Result<Self> {
        #[cfg(windows)]
        let (directory, private) = crate::attachment::journal::storage::prepare(directory)?;
        #[cfg(not(windows))]
        let directory = crate::attachment::journal::prepare_directory(directory)?;
        let path = directory.join("journal.sqlite3");
        drop(crate::attachment::journal::open_private_file(&path)?);
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(Duration::ZERO)?;
        #[cfg(windows)]
        crate::attachment::journal::storage::configure(&connection)?;
        connection.execute_batch(
            "PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY;",
        )?;
        let page_size: u64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        ensure!(
            (512..=65536).contains(&page_size),
            "GitHub store page size is unsupported"
        );
        let maximum_pages = 96 * 1024 * 1024 / page_size;
        let actual_maximum: u64 = connection.query_row(
            &format!("PRAGMA max_page_count={maximum_pages}"),
            [],
            |row| row.get(0),
        )?;
        ensure!(
            actual_maximum <= maximum_pages,
            "GitHub store exceeds its physical capacity"
        );
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS github_schema(id INTEGER PRIMARY KEY CHECK(id=1), version INTEGER NOT NULL);")?;
        let version: Option<i64> = tx
            .query_row("SELECT version FROM github_schema WHERE id=1", [], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(version) = version {
            ensure!(version == 1, "unsupported GitHub operation schema");
            let operations: u64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('operations','audit')",
                [],
                |row| row.get(0),
            )?;
            ensure!(
                operations == 2,
                "GitHub operation or audit table is missing; restore the private store"
            );
        } else {
            let tables: u64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name!='github_schema'",
                [],
                |row| row.get(0),
            )?;
            ensure!(tables == 0, "GitHub store schema identity is missing");
            tx.execute("INSERT INTO github_schema VALUES(1,1)", [])?;
        }
        tx.execute_batch("CREATE TABLE IF NOT EXISTS operations(id TEXT PRIMARY KEY, digest TEXT NOT NULL, owner TEXT NOT NULL, state TEXT NOT NULL, data TEXT NOT NULL); CREATE INDEX IF NOT EXISTS operation_digest ON operations(digest);")?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS audit(sequence INTEGER PRIMARY KEY, id TEXT NOT NULL, digest TEXT NOT NULL, data TEXT NOT NULL);")?;
        schema(&tx)?;
        tx.commit()?;
        #[cfg(windows)]
        crate::attachment::journal::storage::verify(&private)?;
        Ok(Self {
            connection,
            #[cfg(windows)]
            _private: private,
        })
    }

    pub fn prepare(
        &mut self,
        draft: Draft,
        actor: Actor,
        policy_digest: String,
        observed_head: Option<String>,
        observed_base: Option<super::context::Base>,
        owner: Owner,
    ) -> Result<Operation> {
        let scope = owner.scope()?;
        let digest = operation_digest(
            &draft,
            &actor,
            &policy_digest,
            observed_head.as_deref(),
            observed_base.as_ref(),
            &owner,
        )?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let existing: Option<(String, String, String)> = tx.query_row("SELECT id,state,CASE WHEN length(CAST(data AS BLOB))<=131072 THEN data ELSE NULL END FROM operations WHERE digest=?1 AND owner=?2 AND state NOT IN ('cancelled','disposed') ORDER BY rowid LIMIT 1", params![digest,scope], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
        if let Some((id, state, existing)) = existing {
            let operation = decode(&existing)?;
            ensure!(
                operation.id.to_string() == id
                    && operation.digest == digest
                    && state == state_name(&operation.state),
                "GitHub operation index is corrupt"
            );
            return Ok(operation);
        }
        let count: u64 = tx.query_row("SELECT count(*) FROM operations", [], |row| row.get(0))?;
        ensure!(
            count < MAX_OPERATIONS,
            "GitHub operation capacity reached; inspect and forget terminal records explicitly"
        );
        let audit_count: u64 = tx.query_row("SELECT count(*) FROM audit", [], |row| row.get(0))?;
        ensure!(
            audit_count < MAX_AUDIT - MAX_OPERATIONS,
            "GitHub audit admission capacity reached; export and explicitly maintain the audit before preparing more operations"
        );
        let now = Utc::now();
        let operation = Operation {
            id: Uuid::new_v4(),
            digest,
            policy_digest,
            owner,
            actor,
            draft,
            observed_head,
            observed_base,
            state: State::Prepared,
            created_at: now,
            expires_at: now + chrono::Duration::minutes(15),
            receipt: None,
            disposition: None,
        };
        let data = serde_json::to_string(&operation)?;
        decode(&data)?;
        tx.execute(
            "INSERT INTO operations VALUES(?1,?2,?3,'prepared',?4)",
            params![operation.id.to_string(), operation.digest, scope, data],
        )?;
        tx.commit()?;
        Ok(operation)
    }

    pub fn inspect(&self, id: Uuid, owner: &Owner) -> Result<Operation> {
        schema(&self.connection)?;
        let (digest, state, data): (String, String, String) = self.connection.query_row(
            "SELECT digest,state,CASE WHEN length(CAST(data AS BLOB))<=131072 THEN data ELSE NULL END FROM operations WHERE id=?1 AND owner=?2",
            params![id.to_string(), owner.scope()?],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let operation = decode(&data)?;
        ensure!(
            operation.id == id
                && operation.digest == digest
                && state == state_name(&operation.state)
                && operation.owner.scope()? == owner.scope()?,
            "GitHub operation index is corrupt"
        );
        Ok(operation)
    }

    pub fn list(&self, owner: &Owner, offset: u32) -> Result<Vec<Operation>> {
        schema(&self.connection)?;
        ensure!(
            offset <= MAX_OPERATIONS as u32,
            "GitHub operation offset exceeds limit"
        );
        let mut statement = self.connection.prepare(
            "SELECT id FROM operations WHERE owner=?1 ORDER BY rowid DESC LIMIT 50 OFFSET ?2",
        )?;
        let ids = statement
            .query_map(params![owner.scope()?, offset], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| self.inspect(Uuid::parse_str(&id)?, owner))
            .collect()
    }

    /// Global ownership bypass is confined to the attended operator adapter.
    pub(super) fn admin_inspect(&self, id: Uuid) -> Result<Operation> {
        schema(&self.connection)?;
        let data: String = self.connection.query_row(
            "SELECT CASE WHEN length(CAST(data AS BLOB))<=131072 THEN data ELSE NULL END FROM operations WHERE id=?1",
            [id.to_string()], |row| row.get(0))?;
        let operation = decode(&data)?;
        self.inspect(id, &operation.owner)
    }

    pub(super) fn admin_list(&self, offset: u32) -> Result<Vec<Operation>> {
        schema(&self.connection)?;
        ensure!(
            offset <= MAX_OPERATIONS as u32,
            "GitHub operation offset exceeds limit"
        );
        let mut statement = self
            .connection
            .prepare("SELECT id FROM operations ORDER BY rowid DESC LIMIT 50 OFFSET ?1")?;
        let ids = statement
            .query_map([offset], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter()
            .map(|id| self.admin_inspect(Uuid::parse_str(&id)?))
            .collect()
    }

    pub(super) fn audit(&self) -> Result<AuditSnapshot> {
        audit_snapshot(&self.connection)
    }

    /// The operator must export and confirm this exact snapshot first. This
    /// removes local audit evidence, never authorizes a remote request retry.
    pub(super) fn clear_audit(&mut self, expected: &str) -> Result<u64> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let snapshot = audit_snapshot(&tx)?;
        ensure!(
            snapshot.digest == expected,
            "GitHub audit changed; export the current snapshot before maintenance"
        );
        let removed = tx.execute("DELETE FROM audit", [])?;
        tx.commit()?;
        Ok(removed as u64)
    }

    /// Only call after the current exact attended approval and all revalidation.
    pub(super) fn begin_send(&mut self, expected: &Operation) -> Result<Operation> {
        self.transition(
            expected.id,
            &expected.digest,
            &expected.owner,
            |operation| {
                ensure!(
                    operation.state == State::Prepared && operation.expires_at > Utc::now(),
                    "GitHub operation is expired, cancelled or already sent"
                );
                operation.state = State::Sending;
                Ok(())
            },
        )
    }
    pub(super) fn publish(&mut self, expected: &Operation, receipt: Receipt) -> Result<Operation> {
        self.transition(
            expected.id,
            &expected.digest,
            &expected.owner,
            |operation| {
                ensure!(
                    operation.state == State::Sending,
                    "GitHub operation is no longer awaiting a receipt"
                );
                operation.state = State::Published;
                operation.receipt = Some(receipt);
                Ok(())
            },
        )
    }
    pub fn cancel(&mut self, id: Uuid, digest: &str, owner: &Owner) -> Result<Operation> {
        self.transition(id, digest, owner, |operation| {
            ensure!(
                operation.state == State::Prepared,
                "possibly sent GitHub operation cannot be cancelled as unsent"
            );
            operation.state = State::Cancelled;
            Ok(())
        })
    }
    pub(super) fn dispose(
        &mut self,
        id: Uuid,
        digest: &str,
        owner: &Owner,
        note: String,
    ) -> Result<Operation> {
        ensure!(
            !note.trim().is_empty() && note.len() <= 1024,
            "GitHub disposition requires a bounded note"
        );
        self.transition(id, digest, owner, |operation| {
            ensure!(
                operation.state == State::Sending,
                "only uncertain GitHub operations need disposition"
            );
            operation.state = State::Disposed;
            operation.disposition = Some(note);
            Ok(())
        })
    }
    pub fn forget(&mut self, id: Uuid, digest: &str, owner: &Owner) -> Result<()> {
        self.forget_checked(id, digest, owner, None)
    }
    pub(super) fn forget_exact(&mut self, expected: &Operation) -> Result<()> {
        self.forget_checked(
            expected.id,
            &expected.digest,
            &expected.owner,
            Some(expected.snapshot_digest()?),
        )
    }
    fn forget_checked(
        &mut self,
        id: Uuid,
        digest: &str,
        owner: &Owner,
        snapshot: Option<String>,
    ) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let (indexed_digest, indexed_state, data): (String, String, String) = tx.query_row(
            "SELECT digest,state,CASE WHEN length(CAST(data AS BLOB))<=131072 THEN data ELSE NULL END FROM operations WHERE id=?1 AND owner=?2",
            params![id.to_string(),owner.scope()?], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        let operation = decode(&data)?;
        ensure!(
            snapshot
                .as_ref()
                .is_none_or(|snapshot| operation.snapshot_digest().as_ref().ok() == Some(snapshot)),
            "GitHub operation changed after preview; confirm the current snapshot"
        );
        ensure!(
            operation.id == id
                && operation.owner.scope()? == owner.scope()?
                && indexed_digest == digest
                && indexed_state == state_name(&operation.state)
                && operation.digest == digest
                && matches!(
                    operation.state,
                    State::Published | State::Cancelled | State::Disposed
                ),
            "only exact terminal GitHub records may be forgotten"
        );
        let count: u64 = tx.query_row("SELECT count(*) FROM audit", [], |row| row.get(0))?;
        ensure!(
            count < MAX_AUDIT,
            "GitHub audit is full; export and explicitly maintain it before forgetting records"
        );
        let audit = Audit {
            id,
            digest: digest.into(),
            owner: operation.owner,
            object: operation.draft.object,
            former_state: operation.state,
            receipt: operation.receipt,
            disposition: operation.disposition,
            forgotten_at: Utc::now(),
        };
        let audit = serde_json::to_string(&audit)?;
        ensure!(audit.len() <= 4096, "GitHub audit record exceeds limit");
        tx.execute(
            "INSERT INTO audit(id,digest,data) VALUES(?1,?2,?3)",
            params![id.to_string(), digest, audit],
        )?;
        let changed = tx.execute("DELETE FROM operations WHERE id=?1 AND digest=?2 AND owner=?3 AND state IN ('published','cancelled','disposed')", params![id.to_string(), digest,owner.scope()?])?;
        ensure!(changed == 1, "GitHub operation changed before forgetting");
        tx.commit()?;
        Ok(())
    }
    pub(super) fn cancel_exact(&mut self, expected: &Operation) -> Result<Operation> {
        let snapshot = expected.snapshot_digest()?;
        self.transition(
            expected.id,
            &expected.digest,
            &expected.owner,
            move |operation| {
                ensure!(
                    operation.snapshot_digest()? == snapshot,
                    "GitHub operation changed after preview"
                );
                ensure!(
                    operation.state == State::Prepared,
                    "only prepared GitHub operations can be cancelled"
                );
                operation.state = State::Cancelled;
                Ok(())
            },
        )
    }
    pub(super) fn dispose_exact(
        &mut self,
        expected: &Operation,
        note: String,
    ) -> Result<Operation> {
        ensure!(
            !note.trim().is_empty() && note.len() <= 1024,
            "GitHub disposition requires a bounded note"
        );
        let snapshot = expected.snapshot_digest()?;
        self.transition(
            expected.id,
            &expected.digest,
            &expected.owner,
            move |operation| {
                ensure!(
                    operation.snapshot_digest()? == snapshot,
                    "GitHub operation changed after preview"
                );
                ensure!(
                    operation.state == State::Sending,
                    "only uncertain GitHub operations need disposition"
                );
                operation.state = State::Disposed;
                operation.disposition = Some(note);
                Ok(())
            },
        )
    }
    fn transition(
        &mut self,
        id: Uuid,
        digest: &str,
        owner: &Owner,
        change: impl FnOnce(&mut Operation) -> Result<()>,
    ) -> Result<Operation> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let (indexed_digest, indexed_state, data): (String, String, String) = tx.query_row(
            "SELECT digest,state,CASE WHEN length(CAST(data AS BLOB))<=131072 THEN data ELSE NULL END FROM operations WHERE id=?1 AND owner=?2",
            params![id.to_string(), owner.scope()?],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        let mut operation = decode(&data)?;
        ensure!(
            operation.id == id
                && operation.digest == digest
                && indexed_digest == digest
                && indexed_state == state_name(&operation.state)
                && operation.owner.scope()? == owner.scope()?,
            "GitHub operation changed or is corrupt"
        );
        change(&mut operation)?;
        let data = serde_json::to_string(&operation)?;
        decode(&data)?;
        tx.execute(
            "UPDATE operations SET state=?1,data=?2 WHERE id=?3",
            params![state_name(&operation.state), data, id.to_string()],
        )?;
        tx.commit()?;
        Ok(operation)
    }
}
fn operation_digest(
    draft: &Draft,
    actor: &Actor,
    policy_digest: &str,
    head: Option<&str>,
    base: Option<&super::context::Base>,
    owner: &Owner,
) -> Result<String> {
    use sha2::{Digest, Sha256};
    ensure!(
        (draft.object.kind == super::repository::ObjectKind::PullRequest) == head.is_some(),
        "GitHub operation head binding is missing or unexpected"
    );
    if let Some(head) = head {
        super::context::sha(&serde_json::Value::String(head.into()))?;
    }
    ensure!(
        head.is_some() == base.is_some(),
        "GitHub operation base binding is missing or unexpected"
    );
    if let Some(base) = base {
        base.validate()?;
    }
    owner.scope()?;
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&(
        draft.digest(actor, policy_digest)?,
        head,
        base,
        owner,
    ))?)))
}
fn state_name(state: &State) -> &'static str {
    match state {
        State::Prepared => "prepared",
        State::Sending => "sending",
        State::Published => "published",
        State::Cancelled => "cancelled",
        State::Disposed => "disposed",
    }
}
