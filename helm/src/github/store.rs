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
    pub state: State,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub receipt: Option<Receipt>,
    /// Explicit human disposition preserves the uncertain operation and is not a retry.
    pub disposition: Option<String>,
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
                &operation.owner
            )?,
        "GitHub operation binding is corrupt"
    );
    ensure!(
        operation.expires_at > operation.created_at,
        "GitHub operation expiry is corrupt"
    );
    ensure!(
        (operation.state == State::Published) == operation.receipt.is_some(),
        "GitHub operation receipt state is corrupt"
    );
    Ok(operation)
}

fn schema(connection: &Connection) -> Result<()> {
    let version: i64 =
        connection.query_row("SELECT version FROM github_schema WHERE id=1", [], |row| {
            row.get(0)
        })?;
    ensure!(version == 1, "unsupported GitHub operation schema");
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
        connection.execute_batch("PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA temp_store=MEMORY; PRAGMA max_page_count=24576;")?;
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
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='operations'",
                [],
                |row| row.get(0),
            )?;
            ensure!(
                operations == 1,
                "GitHub operation table is missing; restore the private store"
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
        owner: Owner,
    ) -> Result<Operation> {
        let scope = owner.scope()?;
        let digest = operation_digest(
            &draft,
            &actor,
            &policy_digest,
            observed_head.as_deref(),
            &owner,
        )?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        schema(&tx)?;
        let existing: Option<(String, String, String)> = tx.query_row("SELECT id,state,data FROM operations WHERE digest=?1 AND owner=?2 AND state NOT IN ('cancelled','disposed') ORDER BY rowid LIMIT 1", params![digest,scope], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
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
        let now = Utc::now();
        let operation = Operation {
            id: Uuid::new_v4(),
            digest,
            policy_digest,
            owner,
            actor,
            draft,
            observed_head,
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
            "SELECT digest,state,data FROM operations WHERE id=?1 AND owner=?2",
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
        let operation = self.inspect(id, owner)?;
        ensure!(
            operation.digest == digest
                && matches!(
                    operation.state,
                    State::Published | State::Cancelled | State::Disposed
                ),
            "only exact terminal GitHub records may be forgotten"
        );
        let changed = self.connection.execute("DELETE FROM operations WHERE id=?1 AND digest=?2 AND state IN ('published','cancelled','disposed')", params![id.to_string(), digest])?;
        ensure!(changed == 1, "GitHub operation changed before forgetting");
        Ok(())
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
            "SELECT digest,state,data FROM operations WHERE id=?1 AND owner=?2",
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
    owner.scope()?;
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(&(
        draft.digest(actor, policy_digest)?,
        head,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::{publication::Action, repository::Object};
    fn fixture() -> (tempfile::TempDir, Store, Owner, Draft, Actor) {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path().join("operations")).unwrap();
        let owner = Owner::new(temp.path(), Some(Uuid::new_v4()), Some(Uuid::new_v4())).unwrap();
        let draft = Draft {
            object: Object::parse("https://github.com/o/r/issues/1").unwrap(),
            action: Action::Comment {
                body: "Review this exact text".into(),
            },
        };
        let actor = Actor {
            id: 1,
            login: "alice".into(),
        };
        (temp, store, owner, draft, actor)
    }
    #[test]
    fn sending_survives_reopen_and_never_replays_or_cancels_as_unsent() {
        let (temp, mut store, owner, draft, actor) = fixture();
        let operation = store
            .prepare(
                draft.clone(),
                actor.clone(),
                "policy".into(),
                None,
                owner.clone(),
            )
            .unwrap();
        store.begin_send(&operation).unwrap();
        assert!(store.begin_send(&operation).is_err());
        assert!(
            store
                .cancel(operation.id, &operation.digest, &owner)
                .is_err()
        );
        drop(store);
        let mut store = Store::open(temp.path().join("operations")).unwrap();
        assert_eq!(
            store.inspect(operation.id, &owner).unwrap().state,
            State::Sending
        );
        let existing = store
            .prepare(draft, actor, "policy".into(), None, owner)
            .unwrap();
        assert_eq!(existing.id, operation.id);
        assert_eq!(existing.state, State::Sending);
    }
    #[test]
    fn copied_ids_do_not_cross_voyage_scope_or_adopt_its_state() {
        let (_temp, mut store, owner, draft, actor) = fixture();
        let operation = store
            .prepare(
                draft.clone(),
                actor.clone(),
                "policy".into(),
                None,
                owner.clone(),
            )
            .unwrap();
        let other = Owner {
            session: Some(Uuid::new_v4()),
            ..owner.clone()
        };
        assert!(store.inspect(operation.id, &other).is_err());
        assert!(store.list(&other, 0).unwrap().is_empty());
        assert!(
            store
                .cancel(operation.id, &operation.digest, &other)
                .is_err()
        );
        assert!(
            store
                .forget(operation.id, &operation.digest, &other)
                .is_err()
        );
        let independent = store
            .prepare(draft, actor, "policy".into(), None, other)
            .unwrap();
        assert_ne!(operation.id, independent.id);
        assert_ne!(operation.digest, independent.digest);
    }
    #[test]
    fn expired_preparation_can_be_cancelled_but_not_sent() {
        let (_temp, mut store, owner, draft, actor) = fixture();
        let mut operation = store
            .prepare(draft, actor, "policy".into(), None, owner.clone())
            .unwrap();
        operation.created_at = Utc::now() - chrono::Duration::minutes(30);
        operation.expires_at = Utc::now() - chrono::Duration::minutes(15);
        store
            .connection
            .execute(
                "UPDATE operations SET data=?1 WHERE id=?2",
                params![
                    serde_json::to_string(&operation).unwrap(),
                    operation.id.to_string()
                ],
            )
            .unwrap();
        assert!(store.begin_send(&operation).is_err());
        assert_eq!(
            store
                .cancel(operation.id, &operation.digest, &owner)
                .unwrap()
                .state,
            State::Cancelled
        );
    }
    #[test]
    fn future_or_missing_schema_does_not_reset_unknown_delivery() {
        let (temp, mut store, owner, draft, actor) = fixture();
        let operation = store
            .prepare(draft, actor, "policy".into(), None, owner.clone())
            .unwrap();
        store.begin_send(&operation).unwrap();
        store
            .connection
            .execute("UPDATE github_schema SET version=999", [])
            .unwrap();
        assert!(store.inspect(operation.id, &owner).is_err());
        assert!(store.begin_send(&operation).is_err());
        drop(store);
        assert!(Store::open(temp.path().join("operations")).is_err());
        let connection = Connection::open(temp.path().join("operations/journal.sqlite3")).unwrap();
        connection.execute("DELETE FROM github_schema", []).unwrap();
        drop(connection);
        assert!(Store::open(temp.path().join("operations")).is_err());
    }
    #[test]
    fn missing_existing_operation_table_is_not_recreated() {
        let (temp, store, _owner, _draft, _actor) = fixture();
        store
            .connection
            .execute("DROP TABLE operations", [])
            .unwrap();
        drop(store);
        assert!(Store::open(temp.path().join("operations")).is_err());
    }
}
