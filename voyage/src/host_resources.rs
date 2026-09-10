//! Account-wide reservations shared by every independent runtime on this host.
//! Dropping a reservation never establishes cleanup; unresolved rows retain quota.
use anyhow::{Result, ensure};
use rusqlite::{Connection, TransactionBehavior, params};
use serde_json::{Value, json};
use std::path::PathBuf;
use uuid::Uuid;
pub mod cli;

#[derive(Debug, thiserror::Error)]
#[error(
    "host {kind} quota exhausted ({used} charged, {requested} requested, limit {maximum}); unconfirmed cleanup reservations remain charged"
)]
struct QuotaExhausted {
    kind: String,
    used: i64,
    requested: usize,
    maximum: usize,
}

/// Authored labels only: storage diagnostics can contain private host paths.
pub(crate) fn startup_failure(error: &anyhow::Error) -> &'static str {
    if error.downcast_ref::<QuotaExhausted>().is_some() {
        "Host execution capacity exhausted. Wait for active voyages to finish or reconcile stopped owners' cleanup reservations."
    } else if error
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
        "Host execution capacity could not be reserved. Check host resource accounting on the executing machine."
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
    let parent = crate::config::default_data_dir();
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
    Ok((connection, path))
}
impl Reservation {
    pub fn acquire(kind: &str, owner: Uuid, units: usize) -> Result<Self> {
        let maximum = match kind {
            "executors" => 64,
            "terminals" => 256,
            _ => anyhow::bail!("unknown host resource kind"),
        };
        ensure!(
            units > 0 && units <= maximum,
            "host resource request exceeds its limit"
        );
        let (mut connection, path) = database()?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let used: i64 = tx.query_row(
            "SELECT coalesce(sum(units),0) FROM reservations WHERE kind=?1 AND observed=0",
            [kind],
            |row| row.get(0),
        )?;
        if used + units as i64 > maximum as i64 {
            return Err(QuotaExhausted {
                kind: kind.to_owned(),
                used,
                requested: units,
                maximum,
            }
            .into());
        }
        let count: i64 = tx.query_row("SELECT count(*) FROM reservations", [], |row| row.get(0))?;
        ensure!(count < 100000, "host resource evidence capacity reached");
        let id = Uuid::new_v4();
        let lease = lease(&path, id)?;
        lease.try_lock()?;
        tx.execute(
            "INSERT INTO reservations(id,kind,owner,units) VALUES(?1,?2,?3,?4)",
            params![id.to_string(), kind, owner.to_string(), units as i64],
        )?;
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
        json!({"reservation_id":id,"cleanup":"operator_attested_not_observed","quota_released":true}),
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
        json!({"scope":"executing_os_account_on_this_host","database":path,"limits":{"executors":64,"terminals":256},"reservations":rows,"total":total,"truncated":total>256,"release":"observed_cleanup_only"}),
    )
}
