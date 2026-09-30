//! Embedded supervisor storage. Canonical conversations remain in Voyage journals.
//! Files required by old/live runtimes are derived registration projections.
use super::registry;
use anyhow::{Context, Result, ensure};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::execution_identity::{
    AdministratorGrant, AuthorityClass, ConfiguredExecutionIdentity, ExecutionBinding,
};
use voyage_protocol::process::{
    CatalogueMetadata, CatalogueSummary, ProcessInfo, ProcessRegistration,
};

#[cfg(target_os = "linux")]
#[path = "database_execution_reviews.rs"]
pub(super) mod execution_reviews;

pub(super) const CATALOGUE_READ_SCHEMAS: &[i64] = &[1, 2, 3];
pub(super) const CATALOGUE_WRITE_SCHEMAS: &[i64] = &[2, 3];

const FILE: &str = "catalogue.sqlite3";
fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn private_file(path: &Path) -> Result<()> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.is_file()
            && m.nlink() == 1
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "unsafe supervisor database file"
    );
    Ok(())
}
pub(super) fn open(root: &Path) -> Result<Connection> {
    open_file(root, FILE)
}
fn open_file(root: &Path, file: &str) -> Result<Connection> {
    #[cfg(target_os = "linux")]
    if unsafe { libc::geteuid() } == 0 {
        // A root supervisor may not open SQLite below a user-controlled path.
        // The legacy user service retains its existing private-directory check.
        let _control = voyage_storage::protected_linux::RootDirectory::open(root)?;
    }
    registry::private_directory(root)?;
    for name in [file.to_owned(), format!("{file}-journal")] {
        private_file(&root.join(name))?;
    }
    for name in [format!("{file}-wal"), format!("{file}-shm")] {
        ensure!(
            !root.join(name).try_exists()?,
            "unexpected supervisor WAL state"
        );
    }
    let mut db = Connection::open_with_flags(
        root.join(file),
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=PERSIST; PRAGMA synchronous=FULL; PRAGMA journal_size_limit=1048576;")?;
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='schema_version')",
        [],
        |r| r.get(0),
    )?;
    if !exists {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let initialized: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='schema_version')",
            [],
            |r| r.get(0),
        )?;
        if !initialized {
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )?;
            ensure!(count == 0, "unrecognized supervisor database");
            tx.execute_batch(include_str!("database.sql"))?;
        }
        tx.commit()?;
        fs::File::open(root)?.sync_all()?;
    }
    let page_size: u64 = db.pragma_query_value(None, "page_size", |r| r.get(0))?;
    ensure!(page_size > 0, "invalid supervisor database page size");
    db.pragma_update(None, "max_page_count", (512 * 1024 * 1024u64) / page_size)?;
    let version: i64 = db.query_row("SELECT version FROM schema_version WHERE id=1", [], |r| {
        r.get(0)
    })?;
    ensure!(CATALOGUE_READ_SCHEMAS.contains(&version), "unsupported supervisor database version");
    if version == 1 {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(include_str!("database_v2_migration.sql"))?;
        tx.commit()?;
        fs::File::open(root)?.sync_all()?;
    } else {
        ensure!(
            CATALOGUE_WRITE_SCHEMAS.contains(&version),
            "unsupported supervisor database version"
        );
    }
    Ok(db)
}
fn save_tx(
    tx: &Transaction<'_>,
    registration: &ProcessRegistration,
    new_binding: Option<&ExecutionBinding>,
) -> Result<()> {
    let bound: Option<String> = tx
        .query_row(
            "SELECT record FROM execution_bindings WHERE session_id=?1",
            [registration.session_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let bound: Option<ExecutionBinding> = bound
        .map(|record| serde_json::from_str(&record))
        .transpose()?;
    ensure!(
        bound.as_ref().is_none_or(|binding| {
            binding.incarnation == registration.incarnation
                && registration.peer_uids.as_ref() == Some(&binding.peer_uids)
        }),
        "execution identity replacement requires a new protected binding"
    );
    ensure!(
        bound.is_some() || new_binding.is_some() || registration.peer_uids.is_none(),
        "cross-identity registration requires a protected binding"
    );
    let previous: Option<String> = tx
        .query_row(
            "SELECT session_id FROM incarnations WHERE incarnation=?1",
            [registration.incarnation.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(
        previous
            .as_ref()
            .is_none_or(|id| id == &registration.session_id.to_string()),
        "incarnation belongs to another voyage"
    );
    let current: Option<String> = tx
        .query_row(
            "SELECT incarnation FROM voyages WHERE session_id=?1",
            [registration.session_id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    ensure!(
        current
            .as_ref()
            .is_none_or(|current| current == &registration.incarnation.to_string()
                || (previous.is_none()
                    && registration.restart_from.map(|id| id.to_string()).as_ref()
                        == Some(current))),
        "stale incarnation publication"
    );
    let old_record: Option<String> = tx
        .query_row(
            "SELECT registration FROM voyages WHERE session_id=?1",
            [registration.session_id.to_string()],
            |row| row.get(0),
        )
        .optional()?;
    let bytes = serde_json::to_string(registration)?;
    ensure!(bytes.len() < 16384, "registration exceeds limit");
    let id = registration.session_id.to_string();
    let inc = registration.incarnation.to_string();
    let state = serde_json::to_value(&registration.state)?
        .as_str()
        .context("invalid process state")?
        .to_owned();
    tx.execute("INSERT INTO voyages VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(session_id) DO UPDATE SET incarnation=excluded.incarnation,workspace=excluded.workspace,state=excluded.state,name=excluded.name,registration=excluded.registration,updated_at_ms=excluded.updated_at_ms",params![id,inc,registration.workspace.as_os_str().as_encoded_bytes(),state,registration.name,bytes,now()])?;
    tx.execute("INSERT INTO incarnations VALUES(?1,?2,?3,?4,?5) ON CONFLICT(incarnation) DO UPDATE SET registration=excluded.registration",params![inc,id,registration.command_id.to_string(),bytes,now()])?;
    tx.execute("INSERT INTO catalogue(session_id,incarnation) VALUES(?1,?2) ON CONFLICT(session_id) DO UPDATE SET incarnation=excluded.incarnation,fingerprint=CASE WHEN incarnation!=excluded.incarnation THEN NULL ELSE fingerprint END,error_code=CASE WHEN incarnation!=excluded.incarnation THEN 'owner_changed' ELSE error_code END,next_attempt_ms=0",params![id,inc])?;
    if old_record
        .as_ref()
        .is_none_or(|previous| previous != &bytes)
    {
        tx.execute(
            "INSERT INTO catalogue_events(session_id,kind,recorded_at_ms) VALUES(?1,?2,?3)",
            params![
                id,
                if current.is_none() {
                    "created"
                } else if current.as_deref() == Some(&inc) {
                    "registration"
                } else {
                    "owner_changed"
                },
                now()
            ],
        )?;
    }
    Ok(())
}
fn record_tx(
    tx: &Transaction<'_>,
    namespace: &str,
    id: Uuid,
    bytes: &[u8],
    reserve: bool,
) -> Result<bool> {
    ensure!(
        bytes.len() <= 16384,
        "lifecycle command exceeds receipt limit"
    );
    let migrated:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='legacy_migration_commands')",[],|row|row.get(0))?;
    if migrated {
        ensure!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM legacy_migration_commands WHERE command_id=?1)",[id.to_string()],|row|row.get::<_,bool>(0))?,"legacy command retired by scope migration; inspect its original outcome without replay");
    }
    let saved: Option<Vec<u8>> = tx
        .query_row(
            "SELECT request FROM lifecycle_commands WHERE namespace=?1 AND command_id=?2",
            params![namespace, id.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(saved) = saved {
        ensure!(saved == bytes, "lifecycle command ID payload conflict");
        return Ok(true);
    }
    if reserve {
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM lifecycle_commands WHERE namespace=?1",
            [namespace],
            |r| r.get(0),
        )?;
        ensure!(count < 65536, "lifecycle receipt capacity exhausted");
        tx.execute(
            "INSERT INTO lifecycle_commands VALUES(?1,?2,?3)",
            params![namespace, id.to_string(), bytes],
        )?;
    }
    Ok(false)
}
async fn blocking<T: Send + 'static>(
    root: &Path,
    f: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
) -> Result<T> {
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || f(&mut open(&root)?)).await?
}
pub async fn save(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    let r = registration.clone();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        save_tx(&tx, &r, None)?;
        tx.commit()?;
        Ok(())
    })
    .await
}

#[allow(dead_code)] // Wired to the system installer only when privileged launch is enabled.
pub async fn store_identity(root: &Path, identity: &ConfiguredExecutionIdentity) -> Result<()> {
    ensure!(
        !identity.identity.id.is_nil()
            && !identity.account_context.id.is_nil()
            && !identity.label.is_empty()
            && identity.label.len() <= 128
            && !identity.label.chars().any(char::is_control)
            && !identity.user_name.is_empty()
            && identity.user_name.len() <= 64
            && identity
                .user_name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            && identity.home.is_absolute()
            && !identity
                .home
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
            && identity.supplementary_groups.len() <= 64
            && matches!(
                (identity.authority, identity.uid),
                (AuthorityClass::Administrator, 0) | (AuthorityClass::Ordinary, 1..=u32::MAX)
            ),
        "invalid configured execution identity"
    );
    let identity = identity.clone();
    blocking(root, move |db| {
        let id = identity.identity.id.to_string();
        let revision = identity.identity.revision.get();
        let bytes = serde_json::to_string(&identity)?;
        ensure!(bytes.len() <= 16384, "execution identity exceeds limit");
        db.execute(
            "INSERT OR IGNORE INTO execution_identities VALUES(?1,?2,?3)",
            params![id, revision, bytes],
        )?;
        let saved: String = db.query_row(
            "SELECT record FROM execution_identities WHERE identity_id=?1 AND revision=?2",
            params![id, revision],
            |row| row.get(0),
        )?;
        ensure!(saved == bytes, "execution identity revision conflict");
        Ok(())
    })
    .await
}

/// Resolve the exact current host-controlled identity before reserving a new
/// bound voyage. A reused or revised account reference cannot acquire a launch.
#[cfg(target_os = "linux")]
pub async fn configured_identity(
    root: &Path,
    reference: &voyage_protocol::execution_identity::IdentityRef,
) -> Result<ConfiguredExecutionIdentity> {
    let reference = reference.clone();
    blocking(root, move |db| {
        let latest: u64 = db.query_row(
            "SELECT max(revision) FROM execution_identities WHERE identity_id=?1",
            [reference.id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            latest == reference.revision.get(),
            "execution identity revision changed"
        );
        let saved: String = db.query_row(
            "SELECT record FROM execution_identities WHERE identity_id=?1 AND revision=?2",
            params![reference.id.to_string(), reference.revision.get()],
            |row| row.get(0),
        )?;
        let identity: ConfiguredExecutionIdentity = serde_json::from_str(&saved)?;
        ensure!(
            identity.enabled && identity.identity == reference,
            "execution identity is unavailable"
        );
        Ok(identity)
    })
    .await
}

#[allow(dead_code)] // Used by the privileged launch/recovery path after migration.
pub async fn execution_binding(root: &Path, session: Uuid) -> Result<Option<ExecutionBinding>> {
    blocking(root, move |db| {
        let saved: Option<String> = db
            .query_row(
                "SELECT record FROM execution_bindings WHERE session_id=?1",
                [session.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        saved
            .map(|value| Ok(serde_json::from_str(&value)?))
            .transpose()
    })
    .await
}

/// Resolve an observer's execution identity only from the protected catalogue.
/// The runtime-visible registration is a projection, never an authority source.
#[cfg(target_os = "linux")]
pub async fn bound_observer_identity(
    root: &Path,
    registration: &ProcessRegistration,
) -> Result<ConfiguredExecutionIdentity> {
    let registration = registration.clone();
    blocking(root, move |db| {
        let saved: String = db.query_row(
            "SELECT record FROM execution_bindings WHERE session_id=?1",
            [registration.session_id.to_string()],
            |row| row.get(0),
        )?;
        let binding: ExecutionBinding = serde_json::from_str(&saved)?;
        ensure!(
            binding.session_id == registration.session_id
                && binding.incarnation == registration.incarnation
                && registration.peer_uids.as_ref() == Some(&binding.peer_uids)
                && binding.peer_uids.supervisor == unsafe { libc::geteuid() },
            "observer execution binding changed"
        );
        let latest: u64 = db.query_row(
            "SELECT max(revision) FROM execution_identities WHERE identity_id=?1",
            [binding.identity.id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            latest == binding.identity.revision.get(),
            "observer execution identity revision changed"
        );
        let saved: String = db.query_row(
            "SELECT record FROM execution_identities WHERE identity_id=?1 AND revision=?2",
            params![binding.identity.id.to_string(), binding.identity.revision.get()],
            |row| row.get(0),
        )?;
        let identity: ConfiguredExecutionIdentity = serde_json::from_str(&saved)?;
        ensure!(
            identity.enabled
                && identity.identity == binding.identity
                && identity.account_context == binding.account_context
                && identity.uid == binding.peer_uids.runtime
                && binding.administrator_grant_id.is_some()
                    == (identity.authority == AuthorityClass::Administrator),
            "observer execution identity is unavailable"
        );
        if let Some(grant_id) = binding.administrator_grant_id {
            let (saved, revoked): (String, Option<u64>) = db.query_row(
                "SELECT g.record,r.revoked_at_ms FROM administrator_grants g LEFT JOIN administrator_revocations r USING(grant_id) WHERE g.grant_id=?1",
                [grant_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let mut grant: AdministratorGrant = serde_json::from_str(&saved)?;
            grant.revoked_at_ms = revoked;
            ensure!(
                grant.check_current(&grant).is_ok()
                    && grant.grant_id == grant_id
                    && grant.session_id == binding.session_id
                    && grant.identity == binding.identity
                    && grant.account_context == binding.account_context
                    && grant.host_identity_digest == binding.host_identity_digest
                    && grant.policy_digest == binding.policy_digest,
                "observer administrator authority is unavailable"
            );
        }
        Ok(identity)
    })
    .await
}

#[allow(dead_code)] // Reached through the owner-only administrator review path.
pub async fn issue_administrator_grant(root: &Path, grant: &AdministratorGrant) -> Result<()> {
    ensure!(
        grant.revoked_at_ms.is_none() && grant.check_current(grant).is_ok(),
        "invalid administrator grant"
    );
    let grant = grant.clone();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let configured: String = tx.query_row(
            "SELECT record FROM execution_identities WHERE identity_id=?1 AND revision=?2",
            params![grant.identity.id.to_string(), grant.identity.revision.get()],
            |row| row.get(0),
        )?;
        let configured: ConfiguredExecutionIdentity = serde_json::from_str(&configured)?;
        ensure!(
            configured.enabled
                && configured.authority == AuthorityClass::Administrator
                && configured.account_context == grant.account_context,
            "administrator grant identity is unavailable"
        );
        let active: i64 = tx.query_row(
            "SELECT count(*) FROM administrator_grants g LEFT JOIN administrator_revocations r USING(grant_id) WHERE g.session_id=?1 AND r.grant_id IS NULL",
            [grant.session_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(active == 0, "voyage already has an active administrator grant");
        let record = serde_json::to_string(&grant)?;
        ensure!(record.len() <= 16384, "administrator grant exceeds limit");
        tx.execute(
            "INSERT INTO administrator_grants VALUES(?1,?2,?3)",
            params![grant.grant_id.to_string(), grant.session_id.to_string(), record],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

#[allow(dead_code)] // Reached through the owner-only revocation path.
pub async fn revoke_administrator_grant(
    root: &Path,
    grant_id: Uuid,
    command_id: Uuid,
    revoked_at_ms: u64,
) -> Result<()> {
    ensure!(
        !grant_id.is_nil() && !command_id.is_nil() && revoked_at_ms > 0,
        "invalid administrator revocation"
    );
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let created: String = tx.query_row(
            "SELECT record FROM administrator_grants WHERE grant_id=?1",
            [grant_id.to_string()],
            |row| row.get(0),
        )?;
        let created: AdministratorGrant = serde_json::from_str(&created)?;
        ensure!(
            revoked_at_ms >= created.created_at_ms,
            "revocation precedes grant"
        );
        tx.execute(
            "INSERT OR IGNORE INTO administrator_revocations VALUES(?1,?2,?3)",
            params![grant_id.to_string(), command_id.to_string(), revoked_at_ms],
        )?;
        let saved: (String, u64) = tx.query_row(
            "SELECT command_id,revoked_at_ms FROM administrator_revocations WHERE grant_id=?1",
            [grant_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        ensure!(
            saved == (command_id.to_string(), revoked_at_ms),
            "administrator revocation identity conflict"
        );
        tx.commit()?;
        Ok(())
    })
    .await
}

#[allow(dead_code)] // Used at privileged launch and owner status after activation.
pub async fn administrator_grant(root: &Path, grant_id: Uuid) -> Result<AdministratorGrant> {
    blocking(root, move |db| {
        let (saved, revoked): (String, Option<u64>) = db.query_row(
            "SELECT g.record,r.revoked_at_ms FROM administrator_grants g LEFT JOIN administrator_revocations r USING(grant_id) WHERE g.grant_id=?1",
            [grant_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let mut grant: AdministratorGrant = serde_json::from_str(&saved)?;
        ensure!(grant.grant_id == grant_id, "administrator grant identity mismatch");
        grant.revoked_at_ms = revoked;
        Ok(grant)
    })
    .await
}
pub async fn command(
    root: &Path,
    namespace: &str,
    id: Uuid,
    bytes: Vec<u8>,
    reserve: bool,
) -> Result<bool> {
    let namespace = namespace.to_owned();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = record_tx(&tx, &namespace, id, &bytes, reserve)?;
        tx.commit()?;
        Ok(result)
    })
    .await
}
fn validate_binding_tx(tx: &Transaction<'_>, binding: &ExecutionBinding) -> Result<()> {
    let latest: u64 = tx.query_row(
        "SELECT max(revision) FROM execution_identities WHERE identity_id=?1",
        [binding.identity.id.to_string()],
        |row| row.get(0),
    )?;
    ensure!(
        latest == binding.identity.revision.get(),
        "execution identity revision changed"
    );
    let identity: String = tx.query_row(
        "SELECT record FROM execution_identities WHERE identity_id=?1 AND revision=?2",
        params![
            binding.identity.id.to_string(),
            binding.identity.revision.get()
        ],
        |row| row.get(0),
    )?;
    let identity: ConfiguredExecutionIdentity = serde_json::from_str(&identity)?;
    ensure!(
        identity.enabled
            && identity.identity == binding.identity
            && identity.account_context == binding.account_context
            && identity.uid == binding.peer_uids.runtime
            && binding.peer_uids.supervisor == unsafe { libc::geteuid() }
            && binding.administrator_grant_id.is_some()
                == (identity.authority == AuthorityClass::Administrator)
            && [&binding.host_identity_digest, &binding.policy_digest]
                .into_iter()
                .all(|value| {
                    value.len() == 64
                        && value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                }),
        "execution binding fails protected identity or authority checks"
    );
    if let Some(grant_id) = binding.administrator_grant_id {
        let (saved, revoked): (String, Option<u64>) = tx.query_row(
                    "SELECT g.record,r.revoked_at_ms FROM administrator_grants g LEFT JOIN administrator_revocations r USING(grant_id) WHERE g.grant_id=?1",
                    [grant_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
        let mut grant: AdministratorGrant = serde_json::from_str(&saved)?;
        grant.revoked_at_ms = revoked;
        ensure!(
            grant.check_current(&grant).is_ok()
                && grant.grant_id == grant_id
                && grant.session_id == binding.session_id
                && grant.identity == binding.identity
                && grant.account_context == binding.account_context
                && grant.host_identity_digest == binding.host_identity_digest
                && grant.policy_digest == binding.policy_digest,
            "administrator grant does not authorize this execution binding"
        );
    }
    Ok(())
}

/// The exact receipt, new incarnation and binding become visible together.
/// Callers must prove previous local cleanup before requesting this transition.
#[cfg(target_os = "linux")]
pub async fn restart_bound(
    root: &Path,
    previous: &ProcessRegistration,
    next: &ProcessRegistration,
    bytes: Vec<u8>,
) -> Result<()> {
    let previous = previous.clone();
    let next = next.clone();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current: String = tx.query_row(
            "SELECT registration FROM voyages WHERE session_id=?1",
            [previous.session_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(
            current == serde_json::to_string(&previous)?,
            "restart admission changed"
        );
        ensure!(
            next.session_id == previous.session_id
                && next.incarnation != previous.incarnation
                && next.restart_from == Some(previous.incarnation)
                && next.peer_uids == previous.peer_uids
                && next.state == voyage_protocol::process::ProcessState::Starting,
            "invalid bound restart transition"
        );
        ensure!(
            !record_tx(&tx, "commands", next.command_id, &bytes, true)?,
            "restart already reserved"
        );
        let record: String = tx.query_row(
            "SELECT record FROM execution_bindings WHERE session_id=?1",
            [previous.session_id.to_string()],
            |row| row.get(0),
        )?;
        let mut binding: ExecutionBinding = serde_json::from_str(&record)?;
        ensure!(
            binding.incarnation == previous.incarnation
                && previous.peer_uids.as_ref() == Some(&binding.peer_uids),
            "restart binding changed"
        );
        validate_binding_tx(&tx, &binding)?;
        binding.incarnation = next.incarnation;
        // This removal is transaction-local. Foreign keys require the new
        // incarnation to exist before its binding can be inserted.
        tx.execute(
            "DELETE FROM execution_bindings WHERE session_id=?1",
            [previous.session_id.to_string()],
        )?;
        save_tx(&tx, &next, Some(&binding))?;
        tx.execute(
            "INSERT INTO execution_bindings VALUES(?1,?2,?3,?4,?5)",
            params![
                binding.session_id.to_string(),
                binding.incarnation.to_string(),
                binding.identity.id.to_string(),
                binding.identity.revision.get(),
                serde_json::to_string(&binding)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}

pub async fn admit(root: &Path, registration: &ProcessRegistration, bytes: Vec<u8>) -> Result<()> {
    admit_with_binding(root, registration, bytes, None).await
}

pub async fn admit_with_binding(
    root: &Path,
    registration: &ProcessRegistration,
    bytes: Vec<u8>,
    binding: Option<&ExecutionBinding>,
) -> Result<()> {
    ensure!(
        binding.is_none_or(|binding| {
            binding.session_id == registration.session_id
                && binding.incarnation == registration.incarnation
                && registration.peer_uids.as_ref() == Some(&binding.peer_uids)
        }),
        "execution binding does not match admission"
    );
    ensure!(
        binding.is_some() || registration.peer_uids.is_none(),
        "cross-identity admission requires a protected binding"
    );
    let r = registration.clone();
    let binding = binding.cloned();
    blocking(root, move |db| {
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        ensure!(
            !record_tx(&tx, "start-resolution/intent", r.command_id, &bytes, false)?,
            "start command fenced as not admitted"
        );
        ensure!(
            !record_tx(&tx, "commands", r.command_id, &bytes, true)?,
            "creation already reserved"
        );
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM voyages WHERE session_id=?1)",
            [r.session_id.to_string()],
            |row| row.get(0),
        )?;
        ensure!(!exists, "voyage already reserved");
        save_tx(&tx, &r, binding.as_ref())?;
        if let Some(binding) = binding {
            validate_binding_tx(&tx, &binding)?;
            let record = serde_json::to_string(&binding)?;
            ensure!(record.len() <= 16384, "execution binding exceeds limit");
            tx.execute(
                "INSERT INTO execution_bindings VALUES(?1,?2,?3,?4,?5)",
                params![
                    binding.session_id.to_string(),
                    binding.incarnation.to_string(),
                    binding.identity.id.to_string(),
                    binding.identity.revision.get(),
                    record
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    })
    .await
}
pub async fn creation_receipt(root: &Path, id: Uuid) -> Result<Option<ProcessInfo>> {
    blocking(root, move |db| {
        let s: Option<String> = db
            .query_row(
                "SELECT result FROM creation_receipts WHERE command_id=?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        s.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    })
    .await
}
pub async fn settle_creation(root: &Path, id: Uuid, info: &ProcessInfo) -> Result<()> {
    let info = info.clone();
    blocking(root, move |db| {
        db.execute(
            "INSERT OR IGNORE INTO creation_receipts VALUES(?1,?2,?3)",
            params![
                id.to_string(),
                info.session_id.to_string(),
                serde_json::to_string(&info)?
            ],
        )?;
        Ok(())
    })
    .await
}

/// One supervisor lock surrounds import and registration publication. Legacy
/// input is preserved; once imported, SQLite alone supplies registration state.
pub async fn initialize(root: &Path) -> Result<HashMap<Uuid, ProcessRegistration>> {
    let path = root.to_owned();
    blocking(root,move|db|{
  let tx=db.transaction_with_behavior(TransactionBehavior::Immediate)?;
  let imported:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM legacy_imports WHERE source='complete-v1')",[],|r|r.get(0))?;
  if !imported {
   for entry in fs::read_dir(path.join("sessions"))? {
    let dir=entry?.path();let source=dir.join("registration.json");
    if !source.try_exists()? {continue}
    registry::private_directory(&dir)?;
    let r=registry::load(&source)?;
    ensure!(dir==registry::directory(&path,r.session_id) && r.protocol==voyage_protocol::process::PROCESS_PROTOCOL,"invalid legacy registration identity");
    save_tx(&tx,&r,None)?;
    let digest=Sha256::digest(fs::read(&source)?);
    tx.execute("INSERT INTO legacy_imports VALUES(?1,?2)",params![format!("sessions/{}/registration.json",r.session_id),digest.as_slice()])?;
   }
   for namespace in ["commands","start-resolution/intent","start-resolution/not-admitted"] {
    let dir=if namespace=="commands" {path.join(namespace)}else{path.join(namespace).join("commands")};
    if !dir.try_exists()? {continue}
    registry::private_directory(&dir)?;
    for entry in fs::read_dir(dir)? {
     let source=entry?.path();if source.extension().and_then(|v|v.to_str())!=Some("json"){continue}
     let id=Uuid::parse_str(source.file_stem().and_then(|v|v.to_str()).context("invalid receipt name")?)?;
     let file=OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&source)?;
     let m=file.metadata()?;ensure!(m.is_file() && m.nlink()==1 && m.uid()==unsafe{libc::geteuid()} && m.mode()&0o077==0 && m.len()<=16384,"unsafe legacy receipt");
     use std::io::Read;let mut bytes=Vec::new();file.take(16385).read_to_end(&mut bytes)?;
     record_tx(&tx,namespace,id,&bytes,true)?;
    }
   }
   tx.execute("INSERT INTO legacy_imports VALUES('complete-v1',?1)",[&[0u8;32][..]])?;
  }
  tx.execute("UPDATE catalogue SET process_info=NULL,fingerprint=NULL,error_code='refresh_pending',next_attempt_ms=0",[])?;
  let rows=tx.prepare("SELECT registration FROM voyages ORDER BY session_id")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
  ensure!(rows.len()<=4096,"supervisor registration retention limit reached");
  let registrations=rows.into_iter().map(|s|{let r:ProcessRegistration=serde_json::from_str(&s)?;Ok((r.session_id,r))}).collect::<Result<HashMap<_,_>>>()?;
  tx.commit()?;
  Ok(registrations)
 }).await
}

pub async fn catalogue(root: &Path) -> Result<Vec<ProcessInfo>> {
    blocking(root, |db| catalogue_in(db, None)).await
}

fn catalogue_in(db: &Connection, session: Option<Uuid>) -> Result<Vec<ProcessInfo>> {
    let mut stmt=db.prepare("SELECT v.registration,c.summary,c.observed_at_ms,c.error_code,c.process_info FROM voyages v LEFT JOIN catalogue c USING(session_id) WHERE (?1 IS NULL OR v.session_id=?1) ORDER BY v.session_id")?;
    let rows = stmt.query_map([session.map(|id| id.to_string())], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    let mut infos = Vec::new();
    for row in rows {
        let (reg, summary, observed, error, info) = row?;
        let r: ProcessRegistration = serde_json::from_str(&reg)?;
        if matches!(
            r.initialize,
            Some(voyage_protocol::process::RuntimeInitialization::Participant { .. })
        ) {
            continue;
        }
        let mut i = info
            .and_then(|s| serde_json::from_str::<ProcessInfo>(&s).ok())
            .filter(|i| i.incarnation == r.incarnation)
            .unwrap_or_else(|| ProcessInfo::from(&r));
        if i.state == voyage_protocol::process::ProcessState::Live
            && observed.is_none_or(|at| now().saturating_sub(at) > 5000)
        {
            i.state = voyage_protocol::process::ProcessState::Unavailable;
        }
        let summary: Option<CatalogueSummary> =
            summary.map(|s| serde_json::from_str(&s)).transpose()?;
        if let Some(s) = &summary {
            i.name = s.name.clone().or(i.name);
        }
        i.catalogue = Some(Box::new(CatalogueMetadata {
            summary,
            observed_at_ms: observed.and_then(|v| v.try_into().ok()),
            stale: error.is_some() || observed.is_none(),
            error_code: error,
        }));
        infos.push(i);
    }
    Ok(infos)
}

/// Cursor and projections share one read transaction. The event journal is a
/// bounded invalidation log: repeated IDs coalesce to their current projection.
pub async fn catalogue_changes(
    root: &Path,
    after: Option<u64>,
    limit: u16,
) -> Result<voyage_protocol::vessel::CatalogueChanges> {
    ensure!(
        (1..=128).contains(&limit),
        "catalogue page limit must be 1..128"
    );
    ensure!(
        after.is_none_or(|value| value <= i64::MAX as u64),
        "invalid catalogue cursor"
    );
    blocking(root, move |db| {
        let tx = db.transaction()?;
        let (earliest, latest): (Option<u64>, u64) = tx.query_row(
            "SELECT min(sequence),coalesce(max(sequence),0) FROM catalogue_events", [],
            |row| Ok((row.get(0)?, row.get(1)?)))?;
        let gap = after.is_none_or(|cursor| cursor > latest || earliest.is_some_and(|first| cursor.saturating_add(1) < first));
        let mut page = voyage_protocol::vessel::CatalogueChanges {
            cursor: after.unwrap_or(latest), latest_cursor: latest,
            has_more: false, replay_gap: gap, entries: Vec::new(),
        };
        if gap { page.cursor = latest; return Ok(page); }
        let changes = tx.prepare("SELECT sequence,session_id FROM catalogue_events WHERE sequence>?1 ORDER BY sequence LIMIT ?2")?
            .query_map(params![after.unwrap_or(0),limit], |row| Ok((row.get::<_,u64>(0)?,row.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut seen = std::collections::HashSet::new();
        for (cursor, session) in changes {
            page.cursor = cursor;
            let session = Uuid::parse_str(&session)?;
            if seen.insert(session) { page.entries.extend(catalogue_in(&tx, Some(session))?); }
        }
        page.has_more = page.cursor < latest;
        Ok(page)
    }).await
}

pub async fn refresh(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    refresh_mode(root, registration, false).await
}
pub async fn refresh_now(root: &Path, registration: &ProcessRegistration) -> Result<()> {
    refresh_mode(root, registration, true).await
}
async fn refresh_mode(root: &Path, registration: &ProcessRegistration, force: bool) -> Result<()> {
    let r = registration.clone();
    let path = root.to_owned();
    // Never open execution-identity-owned SQLite inside the supervisor. The
    // bounded helper runs outside the catalogue transaction and before parsing.
    let bound_summary = if r.peer_uids.is_some() {
        let session = r.session_id;
        let ready = blocking(root, move |db| {
            let next: i64 = db.query_row(
                "SELECT next_attempt_ms FROM catalogue WHERE session_id=?1",
                [session.to_string()],
                |row| row.get(0),
            )?;
            Ok(force || next <= now())
        })
        .await?;
        if !ready {
            return Ok(());
        }
        Some(super::catalogue_observer::read(root, &r).await)
    } else {
        None
    };
    blocking(root,move|db|{
  let bound = r.peer_uids.is_some();
  let dir=registry::directory(&path,r.session_id);
  let fingerprint=if bound { None } else { fingerprint(&dir) };
  let old:(Option<String>,i64)=db.query_row("SELECT fingerprint,next_attempt_ms FROM catalogue WHERE session_id=?1",[r.session_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
  if !force && (old.1>now() || (fingerprint.is_some() && fingerprint==old.0)){return Ok(())}
  let summary=bound_summary.unwrap_or_else(|| voyage_runtime::catalogue::read(&dir.join("journal"),r.session_id));
  let mut info=ProcessInfo::from(&r);
  info.state=if bound {voyage_protocol::process::ProcessState::Unavailable}
   else if super::recovery::suspended(&dir,&r){voyage_protocol::process::ProcessState::Suspended}
   else if r.state==voyage_protocol::process::ProcessState::Relinquished{r.state.clone()}
   else if super::recovery::clean_stop(&dir,&r){voyage_protocol::process::ProcessState::Stopped}
   else {voyage_protocol::process::ProcessState::Unavailable};
  if info.state==voyage_protocol::process::ProcessState::Stopped {info.archive=super::recovery::archived(&dir,&r);info.deletion=super::recovery::deletion(&dir,&r);}
  let tx=db.transaction_with_behavior(TransactionBehavior::Immediate)?;
  let current:String=tx.query_row("SELECT incarnation FROM voyages WHERE session_id=?1",[r.session_id.to_string()],|row|row.get(0))?;
  if current!=r.incarnation.to_string(){return Ok(())}
  let (old_info, old_error):(Option<String>,Option<String>)=tx.query_row("SELECT process_info,error_code FROM catalogue WHERE session_id=?1",[r.session_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?)))?;
  // Journal metadata refresh is not a failed liveness observation. Preserve a
  // current live projection while its socket exists; the separate bounded
  // inspect publishes an actual unavailable/suspended/stopped transition.
  if !bound && info.state==voyage_protocol::process::ProcessState::Unavailable && dir.join("runtime.sock").exists()
   && let Some(previous)=old_info.as_deref().and_then(|value|serde_json::from_str::<ProcessInfo>(value).ok())
   && previous.incarnation==r.incarnation && previous.state==voyage_protocol::process::ProcessState::Live {info=previous;}
  let mut changed = old_info.as_deref()!=Some(serde_json::to_string(&info)?.as_str()) || old_error.as_deref()!=if summary.is_ok(){None}else{Some("journal_unavailable")};
  match summary {
   Ok(summary)=>{
    let previous:Option<String>=tx.query_row("SELECT summary FROM catalogue WHERE session_id=?1",[r.session_id.to_string()],|row|row.get(0))?;
    let summary_json=serde_json::to_string(&summary)?;
    changed|=previous.as_ref()!=Some(&summary_json);
    if let Some(previous)=previous {
        let previous:CatalogueSummary=serde_json::from_str(&previous)?;
        if (summary.revision,summary.observation_cursor)<(previous.revision,previous.observation_cursor) {return Ok(())}
    }
    tx.execute("UPDATE catalogue SET summary=?2,observed_at_ms=?3,fingerprint=?4,process_info=?5,error_code=NULL,failures=0,next_attempt_ms=?6 WHERE session_id=?1",params![r.session_id.to_string(),summary_json,now(),fingerprint,serde_json::to_string(&info)?,if bound { now()+1000 } else { 0 }])?;},
   Err(_)=>{tx.execute("UPDATE catalogue SET process_info=?2,error_code='journal_unavailable',failures=min(failures+1,6),next_attempt_ms=?3+min(60000,1000*(1<<min(failures,6))) WHERE session_id=?1",params![r.session_id.to_string(),serde_json::to_string(&info)?,now()])?;}
  }
  if changed {
    tx.execute("INSERT INTO catalogue_events(session_id,kind,recorded_at_ms) VALUES(?1,'metadata',?2)",params![r.session_id.to_string(),now()])?;
  }
  tx.commit()?;Ok(())
 }).await
}
fn fingerprint(dir: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for name in [
        "journal/journal.sqlite3",
        "journal/journal.sqlite3-journal",
        "journal/journal.sqlite3-wal",
        "stopped.json",
        "runtime.sock",
    ] {
        match fs::symlink_metadata(dir.join(name)) {
            Ok(m) => parts.push(format!(
                "{}:{}:{}:{}:{}",
                m.ino(),
                m.len(),
                m.mtime(),
                m.mtime_nsec(),
                m.ctime_nsec()
            )),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => parts.push("absent".into()),
            Err(_) => return None,
        }
    }
    Some(parts.join("/"))
}

pub async fn observe_process(
    root: &Path,
    registration: &ProcessRegistration,
    info: &ProcessInfo,
) -> Result<()> {
    let r = registration.clone();
    let i = info.clone();
    blocking(root,move|db|{
        let tx=db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous:Option<String>=tx.query_row("SELECT process_info FROM catalogue WHERE session_id=?1 AND incarnation=?2",params![r.session_id.to_string(),r.incarnation.to_string()],|row|row.get(0)).optional()?.flatten();
        let previous_state=previous.and_then(|json|serde_json::from_str::<ProcessInfo>(&json).ok()).map(|info|info.state);
        let updated=tx.execute("UPDATE catalogue SET process_info=?3,observed_at_ms=?4 WHERE session_id=?1 AND incarnation=?2",params![r.session_id.to_string(),r.incarnation.to_string(),serde_json::to_string(&i)?,now()])?;
        if updated!=0 && previous_state.as_ref()!=Some(&i.state) {
            tx.execute("INSERT INTO catalogue_events(session_id,kind,recorded_at_ms) VALUES(?1,'process',?2)",params![r.session_id.to_string(),now()])?;
        }
        tx.commit()?;Ok(())
    }).await
}

/// Serialize lifecycle operations while loading their starting state from SQLite.
/// A cancelled caller cannot strand a committed incarnation in an old memory map.
pub struct Registrations {
    root: std::path::PathBuf,
    serial: tokio::sync::Mutex<()>,
}
pub struct RegistrationGuard<'a> {
    records: HashMap<Uuid, ProcessRegistration>,
    _serial: tokio::sync::MutexGuard<'a, ()>,
}
impl std::ops::Deref for RegistrationGuard<'_> {
    type Target = HashMap<Uuid, ProcessRegistration>;
    fn deref(&self) -> &Self::Target {
        &self.records
    }
}
impl std::ops::DerefMut for RegistrationGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.records
    }
}
impl Registrations {
    pub fn new(root: std::path::PathBuf) -> Self {
        Self {
            root,
            serial: tokio::sync::Mutex::new(()),
        }
    }
    pub async fn lock(&self) -> Result<RegistrationGuard<'_>> {
        let serial = self.serial.lock().await;
        let records = blocking(&self.root, |db| {
            let mut query = db.prepare("SELECT registration FROM voyages")?;
            let rows = query.query_map([], |r| r.get::<_, String>(0))?;
            let mut records = HashMap::new();
            for row in rows {
                let registration: ProcessRegistration = serde_json::from_str(&row?)?;
                records.insert(registration.session_id, registration);
            }
            Ok(records)
        })
        .await?;
        Ok(RegistrationGuard {
            records,
            _serial: serial,
        })
    }
}

pub async fn registration(root: &Path, session: Uuid) -> Result<ProcessRegistration> {
    blocking(root, move |db| {
        let encoded: String = db
            .query_row(
                "SELECT registration FROM voyages WHERE session_id=?1",
                [session.to_string()],
                |r| r.get(0),
            )
            .context("unknown session")?;
        Ok(serde_json::from_str(&encoded)?)
    })
    .await
}

#[cfg(test)]
mod tests;
