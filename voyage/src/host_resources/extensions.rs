//! Executable package intent joins the existing incarnation-bound cleanup ledger.
//! Callers serialize this admission with package-grant revocation. A disappeared
//! process or released lease is never interpreted here as observed cleanup.
use super::*;

pub(crate) struct ExtensionReservation(Reservation);
impl ExtensionReservation {
    pub(crate) fn acquire(
        binding: &str,
        digest: &str,
        invocation: Uuid,
        run: Uuid,
        action: &str,
    ) -> Result<Self> {
        let (session, incarnation) = PROCESS_SCOPE.get().copied().ok_or_else(|| {
            anyhow::anyhow!("executable extensions require a supervised Voyage owner")
        })?;
        ensure!(
            valid_hash(binding) && valid_hash(digest),
            "invalid executable package identity"
        );
        ensure!(
            !action.is_empty()
                && action.len() <= 160
                && action
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c)),
            "invalid executable action identity"
        );
        let (mut connection, path) = database()?;
        schema(&connection)?;
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: i64 = tx.query_row(
            "SELECT count(*) FROM extension_reservations e JOIN reservations r ON r.id=e.id WHERE e.binding=?1 AND r.observed<>1",
            [binding], |row| row.get(0))?;
        ensure!(
            active < 16,
            "executable package active-work capacity reached"
        );
        let id = Uuid::new_v4();
        let lease = lease(&path, id)?;
        lease.try_lock()?;
        tx.execute(
            "INSERT INTO reservations(id,kind,owner,units) VALUES(?1,'extensions',?2,1)",
            params![id.to_string(), run.to_string()],
        )?;
        tx.execute(
            "INSERT INTO reservation_scopes VALUES(?1,?2,?3)",
            params![id.to_string(), session.to_string(), incarnation.to_string()],
        )?;
        tx.execute("INSERT INTO extension_reservations(id,binding,digest,invocation,action) VALUES(?1,?2,?3,?4,?5)", params![id.to_string(),binding,digest,invocation.to_string(),action])?;
        tx.commit()?;
        Ok(Self(Reservation {
            id,
            path,
            _lease: lease,
        }))
    }
    /// Only the SDK child/namespace observer may call this after cleanup. Neither
    /// a terminal protocol result nor cancellation acknowledgment is sufficient.
    pub(crate) fn release_observed(&self) -> Result<()> {
        self.0.release_observed()
    }
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn schema(connection: &Connection) -> Result<()> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS extension_reservations(id TEXT PRIMARY KEY,binding TEXT NOT NULL,digest TEXT NOT NULL,invocation TEXT NOT NULL,action TEXT NOT NULL); CREATE INDEX IF NOT EXISTS extension_binding ON extension_reservations(binding);")?;
    Ok(())
}
/// Use under the package-grant lock so no admission can race the zero count.
/// An unavailable ledger refuses replacement rather than guessing no work exists.
pub(crate) fn pending_at(binding: &str, user: &std::path::Path) -> Result<u64> {
    ensure!(valid_hash(binding), "invalid executable package binding");
    let (connection, _) = database_at(user.to_path_buf())?;
    schema(&connection)?;
    Ok(connection.query_row(
        "SELECT count(*) FROM extension_reservations e JOIN reservations r ON r.id=e.id WHERE e.binding=?1 AND r.observed<>1",
        [binding], |row| row.get(0))?)
}
