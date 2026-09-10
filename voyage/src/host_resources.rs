//! Account-wide reservations shared by every independent runtime on this host.
//! Dropping a reservation never establishes cleanup; unresolved rows retain evidence.
//! This ledger does not impose account-wide execution or terminal quotas.
use anyhow::{Result, ensure};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::PathBuf;
use uuid::Uuid;
pub mod cli;

static PROCESS_SCOPE: std::sync::OnceLock<(Uuid, Uuid)> = std::sync::OnceLock::new();

pub(crate) fn set_process_scope(session: Uuid, incarnation: Uuid) -> Result<()> {
    ensure!(
        PROCESS_SCOPE.set((session, incarnation)).is_ok(),
        "process resource scope already set"
    );
    Ok(())
}

/// Only the fenced recovery caller with guardian cleanup evidence may release
/// these obligations. Missing legacy scope rows remain unresolved.
pub(crate) fn recover_process_scope(session: Uuid, incarnation: Uuid) -> Result<()> {
    let (mut connection, path) = database()?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let ids = {
        let mut query = tx.prepare("SELECT r.id FROM reservations r JOIN reservation_scopes s ON r.id=s.id WHERE s.session=?1 AND s.incarnation=?2 AND r.observed=0")?;
        query
            .query_map(
                params![session.to_string(), incarnation.to_string()],
                |row| row.get::<_, String>(0),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut leases = Vec::new();
    for id in ids {
        let lock = lease(&path, id.parse()?)?;
        lock.try_lock()
            .map_err(|_| anyhow::anyhow!("resource still owned during recovery"))?;
        leases.push(lock);
        tx.execute(
            "UPDATE reservations SET observed=1 WHERE id=?1 AND observed=0",
            [id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Authored labels only: storage diagnostics can contain private host paths.
pub(crate) fn startup_failure(error: &anyhow::Error) -> &'static str {
    if error
        .downcast_ref::<rusqlite::Error>()
        .is_some_and(|error| {
            matches!(
                error.sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
            )
        })
    {
        "Host resource accounting is busy. Retry this turn."
    } else {
        "Host resource cleanup tracking could not be initialized. Check host resource accounting on the executing machine."
    }
}

pub struct Reservation {
    id: Uuid,
    path: PathBuf,
    _lease: std::fs::File,
}
fn database() -> Result<(Connection, PathBuf)> {
    ensure!(
        cfg!(target_os = "linux"),
        "shared host resource accounting requires Linux private storage"
    );
    database_at(crate::config::default_data_dir())
}
fn database_at(parent: PathBuf) -> Result<(Connection, PathBuf)> {
    std::fs::create_dir_all(&parent)?;
    let directory = crate::attachment::journal::prepare_directory(parent.join("host-resources"))?;
    let path = directory.join("reservations.sqlite3");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(&path)?;
    ensure!(
        file.metadata()?.is_file(),
        "host resource database is not a file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() == 1,
            "host resource database is not private"
        );
    }
    let connection = Connection::open(&path)?;
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    connection.execute_batch("PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS reservations(id TEXT PRIMARY KEY,kind TEXT NOT NULL,owner TEXT NOT NULL,units INTEGER NOT NULL,observed INTEGER NOT NULL DEFAULT 0);")?;
    connection.execute_batch("CREATE TABLE IF NOT EXISTS reservation_scopes(id TEXT PRIMARY KEY,session TEXT NOT NULL,incarnation TEXT NOT NULL);")?;
    Ok((connection, path))
}
impl Reservation {
    pub fn acquire(kind: &str, owner: Uuid, units: usize) -> Result<Self> {
        Self::acquire_in(database()?, kind, owner, units)
    }
    fn acquire_in(
        (mut connection, path): (Connection, PathBuf),
        kind: &str,
        owner: Uuid,
        units: usize,
    ) -> Result<Self> {
        ensure!(
            matches!(kind, "executors" | "terminals"),
            "unknown host resource kind"
        );
        let units = i64::try_from(units)?;
        ensure!(units > 0, "host resource request must be positive");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = Uuid::new_v4();
        let lease = lease(&path, id)?;
        lease.try_lock()?;
        tx.execute(
            "INSERT INTO reservations(id,kind,owner,units) VALUES(?1,?2,?3,?4)",
            params![id.to_string(), kind, owner.to_string(), units],
        )?;
        if let Some((session, incarnation)) = PROCESS_SCOPE.get() {
            tx.execute(
                "INSERT INTO reservation_scopes VALUES(?1,?2,?3)",
                params![id.to_string(), session.to_string(), incarnation.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(Self {
            id,
            path,
            _lease: lease,
        })
    }
    pub fn release_observed(&self) -> Result<()> {
        let (connection, path) = database()?;
        ensure!(path == self.path, "host resource store changed");
        ensure!(
            connection.execute(
                "UPDATE reservations SET observed=1 WHERE id=?1",
                [self.id.to_string()]
            )? == 1,
            "host resource reservation missing"
        );
        Ok(())
    }
}
fn lease(database: &std::path::Path, id: Uuid) -> Result<std::fs::File> {
    let path = database
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing resource directory"))?
        .join(format!("{id}.lease"));
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    ensure!(file.metadata()?.is_file(), "invalid resource lease");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata()?;
        ensure!(
            meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.nlink() == 1,
            "unsafe resource lease"
        );
    }
    Ok(file)
}
pub fn attest(id: Uuid, reason: &str) -> Result<Value> {
    ensure!(
        reason.trim().len() >= 10 && reason.len() <= 1024 && !reason.chars().any(char::is_control),
        "provide a meaningful bounded cleanup attestation reason"
    );
    let (mut connection, path) = database()?;
    let lease = lease(&path, id)?;
    lease
        .try_lock()
        .map_err(|_| anyhow::anyhow!("resource owner is still alive; cannot attest its cleanup"))?;
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS attestations(id TEXT PRIMARY KEY,reason TEXT NOT NULL)",
    )?;
    ensure!(
        tx.execute(
            "UPDATE reservations SET observed=2 WHERE id=?1 AND observed=0",
            [id.to_string()]
        )? == 1,
        "no unresolved reservation for attestation"
    );
    tx.execute(
        "INSERT INTO attestations VALUES(?1,?2)",
        params![id.to_string(), reason],
    )?;
    tx.commit()?;
    Ok(
        json!({"reservation_id":id,"cleanup":"operator_attested_not_observed","quota_released":false}),
    )
}
pub fn inspect() -> Result<Value> {
    let (connection, path) = database()?;
    let mut query = connection.prepare(
        "SELECT id,kind,owner,units FROM reservations WHERE observed=0 ORDER BY rowid LIMIT 256",
    )?;
    let rows=query.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"kind":row.get::<_,String>(1)?,"owner":row.get::<_,String>(2)?,"units":row.get::<_,i64>(3)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
    let total: i64 = connection.query_row(
        "SELECT count(*) FROM reservations WHERE observed=0",
        [],
        |row| row.get(0),
    )?;
    Ok(
        json!({"scope":"executing_os_account_on_this_host","database":path,"limits":{"executors":null,"terminals":null},"reservations":rows,"total":total,"truncated":total>256,"release":"observed_cleanup_only"}),
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn unresolved_and_historical_evidence_does_not_limit_admission() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let open = || database_at(directory.path().to_path_buf());
        let (connection, _) = open()?;
        // A full legacy ledger must not become another execution quota.
        connection.execute_batch(
            "WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<100000)
             INSERT INTO reservations SELECT 'historical-'||x,'executors','old-owner',1,1 FROM n;",
        )?;
        let first = Reservation::acquire_in(open()?, "executors", Uuid::new_v4(), 64)?;
        let first_id = first.id.to_string();
        drop(first); // Unobserved cleanup stays unresolved, but does not block others.
        let executor = Reservation::acquire_in(open()?, "executors", Uuid::new_v4(), 65)?;
        let terminal = Reservation::acquire_in(open()?, "terminals", Uuid::new_v4(), 257)?;
        let another = Reservation::acquire_in(open()?, "executors", Uuid::new_v4(), 1)?;
        let (_, path) = open()?;
        assert!(lease(&path, executor.id)?.try_lock().is_err());
        assert!(lease(&path, terminal.id)?.try_lock().is_err());
        assert!(lease(&path, another.id)?.try_lock().is_err());
        let observed: i64 = connection.query_row(
            "SELECT observed FROM reservations WHERE id=?1",
            [first_id],
            |row| row.get(0),
        )?;
        assert_eq!(observed, 0);
        let unresolved: i64 = connection.query_row(
            "SELECT count(*) FROM reservations WHERE observed=0",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(unresolved, 4);
        Ok(())
    }
}
