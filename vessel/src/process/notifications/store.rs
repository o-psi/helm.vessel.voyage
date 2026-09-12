//! Bounded notification acceptance journal, not an authority or decision engine.
//!
//! The caller MUST authenticate current recipient grant and source on every path,
//! including configuration inspection, receipts and retries. Quiet hours belong
//! to attention presentation; inbox observation remains available during them.
//!
//! Lifetime caps deliberately fail closed: no identity, fingerprint or receipt is
//! recycled. A full journal needs an explicit future generation-retirement design,
//! not deletion/recreation. Revocation and receipt updates reserve no new rows.
//! Expired/revoked payloads are nulled on time-bearing access; their fixed-size
//! fingerprints survive forever. SQLite free pages/backups and already disclosed
//! bytes cannot be recalled. Restore of old backups requires external current
//! grant/source validation; this journal cannot detect whole-directory rollback.
//!
//! This uses Vessel's existing Unix private-directory boundary, not an OS sandbox
//! or a native Windows security claim. No WAL, no unbounded in-memory queues.
//! Logical capacity reserves lifecycle/receipt headroom; actual I/O/disk-full
//! errors are still errors, never reported as durable settlement. Never remove
//! this directory automatically to recover capacity or an unavailable clock.
use anyhow::{Result, bail, ensure};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::Path,
    time::Duration,
};
use uuid::Uuid;
use voyage_protocol::notifications::{
    Destination, DestinationRecord, InboxEntry, InboxPage, MAX_DESTINATION_LIFETIME_MS,
    MAX_INBOX_PAGE, MAX_NOTIFICATION_TTL_MS, Notification, NotificationKind, NotificationReceipt,
    ProducerCursor, ProducerError, ReceiptState,
};

/// Permanent admissions, including revoked/expired records. All rows have bounded
/// typed metadata; each destination consumes three lifecycle commands plus one command per synthetic test event.
pub const MAX_DESTINATIONS: usize = 256;
pub const MAX_DESTINATIONS_PER_GRANT: usize = 32;
pub const MAX_DESTINATIONS_PER_SOURCE: usize = 32;
pub const MAX_EVENTS: usize = 16_384;
pub const MAX_EVENTS_PER_DESTINATION: usize = 1024;
const MAX_RECORD_BYTES: usize = 1024;
// Admission budgets remain well below this hard SQLite main-file ceiling (64 MiB).
// Spare pages allow receipt/revocation updates even at logical admission capacity.
const MAX_DATABASE_PAGES: u32 = 16_384;

pub struct Store {
    connection: Connection,
    current_clock: bool,
    authority: Option<(
        std::path::PathBuf,
        Destination,
        voyage_protocol::process::ProcessRight,
        voyage_protocol::process::ProcessRegistration,
    )>,
    #[cfg(test)]
    event_capacity: usize,
}

impl Store {
    /// `directory` is a dedicated notification directory under a trusted private
    /// parent, not the Vessel root. Multiple handles serialize with BEGIN IMMEDIATE.
    pub fn open(directory: impl AsRef<Path>) -> Result<Self> {
        let directory = directory.as_ref();
        crate::process::registry::private_directory(directory)?;
        for name in ["notifications.sqlite3", "notifications.sqlite3-journal"] {
            private_file(&directory.join(name))?;
        }
        // Never silently open unexpected WAL state with unchecked sidecars.
        for name in ["notifications.sqlite3-wal", "notifications.sqlite3-shm"] {
            ensure!(
                !directory.join(name).try_exists()?,
                "unexpected notification WAL state"
            );
        }
        let connection = Connection::open_with_flags(
            directory.join("notifications.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.execute_batch(&format!(
            "PRAGMA page_size=4096; PRAGMA max_page_count={MAX_DATABASE_PAGES};
             PRAGMA journal_mode=PERSIST; PRAGMA synchronous=FULL;
             PRAGMA temp_store=MEMORY; PRAGMA secure_delete=ON;
             PRAGMA journal_size_limit=1048576; PRAGMA foreign_keys=ON;"
        ))?;
        ensure!(
            connection.query_row("PRAGMA page_size", [], |r| r.get::<_, u32>(0))? == 4096,
            "unsupported notification database page size"
        );
        let version: u32 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        ensure!(version <= 1, "unsupported notification store version");
        connection.execute_batch(
            "BEGIN IMMEDIATE;
             CREATE TABLE IF NOT EXISTS clock (id INTEGER PRIMARY KEY CHECK(id=1), now INTEGER NOT NULL);
             INSERT OR IGNORE INTO clock VALUES(1,0);
             CREATE TABLE IF NOT EXISTS destinations (
               id TEXT PRIMARY KEY, grant_id TEXT NOT NULL, source_id TEXT NOT NULL,
               payload TEXT NOT NULL CHECK(length(payload)<=1024), expires INTEGER NOT NULL,
               revoked INTEGER, accepted INTEGER, cursor INTEGER NOT NULL DEFAULT 0, producer_error TEXT);
             CREATE TABLE IF NOT EXISTS commands (
               id TEXT PRIMARY KEY, destination_id TEXT NOT NULL REFERENCES destinations(id),
               operation INTEGER NOT NULL CHECK(operation BETWEEN 0 AND 3), argument TEXT);
             CREATE UNIQUE INDEX IF NOT EXISTS lifecycle_commands ON commands(destination_id,operation) WHERE operation<3;
             CREATE TABLE IF NOT EXISTS events (
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,
               destination_id TEXT NOT NULL REFERENCES destinations(id), event_id TEXT NOT NULL,
               source_event_id TEXT NOT NULL, fingerprint BLOB NOT NULL CHECK(length(fingerprint)=32),
               payload TEXT CHECK(length(payload)<=1024), expires INTEGER NOT NULL,
               state INTEGER NOT NULL DEFAULT 0 CHECK(state BETWEEN 0 AND 2),
               UNIQUE(destination_id,event_id), UNIQUE(destination_id,source_event_id));
             CREATE INDEX IF NOT EXISTS inbox ON events(destination_id,sequence);
             PRAGMA user_version=1;
             COMMIT;"
        )?;
        // Sync names as well as SQLite's contents on initial directory creation.
        fs::File::open(directory)?.sync_all()?;
        Ok(Self {
            connection,
            current_clock: false,
            authority: None,
            #[cfg(test)]
            event_capacity: MAX_EVENTS,
        })
    }

    /// Service path samples the clock after acquiring SQLite's write lock.
    /// Explicit-clock open remains available to deterministic unit fixtures.
    pub fn open_current(directory: impl AsRef<Path>) -> Result<Self> {
        let mut store = Self::open(directory)?;
        store.current_clock = true;
        Ok(store)
    }

    pub fn authorize(
        &mut self,
        root: &Path,
        destination: &Destination,
        right: voyage_protocol::process::ProcessRight,
        registration: &voyage_protocol::process::ProcessRegistration,
    ) {
        self.authority = Some((
            root.to_owned(),
            destination.clone(),
            right,
            registration.clone(),
        ));
    }

    pub fn configure(
        &self,
        command_id: Uuid,
        destination: Destination,
        now: u64,
    ) -> Result<DestinationRecord> {
        ensure!(!command_id.is_nil(), "nil notification command ID");
        let (tx, now) = self.begin(now, false)?;
        if let Some((id, operation)) = command(&tx, command_id)? {
            ensure!(
                id == destination.id && operation == 0,
                "notification command conflict"
            );
            let record =
                load_destination(&tx, id)?.ok_or_else(|| anyhow::anyhow!("missing destination"))?;
            ensure!(
                record.destination == destination,
                "notification command conflict"
            );
            tx.commit()?;
            return Ok(record);
        }
        check_clock(&tx, now)?;
        validate_destination(&destination, now)?;
        ensure!(
            load_destination(&tx, destination.id)?.is_none(),
            "destination ID already used"
        );
        ensure!(
            count(&tx, "SELECT count(*) FROM destinations", None)? < MAX_DESTINATIONS,
            "notification destination capacity exhausted"
        );
        ensure!(
            count(
                &tx,
                "SELECT count(*) FROM destinations WHERE grant_id=?1",
                Some(destination.recipient_grant_id)
            )? < MAX_DESTINATIONS_PER_GRANT,
            "notification recipient capacity exhausted"
        );
        ensure!(
            count(
                &tx,
                "SELECT count(*) FROM destinations WHERE source_id=?1",
                Some(destination.source_session_id)
            )? < MAX_DESTINATIONS_PER_SOURCE,
            "notification source capacity exhausted"
        );
        let payload = encode(&destination)?;
        tx.execute("INSERT INTO destinations(id,grant_id,source_id,payload,expires) VALUES(?1,?2,?3,?4,?5)",
            params![destination.id.to_string(), destination.recipient_grant_id.to_string(), destination.source_session_id.to_string(), payload, integer(destination.expires_at_ms)?])?;
        tx.execute(
            "INSERT INTO commands(id,destination_id,operation) VALUES(?1,?2,0)",
            params![command_id.to_string(), destination.id.to_string()],
        )?;
        tx.commit()?;
        Ok(DestinationRecord {
            destination,
            revoked_at_ms: None,
            accepted_at_ms: None,
        })
    }

    /// Parent authenticates recipient acceptance separately from owner configuration.
    pub fn accept(
        &self,
        command_id: Uuid,
        destination_id: Uuid,
        now: u64,
    ) -> Result<DestinationRecord> {
        ensure!(!command_id.is_nil(), "nil notification command ID");
        let (tx, now) = self.begin(now, false)?;
        if let Some((id, operation)) = command(&tx, command_id)? {
            ensure!(
                id == destination_id && operation == 2,
                "notification command conflict"
            );
            let record =
                load_destination(&tx, id)?.ok_or_else(|| anyhow::anyhow!("unknown destination"))?;
            tx.commit()?;
            return Ok(record);
        }
        check_clock(&tx, now)?;
        let mut record = load_destination(&tx, destination_id)?
            .ok_or_else(|| anyhow::anyhow!("unknown destination"))?;
        ensure!(
            record.revoked_at_ms.is_none() && record.destination.expires_at_ms > now,
            "inactive destination"
        );
        ensure!(
            record.accepted_at_ms.is_none(),
            "destination accepted by another command"
        );
        tx.execute(
            "INSERT INTO commands(id,destination_id,operation) VALUES(?1,?2,2)",
            params![command_id.to_string(), destination_id.to_string()],
        )?;
        tx.execute(
            "UPDATE destinations SET accepted=?2 WHERE id=?1",
            params![destination_id.to_string(), integer(now)?],
        )?;
        record.accepted_at_ms = Some(now);
        tx.commit()?;
        Ok(record)
    }

    /// Atomically freezes synthetic payload/created_at with the command intent.
    /// Exact retries return the original receipt, not a newly timestamped event.
    pub fn test(
        &self,
        command_id: Uuid,
        destination_id: Uuid,
        incarnation: Uuid,
        now: u64,
    ) -> Result<NotificationReceipt> {
        ensure!(
            !command_id.is_nil() && !incarnation.is_nil(),
            "nil test identity"
        );
        let (tx, now) = self.begin(now, false)?;
        if let Some((id, operation)) = command(&tx, command_id)? {
            ensure!(
                id == destination_id && operation == 3,
                "notification command conflict"
            );
            // Incarnation is server-derived, not part of the public test request.
            // Exact command/destination retry after source restart recovers the
            // original synthetic receipt without re-attributing or re-publishing.
            let receipt = tx.query_row("SELECT sequence,event_id,state FROM events WHERE destination_id=?1 AND event_id=?2", params![destination_id.to_string(), command_id.to_string()], receipt_row)?;
            tx.commit()?;
            return Ok(receipt);
        }
        check_clock(&tx, now)?;
        let record = active_destination(&tx, destination_id, now)?;
        let notification = Notification {
            event_id: command_id,
            source_event_id: command_id,
            session_id: record.destination.source_session_id,
            run_id: None,
            incarnation: Some(incarnation),
            kind: NotificationKind::Test,
            created_at_ms: now,
            expires_at_ms: now
                .saturating_add(record.destination.notification_ttl_ms)
                .min(record.destination.expires_at_ms),
            decision_id: None,
            budget: None,
        };
        let receipt = self.insert_event(&tx, destination_id, &notification, now)?;
        tx.execute(
            "INSERT INTO commands(id,destination_id,operation,argument) VALUES(?1,?2,3,?3)",
            params![
                command_id.to_string(),
                destination_id.to_string(),
                incarnation.to_string()
            ],
        )?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn cursor(&self, destination_id: Uuid) -> Result<ProducerCursor> {
        load_cursor(&self.connection, destination_id)
    }

    /// Call only after durable publish/filtering. A crash between publish and
    /// advance re-reads the old source page and recovers exact publish receipts.
    /// Error updates never move the cursor. Success never rewinds it, and a stale
    /// successful worker cannot clear a newer error at a higher cursor.
    pub fn advance(
        &self,
        destination_id: Uuid,
        after: u64,
        error: Option<ProducerError>,
    ) -> Result<ProducerCursor> {
        integer(after)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let old = load_cursor(&tx, destination_id)?;
        if let Some(error) = error {
            tx.execute(
                "UPDATE destinations SET producer_error=?2 WHERE id=?1",
                params![destination_id.to_string(), serde_json::to_string(&error)?],
            )?;
        } else if after >= old.after {
            tx.execute(
                "UPDATE destinations SET cursor=?2,producer_error=NULL WHERE id=?1",
                params![destination_id.to_string(), integer(after)?],
            )?;
        }
        let cursor = load_cursor(&tx, destination_id)?;
        tx.commit()?;
        Ok(cursor)
    }

    /// One immutable revoke command per destination. A different command for an
    /// already revoked destination conflicts; an exact retry always succeeds.
    /// Clock rollback does not prevent revocation (uses the durable high-water).
    pub fn revoke(&self, command_id: Uuid, id: Uuid, now: u64) -> Result<()> {
        ensure!(!command_id.is_nil(), "nil notification command ID");
        let (tx, _now) = self.begin(now, false)?;
        if let Some((old_id, operation)) = command(&tx, command_id)? {
            ensure!(
                old_id == id && operation == 1,
                "notification command conflict"
            );
            tx.commit()?;
            return Ok(());
        }
        let record =
            load_destination(&tx, id)?.ok_or_else(|| anyhow::anyhow!("unknown destination"))?;
        ensure!(
            record.revoked_at_ms.is_none(),
            "destination already revoked by another command"
        );
        tx.execute(
            "INSERT INTO commands(id,destination_id,operation) VALUES(?1,?2,1)",
            params![command_id.to_string(), id.to_string()],
        )?;
        tx.execute(
            "UPDATE destinations SET revoked=(SELECT now FROM clock WHERE id=1) WHERE id=?1",
            [id.to_string()],
        )?;
        tx.execute(
            "UPDATE events SET payload=NULL WHERE destination_id=?1",
            [id.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Bounded configuration inventory, including expiry/revocation tombstones.
    /// These methods deliberately do not assert that a destination is currently authorized.
    pub fn destinations(&self) -> Result<Vec<DestinationRecord>> {
        let mut statement = self
            .connection
            .prepare("SELECT payload,revoked,accepted FROM destinations ORDER BY id")?;
        let rows = statement.query_map([], destination_row)?;
        rows.map(|row| decode_destination(row?)).collect()
    }

    pub fn destination(&self, id: Uuid) -> Result<Option<DestinationRecord>> {
        load_destination(&self.connection, id)
    }

    pub fn publish(
        &self,
        destination_id: Uuid,
        notification: Notification,
        now: u64,
    ) -> Result<NotificationReceipt> {
        let payload = encode(&notification)?;
        let fingerprint = Sha256::digest(payload.as_bytes()).to_vec();
        let (tx, now) = self.begin(now, false)?;
        let prior = tx.query_row(
            "SELECT sequence,event_id,state,fingerprint FROM events WHERE destination_id=?1 AND event_id=?2",
            params![destination_id.to_string(), notification.event_id.to_string()],
            |row| Ok((receipt_row(row)?, row.get::<_, Vec<u8>>(3)?)),
        ).optional()?;
        if let Some((receipt, old_fingerprint)) = prior {
            ensure!(
                old_fingerprint == fingerprint,
                "notification publish conflict"
            );
            tx.commit()?;
            return Ok(receipt);
        }
        let receipt = self.insert_event(&tx, destination_id, &notification, now)?;
        tx.commit()?;
        Ok(receipt)
    }

    fn insert_event(
        &self,
        tx: &Transaction<'_>,
        destination_id: Uuid,
        notification: &Notification,
        now: u64,
    ) -> Result<NotificationReceipt> {
        check_clock(tx, now)?;
        let record = active_destination(tx, destination_id, now)?;
        validate_notification(&record.destination, notification, now)?;
        let payload = encode(notification)?;
        let fingerprint = Sha256::digest(payload.as_bytes()).to_vec();
        #[cfg(test)]
        let capacity = self.event_capacity;
        #[cfg(not(test))]
        let capacity = MAX_EVENTS;
        ensure!(
            count(tx, "SELECT count(*) FROM events", None)? < capacity,
            "notification event capacity exhausted"
        );
        ensure!(
            count(
                &tx,
                "SELECT count(*) FROM events WHERE destination_id=?1",
                Some(destination_id)
            )? < MAX_EVENTS_PER_DESTINATION,
            "notification destination event capacity exhausted"
        );
        // The source-event uniqueness fence also rejects aliasing a retained source
        // event under a new event_id after expiry, revocation or lost acknowledgement.
        tx.execute("INSERT INTO events(destination_id,event_id,source_event_id,fingerprint,payload,expires) VALUES(?1,?2,?3,?4,?5,?6)",
            params![destination_id.to_string(), notification.event_id.to_string(), notification.source_event_id.to_string(), fingerprint, payload, integer(notification.expires_at_ms)?])?;
        let sequence = tx.last_insert_rowid() as u64;
        Ok(NotificationReceipt {
            sequence,
            event_id: notification.event_id,
            state: ReceiptState::Available,
        })
    }

    pub fn attention_count(&self, destination_id: Uuid, now: u64) -> Result<u64> {
        let (tx, now) = self.begin(now, true)?;
        let count = if is_active(&tx, destination_id, now)? {
            tx.query_row("SELECT count(*) FROM events WHERE destination_id=?1 AND state=0 AND payload IS NOT NULL AND expires>?2", params![destination_id.to_string(), integer(now)?], |row| row.get::<_, u64>(0))?
        } else {
            0
        };
        tx.commit()?;
        Ok(count)
    }

    pub fn inbox(
        &self,
        destination_id: Uuid,
        after: u64,
        limit: u32,
        now: u64,
    ) -> Result<InboxPage> {
        ensure!(
            limit > 0 && limit <= MAX_INBOX_PAGE,
            "invalid notification page size"
        );
        let after_sql = integer(after)?;
        let (tx, now) = self.begin(now, true)?;
        // Absent/revoked/expired destinations have no visible notification payload.
        if !is_active(&tx, destination_id, now)? {
            tx.commit()?;
            return Ok(InboxPage {
                entries: vec![],
                next_after: after,
                has_more: false,
            });
        }
        let mut entries = {
            let mut statement = tx.prepare("SELECT sequence,event_id,state,payload FROM events WHERE destination_id=?1 AND sequence>?2 AND payload IS NOT NULL AND expires>?3 ORDER BY sequence LIMIT ?4")?;
            let rows = statement.query_map(
                params![
                    destination_id.to_string(),
                    after_sql,
                    integer(now)?,
                    i64::from(limit) + 1
                ],
                entry_row,
            )?;
            rows.map(|row| decode_entry(row?))
                .collect::<Result<Vec<_>>>()?
        };
        let has_more = entries.len() > limit as usize;
        entries.truncate(limit as usize);
        let next_after = entries.last().map_or(after, |entry| entry.receipt.sequence);
        tx.commit()?;
        Ok(InboxPage {
            entries,
            next_after,
            has_more,
        })
    }

    /// Settlement remains available at capacity and after expiry/revocation, but
    /// returns only acceptance evidence. The parent must still authenticate it.
    pub fn receipt(
        &self,
        destination_id: Uuid,
        event_id: Uuid,
        state: ReceiptState,
        now: u64,
    ) -> Result<NotificationReceipt> {
        let (tx, _now) = self.begin(now, false)?;
        let changed = tx.execute(
            "UPDATE events SET state=max(state,?3) WHERE destination_id=?1 AND event_id=?2",
            params![
                destination_id.to_string(),
                event_id.to_string(),
                rank(state)
            ],
        )?;
        ensure!(changed == 1, "unknown notification receipt");
        let receipt = tx.query_row(
            "SELECT sequence,event_id,state FROM events WHERE destination_id=?1 AND event_id=?2",
            params![destination_id.to_string(), event_id.to_string()],
            receipt_row,
        )?;
        tx.commit()?;
        Ok(receipt)
    }

    pub fn get(
        &self,
        destination_id: Uuid,
        event_id: Uuid,
        now: u64,
    ) -> Result<Option<InboxEntry>> {
        let (tx, now) = self.begin(now, true)?;
        let entry = if is_active(&tx, destination_id, now)? {
            tx.query_row("SELECT sequence,event_id,state,payload FROM events WHERE destination_id=?1 AND event_id=?2 AND payload IS NOT NULL AND expires>?3",
                params![destination_id.to_string(), event_id.to_string(), integer(now)?], entry_row)
                .optional()?.map(decode_entry).transpose()?
        } else {
            None
        };
        tx.commit()?;
        Ok(entry)
    }

    fn begin(&self, now: u64, strict_clock: bool) -> Result<(Transaction<'_>, u64)> {
        integer(now)?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)?;
        let now = if self.current_clock {
            crate::process::access::store::now()?
        } else {
            now
        };
        if let Some((root, destination, right, registration)) = &self.authority {
            super::current_authority(root, destination, *right, registration)?;
        }
        if strict_clock {
            check_clock(&tx, now)?;
        }
        tx.execute(
            "UPDATE clock SET now=max(now,?1) WHERE id=1",
            [integer(now)?],
        )?;
        tx.execute("UPDATE events SET payload=NULL WHERE payload IS NOT NULL AND (expires<=(SELECT now FROM clock WHERE id=1) OR destination_id IN (SELECT id FROM destinations WHERE revoked IS NOT NULL OR expires<=(SELECT now FROM clock WHERE id=1)))", [])?;
        Ok((tx, now))
    }
}

fn private_file(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe notification store file"
    );
    Ok(())
}

fn integer(value: u64) -> Result<i64> {
    Ok(i64::try_from(value).map_err(|_| anyhow::anyhow!("notification integer out of range"))?)
}

fn check_clock(connection: &Connection, now: u64) -> Result<()> {
    let high_water: i64 =
        connection.query_row("SELECT now FROM clock WHERE id=1", [], |row| row.get(0))?;
    ensure!(
        integer(now)? >= high_water,
        "notification clock moved backwards"
    );
    Ok(())
}

fn encode(value: &impl serde::Serialize) -> Result<String> {
    let payload = serde_json::to_string(value)?;
    ensure!(
        payload.len() <= MAX_RECORD_BYTES,
        "notification metadata too large"
    );
    Ok(payload)
}

fn validate_destination(destination: &Destination, now: u64) -> Result<()> {
    ensure!(
        !destination.id.is_nil()
            && !destination.recipient_grant_id.is_nil()
            && !destination.recipient_principal_id.is_nil()
            && destination.recipient_grant_revision > 0
            && !destination.source_vessel_id.is_nil()
            && !destination.source_session_id.is_nil(),
        "nil notification destination identity"
    );
    integer(destination.expires_at_ms)?;
    ensure!(
        destination.expires_at_ms > now
            && destination.expires_at_ms - now <= MAX_DESTINATION_LIFETIME_MS,
        "invalid destination expiry"
    );
    ensure!(
        destination.notification_ttl_ms > 0
            && destination.notification_ttl_ms <= MAX_NOTIFICATION_TTL_MS,
        "invalid notification TTL"
    );
    ensure!(
        !destination.event_kinds.is_empty() && destination.event_kinds.len() <= 8,
        "invalid notification kinds"
    );
    for (index, kind) in destination.event_kinds.iter().enumerate() {
        ensure!(
            !destination.event_kinds[..index].contains(kind),
            "duplicate notification kind"
        );
    }
    ensure!(
        destination
            .quiet_hours_utc
            .is_none_or(|hours| hours.valid()),
        "invalid UTC quiet hours"
    );
    Ok(())
}

fn validate_notification(
    destination: &Destination,
    notification: &Notification,
    now: u64,
) -> Result<()> {
    ensure!(
        !notification.event_id.is_nil()
            && !notification.source_event_id.is_nil()
            && notification.incarnation.is_none_or(|id| !id.is_nil())
            && notification.run_id.is_none_or(|id| !id.is_nil())
            && notification.decision_id.is_none_or(|id| !id.is_nil()),
        "nil notification identity"
    );
    ensure!(
        notification.session_id == destination.source_session_id,
        "notification source mismatch"
    );
    ensure!(
        (notification.kind == NotificationKind::Test
            || destination.event_kinds.contains(&notification.kind)),
        "notification kind not configured"
    );
    ensure!(
        notification.kind != NotificationKind::Attention
            || (notification.incarnation.is_some() && notification.decision_id.is_some()),
        "attention requires exact decision provenance"
    );
    ensure!(
        notification.decision_id.is_none() || notification.kind == NotificationKind::Attention,
        "decision reference on non-attention notification"
    );
    ensure!(
        notification.budget.is_none() || notification.kind == NotificationKind::Budget,
        "budget detail on non-budget notification"
    );
    if let Some(budget) = &notification.budget {
        ensure!(
            !budget.scope_id.is_nil() && budget.revision > 0 && budget.ledger_sequence > 0,
            "invalid budget provenance"
        );
    }
    ensure!(
        notification.kind != NotificationKind::Test
            || (notification.run_id.is_none()
                && notification.decision_id.is_none()
                && notification.budget.is_none()),
        "synthetic test cannot claim actual run or decision"
    );
    integer(notification.expires_at_ms)?;
    ensure!(
        notification.created_at_ms <= now
            && notification.expires_at_ms > now
            && notification.expires_at_ms <= destination.expires_at_ms
            && notification.expires_at_ms - notification.created_at_ms
                <= destination.notification_ttl_ms,
        "invalid notification lifetime"
    );
    Ok(())
}

fn command(connection: &Connection, command_id: Uuid) -> Result<Option<(Uuid, i64)>> {
    let value: Option<(String, i64)> = connection
        .query_row(
            "SELECT destination_id,operation FROM commands WHERE id=?1",
            [command_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    value
        .map(|(id, operation)| Ok((Uuid::parse_str(&id)?, operation)))
        .transpose()
}

fn count(connection: &Connection, sql: &str, id: Option<Uuid>) -> Result<usize> {
    let count: i64 = match id {
        Some(id) => connection.query_row(sql, [id.to_string()], |row| row.get(0))?,
        None => connection.query_row(sql, [], |row| row.get(0))?,
    };
    Ok(usize::try_from(count)?)
}

fn destination_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, Option<u64>, Option<u64>)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

fn decode_destination(
    (payload, revoked_at_ms, accepted_at_ms): (String, Option<u64>, Option<u64>),
) -> Result<DestinationRecord> {
    Ok(DestinationRecord {
        destination: serde_json::from_str(&payload)?,
        revoked_at_ms,
        accepted_at_ms,
    })
}

fn load_destination(connection: &Connection, id: Uuid) -> Result<Option<DestinationRecord>> {
    connection
        .query_row(
            "SELECT payload,revoked,accepted FROM destinations WHERE id=?1",
            [id.to_string()],
            destination_row,
        )
        .optional()?
        .map(decode_destination)
        .transpose()
}

fn is_active(connection: &Connection, id: Uuid, now: u64) -> Result<bool> {
    Ok(load_destination(connection, id)?.is_some_and(|record| {
        record.revoked_at_ms.is_none()
            && record.accepted_at_ms.is_some()
            && record.destination.expires_at_ms > now
    }))
}

fn active_destination(connection: &Connection, id: Uuid, now: u64) -> Result<DestinationRecord> {
    let record =
        load_destination(connection, id)?.ok_or_else(|| anyhow::anyhow!("unknown destination"))?;
    ensure!(
        record.revoked_at_ms.is_none()
            && record.accepted_at_ms.is_some()
            && record.destination.expires_at_ms > now,
        "inactive destination"
    );
    Ok(record)
}

fn rank(state: ReceiptState) -> i64 {
    match state {
        ReceiptState::Available => 0,
        ReceiptState::Seen => 1,
        ReceiptState::Dismissed => 2,
    }
}

fn receipt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NotificationReceipt> {
    let id: String = row.get(1)?;
    let event_id = Uuid::parse_str(&id).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let state = match row.get::<_, i64>(2)? {
        0 => ReceiptState::Available,
        1 => ReceiptState::Seen,
        2 => ReceiptState::Dismissed,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(NotificationReceipt {
        sequence: row.get(0)?,
        event_id,
        state,
    })
}

fn entry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(NotificationReceipt, String)> {
    Ok((receipt_row(row)?, row.get(3)?))
}

fn decode_entry((receipt, payload): (NotificationReceipt, String)) -> Result<InboxEntry> {
    let notification: Notification = serde_json::from_str(&payload)?;
    if notification.event_id != receipt.event_id {
        bail!("corrupt notification identity");
    }
    Ok(InboxEntry {
        receipt,
        notification,
    })
}

fn load_cursor(connection: &Connection, destination_id: Uuid) -> Result<ProducerCursor> {
    let (after, error): (u64, Option<String>) = connection.query_row(
        "SELECT cursor,producer_error FROM destinations WHERE id=?1",
        [destination_id.to_string()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(ProducerCursor {
        after,
        error: error
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::Cell, path::PathBuf};
    use voyage_protocol::notifications::QuietHoursUtc;

    struct Fixture {
        directory: PathBuf,
        now: Cell<u64>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                directory: std::env::temp_dir()
                    .join(format!("notification-store-{}", Uuid::new_v4())),
                now: Cell::new(1_000_000),
            }
        }
        fn open(&self) -> Store {
            Store::open(&self.directory).unwrap()
        }
        fn now(&self) -> u64 {
            self.now.get()
        }
        fn elapse(&self, delta: u64) {
            self.now.set(self.now() + delta);
        }
        fn destination(&self) -> Destination {
            Destination {
                id: Uuid::new_v4(),
                recipient_grant_id: Uuid::new_v4(),
                recipient_principal_id: Uuid::new_v4(),
                recipient_grant_revision: 1,
                source_vessel_id: Uuid::new_v4(),
                source_session_id: Uuid::new_v4(),
                event_kinds: vec![NotificationKind::Completed, NotificationKind::Incomplete],
                expires_at_ms: self.now() + 100_000,
                notification_ttl_ms: 10_000,
                quiet_hours_utc: Some(QuietHoursUtc {
                    start_minute: 0,
                    end_minute: 60,
                }),
            }
        }
        fn ready(&self, store: &Store) -> Destination {
            let destination = self.destination();
            store
                .configure(Uuid::new_v4(), destination.clone(), self.now())
                .unwrap();
            store
                .accept(Uuid::new_v4(), destination.id, self.now())
                .unwrap();
            destination
        }
        fn event(&self, destination: &Destination) -> Notification {
            let id = Uuid::new_v4();
            Notification {
                event_id: id,
                source_event_id: id,
                session_id: destination.source_session_id,
                run_id: Some(Uuid::new_v4()),
                incarnation: Some(Uuid::new_v4()),
                kind: NotificationKind::Completed,
                created_at_ms: self.now(),
                expires_at_ms: self.now() + 1000,
                decision_id: None,
                budget: None,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn revoked_authority_after_sqlite_wait_cannot_disclose_existing_payload() {
        use crate::process::{access::store as access, identity, registry};
        use voyage_protocol::process::{ProcessGrant, ProcessRegistration, ProcessRight};
        let fixture = Fixture::new();
        let mut store = fixture.open();
        let mut destination = fixture.destination();
        destination.source_vessel_id = identity::public(&fixture.directory).unwrap().vessel_id;
        let grant:ProcessGrant=serde_json::from_value(serde_json::json!({
            "grant_id":destination.recipient_grant_id,"principal_id":destination.recipient_principal_id,
            "session_id":destination.source_session_id,"workspace":fixture.directory,
            "revision":destination.recipient_grant_revision,"rights":["observe"],
            "expires_at_ms":access::now().unwrap()+60_000,"revoked":false,"token_hash":"synthetic-hash"
        })).unwrap();
        let registration:ProcessRegistration=serde_json::from_value(serde_json::json!({
            "protocol":1,"session_id":destination.source_session_id,"incarnation":Uuid::new_v4(),
            "command_id":Uuid::new_v4(),"workspace":fixture.directory,"state":"live","token":"synthetic-runtime"
        })).unwrap();
        registry::private_directory(&fixture.directory.join("access")).unwrap();
        registry::private_directory(&fixture.directory.join("access/grants")).unwrap();
        let grant_path = access::grant_path(&fixture.directory, grant.grant_id);
        access::save(&grant_path, &grant).unwrap();
        store
            .configure(Uuid::new_v4(), destination.clone(), fixture.now())
            .unwrap();
        store
            .accept(Uuid::new_v4(), destination.id, fixture.now())
            .unwrap();
        let event = fixture.event(&destination);
        store
            .publish(destination.id, event.clone(), fixture.now())
            .unwrap();
        store.authorize(
            &fixture.directory,
            &destination,
            ProcessRight::Observe,
            &registration,
        );
        assert!(
            store
                .get(destination.id, event.event_id, fixture.now())
                .unwrap()
                .is_some()
        );
        let blocker = Connection::open(fixture.directory.join("notifications.sqlite3")).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        let (started, observed) = std::sync::mpsc::channel();
        let now = fixture.now();
        let reader = std::thread::spawn(move || {
            started.send(()).unwrap();
            store.get(destination.id, event.event_id, now)
        });
        observed.recv_timeout(Duration::from_secs(1)).unwrap();
        let mut revoked = grant;
        revoked.revoked = true;
        access::save(&grant_path, &revoked).unwrap();
        blocker.execute_batch("ROLLBACK").unwrap();
        assert!(
            reader.join().unwrap().is_err(),
            "pre-wait authority disclosed revoked metadata"
        );
    }

    #[test]
    fn service_clock_and_attention_never_resurrect_expired_or_seen_payload() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let destination = fixture.ready(&store);
        let event = fixture.event(&destination);
        store
            .publish(destination.id, event.clone(), fixture.now())
            .unwrap();
        assert_eq!(
            store
                .attention_count(destination.id, fixture.now())
                .unwrap(),
            1
        );
        store
            .receipt(
                destination.id,
                event.event_id,
                ReceiptState::Seen,
                fixture.now(),
            )
            .unwrap();
        assert_eq!(
            store
                .attention_count(destination.id, fixture.now())
                .unwrap(),
            0
        );
        drop(store);
        // A stale caller clock cannot override the service's post-lock clock.
        let current = Store::open_current(&fixture.directory).unwrap();
        assert!(
            current
                .get(destination.id, event.event_id, fixture.now())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            current
                .attention_count(destination.id, fixture.now())
                .unwrap(),
            0
        );
    }

    #[test]
    fn consent_is_separate_and_commands_are_globally_immutable() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let destination = fixture.destination();
        let configure = Uuid::new_v4();
        let pending = store
            .configure(configure, destination.clone(), fixture.now())
            .unwrap();
        assert_eq!(pending.accepted_at_ms, None);
        let event = fixture.event(&destination);
        assert!(
            store
                .publish(destination.id, event.clone(), fixture.now())
                .is_err()
        );
        assert!(
            store
                .inbox(destination.id, 0, 10, fixture.now())
                .unwrap()
                .entries
                .is_empty()
        );
        assert!(
            store
                .accept(configure, destination.id, fixture.now())
                .is_err()
        );
        let accept = Uuid::new_v4();
        store.accept(accept, destination.id, fixture.now()).unwrap();
        store.accept(accept, destination.id, fixture.now()).unwrap();
        assert!(
            store
                .accept(Uuid::new_v4(), destination.id, fixture.now())
                .is_err()
        );
        store.publish(destination.id, event, fixture.now()).unwrap();
        let mut changed = destination.clone();
        changed.recipient_grant_revision += 1;
        assert!(store.configure(configure, changed, fixture.now()).is_err());
        assert!(
            store
                .configure(Uuid::new_v4(), destination.clone(), fixture.now())
                .is_err()
        );
        let revoke = Uuid::new_v4();
        store.revoke(revoke, destination.id, fixture.now()).unwrap();
        store.revoke(revoke, destination.id, fixture.now()).unwrap();
        assert!(
            store
                .revoke(Uuid::new_v4(), destination.id, fixture.now())
                .is_err()
        );
        let record = store
            .configure(configure, destination.clone(), fixture.now())
            .unwrap();
        assert!(record.revoked_at_ms.is_some());
        assert!(
            store
                .accept(accept, destination.id, fixture.now())
                .unwrap()
                .revoked_at_ms
                .is_some()
        );
        assert!(
            store
                .get(destination.id, Uuid::new_v4(), fixture.now())
                .unwrap()
                .is_none()
        );
        let other = fixture.destination();
        assert!(store.configure(revoke, other, fixture.now()).is_err());
        assert_eq!(store.destinations().unwrap().len(), 1);
    }

    #[test]
    fn publish_dedup_fingerprints_survive_expiry_revoke_and_reopen() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let destination = fixture.ready(&store);
        let event = fixture.event(&destination);
        let receipt = store
            .publish(destination.id, event.clone(), fixture.now())
            .unwrap();
        assert_eq!(
            store
                .publish(destination.id, event.clone(), fixture.now())
                .unwrap(),
            receipt
        );
        let mut changed = event.clone();
        changed.kind = NotificationKind::Incomplete;
        assert!(
            store
                .publish(destination.id, changed, fixture.now())
                .is_err()
        );
        let mut alias = event.clone();
        alias.event_id = Uuid::new_v4();
        assert!(store.publish(destination.id, alias, fixture.now()).is_err());
        fixture.elapse(1000);
        assert!(
            store
                .get(destination.id, event.event_id, fixture.now())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store
                .publish(destination.id, event.clone(), fixture.now())
                .unwrap(),
            receipt
        );
        let retained: (i64, i64) = store
            .connection
            .query_row("SELECT count(*),count(payload) FROM events", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .unwrap();
        assert_eq!(retained, (1, 0));
        store
            .revoke(Uuid::new_v4(), destination.id, fixture.now())
            .unwrap();
        drop(store);
        let reopened = fixture.open();
        assert_eq!(
            reopened
                .publish(destination.id, event.clone(), fixture.now())
                .unwrap(),
            receipt
        );
        assert!(
            reopened
                .inbox(destination.id, 0, 10, fixture.now())
                .unwrap()
                .entries
                .is_empty()
        );
        assert!(
            reopened
                .publish(destination.id, fixture.event(&destination), fixture.now())
                .is_err()
        );
        assert!(
            reopened
                .configure(Uuid::new_v4(), destination, fixture.now())
                .is_err()
        );
    }

    #[test]
    fn monotonic_paging_and_independent_receipts_across_handles() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let first = fixture.ready(&store);
        let mut second = fixture.destination();
        second.source_session_id = first.source_session_id;
        store
            .configure(Uuid::new_v4(), second.clone(), fixture.now())
            .unwrap();
        store
            .accept(Uuid::new_v4(), second.id, fixture.now())
            .unwrap();
        let event = fixture.event(&first);
        let a = store
            .publish(first.id, event.clone(), fixture.now())
            .unwrap();
        let b = store
            .publish(second.id, event.clone(), fixture.now())
            .unwrap();
        let later = fixture.event(&first);
        let c = store.publish(first.id, later, fixture.now()).unwrap();
        assert!(a.sequence < b.sequence && b.sequence < c.sequence);
        let page = store.inbox(first.id, 0, 1, fixture.now()).unwrap();
        assert!(page.has_more);
        assert_eq!(page.entries[0].receipt, a);
        let next = store
            .inbox(first.id, page.next_after, 1, fixture.now())
            .unwrap();
        assert!(!next.has_more);
        assert_eq!(next.entries[0].receipt, c);
        let empty = store
            .inbox(first.id, next.next_after, 1, fixture.now())
            .unwrap();
        assert_eq!(empty.next_after, c.sequence);
        assert!(empty.entries.is_empty());
        let other_interface = fixture.open();
        store
            .receipt(
                first.id,
                event.event_id,
                ReceiptState::Dismissed,
                fixture.now(),
            )
            .unwrap();
        assert_eq!(
            other_interface
                .receipt(first.id, event.event_id, ReceiptState::Seen, fixture.now())
                .unwrap()
                .state,
            ReceiptState::Dismissed
        );
        assert_eq!(
            other_interface
                .get(second.id, event.event_id, fixture.now())
                .unwrap()
                .unwrap()
                .receipt
                .state,
            ReceiptState::Available
        );
        assert!(
            store
                .receipt(
                    Uuid::new_v4(),
                    event.event_id,
                    ReceiptState::Seen,
                    fixture.now()
                )
                .is_err()
        );
        assert!(store.inbox(first.id, 0, 0, fixture.now()).is_err());
        assert!(
            store
                .inbox(first.id, 0, MAX_INBOX_PAGE + 1, fixture.now())
                .is_err()
        );
        assert!(store.inbox(first.id, u64::MAX, 1, fixture.now()).is_err());
    }

    #[test]
    fn capacity_preserves_retry_settlement_revocation_and_cursor_failure() {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        store.event_capacity = 1;
        let destination = fixture.ready(&store);
        let event = fixture.event(&destination);
        let receipt = store
            .publish(destination.id, event.clone(), fixture.now())
            .unwrap();
        store.advance(destination.id, 5, None).unwrap();
        assert!(
            store
                .publish(destination.id, fixture.event(&destination), fixture.now())
                .is_err()
        );
        let failed = store
            .advance(destination.id, 99, Some(ProducerError::Capacity))
            .unwrap();
        assert_eq!(failed.after, 5);
        assert_eq!(failed.error, Some(ProducerError::Capacity));
        assert_eq!(store.advance(destination.id, 4, None).unwrap(), failed);
        assert_eq!(
            store
                .publish(destination.id, event.clone(), fixture.now())
                .unwrap(),
            receipt
        );
        store
            .receipt(
                destination.id,
                event.event_id,
                ReceiptState::Seen,
                fixture.now(),
            )
            .unwrap();
        store
            .revoke(Uuid::new_v4(), destination.id, fixture.now())
            .unwrap();
        assert_eq!(
            store
                .receipt(
                    destination.id,
                    event.event_id,
                    ReceiptState::Dismissed,
                    fixture.now()
                )
                .unwrap()
                .state,
            ReceiptState::Dismissed
        );
        assert_eq!(
            store
                .publish(destination.id, event.clone(), fixture.now())
                .unwrap()
                .state,
            ReceiptState::Dismissed
        );
        assert_eq!(store.advance(destination.id, 6, None).unwrap().after, 6);
        drop(store);
        assert_eq!(
            fixture.open().cursor(destination.id).unwrap(),
            ProducerCursor {
                after: 6,
                error: None
            }
        );
    }

    #[test]
    fn per_destination_and_configuration_quotas_are_lifetime_bounds() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let destination = fixture.ready(&store);
        // Seed through SQL only to keep this focused quota test small; public
        // insertion/duplicate paths are exercised in the other cases.
        for _ in 0..MAX_EVENTS_PER_DESTINATION {
            let event = fixture.event(&destination);
            store.connection.execute("INSERT INTO events(destination_id,event_id,source_event_id,fingerprint,payload,expires) VALUES(?1,?2,?2,zeroblob(32),NULL,0)", params![destination.id.to_string(), event.event_id.to_string()]).unwrap();
        }
        assert!(
            store
                .publish(destination.id, fixture.event(&destination), fixture.now())
                .is_err()
        );
        for _ in 1..MAX_DESTINATIONS_PER_GRANT {
            let mut another = fixture.destination();
            another.recipient_grant_id = destination.recipient_grant_id;
            store
                .configure(Uuid::new_v4(), another, fixture.now())
                .unwrap();
        }
        store
            .revoke(Uuid::new_v4(), destination.id, fixture.now())
            .unwrap();
        let mut overflow = fixture.destination();
        overflow.recipient_grant_id = destination.recipient_grant_id;
        assert!(
            store
                .configure(Uuid::new_v4(), overflow, fixture.now())
                .is_err()
        );
    }

    #[test]
    fn test_command_freezes_timestamp_and_cannot_alias_other_operations() {
        let fixture = Fixture::new();
        let mut store = fixture.open();
        store.event_capacity = 1;
        let destination = fixture.ready(&store);
        let command = Uuid::new_v4();
        let incarnation = Uuid::new_v4();
        let accepted = store
            .test(command, destination.id, incarnation, fixture.now())
            .unwrap();
        let event = store
            .get(destination.id, command, fixture.now())
            .unwrap()
            .unwrap()
            .notification;
        assert_eq!(event.kind, NotificationKind::Test);
        assert_eq!(event.created_at_ms, fixture.now());
        assert_eq!(event.run_id, None);
        fixture.elapse(10_000);
        assert_eq!(
            store
                .test(command, destination.id, incarnation, fixture.now())
                .unwrap(),
            accepted
        );
        assert_eq!(
            store
                .test(command, destination.id, Uuid::new_v4(), fixture.now())
                .unwrap(),
            accepted
        );
        assert!(
            store
                .test(command, Uuid::new_v4(), incarnation, fixture.now())
                .is_err()
        );
        assert!(
            store
                .revoke(command, destination.id, fixture.now())
                .is_err()
        );
        assert!(
            store
                .test(Uuid::new_v4(), destination.id, incarnation, fixture.now())
                .is_err()
        );
        let revoke = Uuid::new_v4();
        store.revoke(revoke, destination.id, fixture.now()).unwrap();
        drop(store);
        assert_eq!(
            fixture
                .open()
                .test(command, destination.id, incarnation, fixture.now())
                .unwrap(),
            accepted
        );
    }

    #[test]
    fn clock_rollback_cannot_resurrect_payload_or_block_revocation() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let destination = fixture.ready(&store);
        let event = fixture.event(&destination);
        store
            .publish(destination.id, event.clone(), fixture.now())
            .unwrap();
        let original = fixture.now();
        fixture.elapse(1000);
        assert!(
            store
                .get(destination.id, event.event_id, fixture.now())
                .unwrap()
                .is_none()
        );
        assert!(store.get(destination.id, event.event_id, original).is_err());
        assert!(
            store
                .publish(destination.id, fixture.event(&destination), original)
                .is_err()
        );
        store
            .revoke(Uuid::new_v4(), destination.id, original)
            .unwrap();
        assert_eq!(
            store
                .destination(destination.id)
                .unwrap()
                .unwrap()
                .revoked_at_ms,
            Some(fixture.now())
        );
        store.publish(destination.id, event, original).unwrap();
        assert!(store.get(destination.id, Uuid::new_v4(), u64::MAX).is_err());
    }

    #[test]
    fn expiry_bounds_preferences_and_provenance_are_validated() {
        let fixture = Fixture::new();
        let store = fixture.open();
        let base = fixture.destination();
        for field in 0..8 {
            let mut invalid = base.clone();
            match field {
                0 => invalid.expires_at_ms = fixture.now(),
                1 => invalid.expires_at_ms = fixture.now() + MAX_DESTINATION_LIFETIME_MS + 1,
                2 => invalid.notification_ttl_ms = 0,
                3 => invalid.notification_ttl_ms = MAX_NOTIFICATION_TTL_MS + 1,
                4 => invalid.event_kinds.push(NotificationKind::Completed),
                5 => invalid.recipient_grant_revision = 0,
                6 => invalid.recipient_principal_id = Uuid::nil(),
                _ => {
                    invalid.quiet_hours_utc = Some(QuietHoursUtc {
                        start_minute: 1440,
                        end_minute: 0,
                    })
                }
            }
            assert!(
                store
                    .configure(Uuid::new_v4(), invalid, fixture.now())
                    .is_err()
            );
        }
        let destination = fixture.ready(&store);
        for field in 0..7 {
            let mut invalid = fixture.event(&destination);
            match field {
                0 => invalid.session_id = Uuid::new_v4(),
                1 => invalid.created_at_ms = fixture.now() + 1,
                2 => invalid.expires_at_ms = fixture.now(),
                3 => invalid.expires_at_ms = fixture.now() + destination.notification_ttl_ms + 1,
                4 => invalid.incarnation = Some(Uuid::nil()),
                5 => invalid.kind = NotificationKind::Failed,
                _ => invalid.decision_id = Some(Uuid::new_v4()),
            }
            assert!(
                store
                    .publish(destination.id, invalid, fixture.now())
                    .is_err()
            );
        }
        let mut unknown_incarnation = fixture.event(&destination);
        unknown_incarnation.incarnation = None;
        store
            .publish(destination.id, unknown_incarnation, fixture.now())
            .unwrap();
        // Explicit UTC quiet hours do not hide available inbox references.
        assert_eq!(
            store
                .inbox(destination.id, 0, 10, fixture.now())
                .unwrap()
                .entries
                .len(),
            1
        );
        fixture.elapse(100_000);
        assert!(
            store
                .inbox(destination.id, 0, 10, fixture.now())
                .unwrap()
                .entries
                .is_empty()
        );
        assert!(
            store
                .publish(destination.id, fixture.event(&destination), fixture.now())
                .is_err()
        );
    }

    #[test]
    fn private_storage_rejects_symlinks_and_public_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let fixture = Fixture::new();
        drop(fixture.open());
        let database = fixture.directory.join("notifications.sqlite3");
        fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(Store::open(&fixture.directory).is_err());
        fs::set_permissions(&database, fs::Permissions::from_mode(0o600)).unwrap();
        let alias = fixture.directory.join("alias");
        symlink(&fixture.directory, &alias).unwrap();
        assert!(Store::open(&alias).is_err());
        let journal = fixture.directory.join("notifications.sqlite3-journal");
        fs::remove_file(&journal).unwrap();
        symlink(&database, &journal).unwrap();
        assert!(Store::open(&fixture.directory).is_err());
    }
}
