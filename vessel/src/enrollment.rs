//! Dedicated single-owner enrollment authority. No legacy database is adopted.
//! Callers must authenticate the operator before creating invitations or invoking
//! administrative revocation. Machine operations require fresh signed challenges.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::fs;
use std::path::Path;
#[cfg(any(unix, windows, test))]
use std::time::Duration;
use uuid::Uuid;
use voyage_protocol::enrollment::{Challenge, ProofOperation, SignedChallenge};

const MAX_RECORDS: i64 = 10_000;
const MAX_CHALLENGES: i64 = 4096;
const RECOVERY_MS: i64 = 300_000;
const CHALLENGE_MS: i64 = 60_000;
#[cfg(any(unix, windows, test))]
const MAX_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnrollmentError {
    Invalid,
    Denied,
    Conflict,
    Busy,
    Capacity,
    Storage,
    Unsupported,
}
impl std::fmt::Display for EnrollmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid enrollment request",
            Self::Denied => "enrollment authorization denied",
            Self::Conflict => "enrollment state conflict",
            Self::Busy => "enrollment storage busy",
            Self::Capacity => "enrollment capacity reached",
            Self::Storage => "enrollment storage failed",
            Self::Unsupported => "private enrollment storage is unsupported on this platform",
        })
    }
}
impl std::error::Error for EnrollmentError {}
impl From<rusqlite::Error> for EnrollmentError {
    fn from(e: rusqlite::Error) -> Self {
        match e.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                Self::Busy
            }
            Some(rusqlite::ErrorCode::DiskFull) => Self::Capacity,
            _ => Self::Storage,
        }
    }
}
type Result<T> = std::result::Result<T, EnrollmentError>;

/// Secret returned exactly to the authenticated operator; never log or Debug it.
#[derive(Serialize)]
pub struct Invitation {
    pub id: Uuid,
    pub key: String,
    pub expires_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Receipt {
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
    pub revoked: bool,
}

pub struct EnrollmentStore {
    db: Connection,
    #[cfg(windows)]
    _private_directory: Option<voyage_storage::PrivateDirectory>,
    origin: String,
    owner: Uuid,
    challenge_key: ring::hmac::Key,
}
struct Machine {
    key: [u8; 32],
    epoch: i64,
    revoked: bool,
}
struct Recovery {
    digest: Vec<u8>,
    key: [u8; 32],
    machine_id: Uuid,
    epoch: i64,
    revoked: bool,
    expires: i64,
}

/// No credentials, paths, queries, fragments, redirects, or non-TLS remote origins.
/// Development HTTP is allowed only for literal loopback IP addresses, not DNS.
pub fn validate_origin(origin: &str, allow_loopback_http: bool) -> Result<String> {
    if origin.len() > 2048 || origin.chars().any(char::is_control) || origin.trim() != origin {
        return Err(EnrollmentError::Invalid);
    }
    let url = url::Url::parse(origin).map_err(|_| EnrollmentError::Invalid)?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && allow_loopback_http && loopback))
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(EnrollmentError::Invalid);
    }
    Ok(url.origin().ascii_serialization())
}

impl EnrollmentStore {
    pub fn open(directory: &Path, origin: &str, allow_loopback_http: bool) -> Result<Self> {
        let origin = validate_origin(origin, allow_loopback_http)?;
        // Never substitute inherited default permissions for native verification.
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (directory, origin);
            Err(EnrollmentError::Unsupported)
        }
        #[cfg(windows)]
        {
            let private =
                voyage_storage::PrivateDirectory::open(directory).map_err(storage_error)?;
            // PERSIST journaling keeps the securely precreated journal's explicit
            // account owner, including when an elevated token defaults to a group.
            drop(
                private
                    .open_file("enrollment.sqlite3", true)
                    .map_err(storage_error)?,
            );
            drop(
                private
                    .open_file("enrollment.sqlite3-journal", true)
                    .map_err(storage_error)?,
            );
            for name in ["enrollment.sqlite3-wal", "enrollment.sqlite3-shm"] {
                match private.open_file(name, false) {
                    Ok(file) => drop(file),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(storage_error(error)),
                }
            }
            let mut store = Self::initialize(
                Connection::open(private.path().join("enrollment.sqlite3"))?,
                origin,
            )?;
            // Verify SQLite retained the owner-only files before serving requests.
            drop(
                private
                    .open_file("enrollment.sqlite3", false)
                    .map_err(storage_error)?,
            );
            drop(
                private
                    .open_file("enrollment.sqlite3-journal", false)
                    .map_err(storage_error)?,
            );
            store._private_directory = Some(private);
            Ok(store)
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(directory) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(_) => return Err(EnrollmentError::Storage),
            }
            let private = |p: &Path, dir: bool| -> Result<()> {
                let m = fs::symlink_metadata(p).map_err(|_| EnrollmentError::Storage)?;
                if m.file_type().is_symlink()
                    || (dir && !m.is_dir())
                    || (!dir && !m.is_file())
                    || m.mode() & 0o077 != 0
                    || m.uid() != unsafe { libc::geteuid() }
                {
                    return Err(EnrollmentError::Denied);
                }
                Ok(())
            };
            private(directory, true)?;
            let path = directory.join("enrollment.sqlite3");
            if path.symlink_metadata().is_ok() {
                private(&path, false)?;
            }
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| EnrollmentError::Storage)?;
            private(&path, false)?;
            if !file
                .metadata()
                .map_err(|_| EnrollmentError::Storage)?
                .is_file()
            {
                return Err(EnrollmentError::Denied);
            }
            drop(file);
            let store = Self::initialize(Connection::open(path)?, origin)?;
            fs::File::open(directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| EnrollmentError::Storage)?;
            fs::File::open(directory.parent().ok_or(EnrollmentError::Invalid)?)
                .and_then(|f| f.sync_all())
                .map_err(|_| EnrollmentError::Storage)?;
            Ok(store)
        }
    }

    #[cfg(any(unix, windows, test))]
    fn initialize(mut db: Connection, origin: String) -> Result<Self> {
        db.busy_timeout(Duration::ZERO)?;
        #[cfg(windows)]
        configure_private_journal(&db)?;
        db.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
        let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_schema: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='enrollment_schema')", [], |r| r.get(0))?;
        let owner;
        let challenge_secret: Vec<u8>;
        if has_schema {
            let (version, saved_origin, saved_owner, secret): (i64, String, String, Vec<u8>) = tx
                .query_row(
                "SELECT version,origin,owner,challenge_key FROM enrollment_schema WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            challenge_secret = secret;
            if version != 2 || saved_origin != origin {
                return Err(EnrollmentError::Conflict);
            }
            owner = Uuid::parse_str(&saved_owner).map_err(|_| EnrollmentError::Storage)?;
            if owner.is_nil() {
                return Err(EnrollmentError::Storage);
            }
        } else {
            let count: i64 = tx.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'", [], |r| r.get(0))?;
            if count != 0 {
                return Err(EnrollmentError::Conflict);
            }
            owner = Uuid::new_v4();
            let mut secret = [0u8; 32];
            SystemRandom::new()
                .fill(&mut secret)
                .map_err(|_| EnrollmentError::Storage)?;
            challenge_secret = secret.to_vec();
            tx.execute_batch("CREATE TABLE enrollment_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL,origin TEXT NOT NULL,owner TEXT NOT NULL,last_time INTEGER NOT NULL,challenge_key BLOB NOT NULL);
                CREATE TABLE invitations(id TEXT PRIMARY KEY,key_hash BLOB NOT NULL,expires INTEGER NOT NULL,transaction_id TEXT UNIQUE);
                CREATE TABLE machines(id TEXT PRIMARY KEY,public_key BLOB NOT NULL UNIQUE,epoch INTEGER NOT NULL CHECK(epoch>0),revoked INTEGER NOT NULL CHECK(revoked IN(0,1)));
                CREATE TABLE challenges(id TEXT PRIMARY KEY,machine_id TEXT NOT NULL,body TEXT NOT NULL,expires INTEGER NOT NULL,used INTEGER NOT NULL CHECK(used IN(0,1)));
                CREATE INDEX challenges_machine ON challenges(machine_id);
                CREATE TABLE receipts(id TEXT PRIMARY KEY,digest BLOB NOT NULL,authorization_key BLOB NOT NULL,machine_id TEXT NOT NULL REFERENCES machines(id),epoch INTEGER NOT NULL,revoked INTEGER NOT NULL,expires INTEGER NOT NULL);
                CREATE TABLE audit(sequence INTEGER PRIMARY KEY,machine_id TEXT,kind TEXT NOT NULL,time INTEGER NOT NULL);")?;
            tx.execute(
                "INSERT INTO enrollment_schema VALUES(1,2,?1,?2,0,?3)",
                params![origin, owner.to_string(), &challenge_secret],
            )?;
        }
        tx.commit()?;
        #[cfg(not(windows))]
        db.pragma_update(None, "journal_mode", "DELETE")?;
        let size: i64 = db.pragma_query_value(None, "page_size", |r| r.get(0))?;
        let pages: i64 = db.pragma_query_value(None, "page_count", |r| r.get(0))?;
        if size <= 0 || pages > MAX_BYTES / size {
            return Err(EnrollmentError::Capacity);
        }
        db.pragma_update(None, "max_page_count", MAX_BYTES / size)?;
        if challenge_secret.len() != 32 {
            return Err(EnrollmentError::Storage);
        }
        Ok(Self {
            db,
            #[cfg(windows)]
            _private_directory: None,
            origin,
            owner,
            challenge_key: ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &challenge_secret),
        })
    }

    fn observe_time(&mut self, now: i64) -> Result<()> {
        // Commit a watermark even if subsequent authorization/expiry is denied.
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_clock(&tx, now)?;
        tx.commit()?;
        Ok(())
    }

    pub fn owner_id(&self) -> Uuid {
        self.owner
    }
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Operator-authorized creation only. A join key is not operator authorization.
    pub fn invite(&mut self, ttl_ms: i64, now: i64) -> Result<Invitation> {
        if !(1..=900_000).contains(&ttl_ms) {
            return Err(EnrollmentError::Invalid);
        }
        self.observe_time(now)?;
        let expires = now.checked_add(ttl_ms).ok_or(EnrollmentError::Invalid)?;
        let mut entropy = [0u8; 32];
        SystemRandom::new()
            .fill(&mut entropy)
            .map_err(|_| EnrollmentError::Storage)?;
        let key = URL_SAFE_NO_PAD.encode(entropy);
        let invitation = Invitation {
            id: Uuid::new_v4(),
            key,
            expires_at_ms: expires,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_clock(&tx, now)?;
        tx.execute("DELETE FROM invitations WHERE expires<=?1", [now])?;
        capacity(&tx, "invitations", MAX_RECORDS)?;
        tx.execute(
            "INSERT INTO invitations VALUES(?1,?2,?3,NULL)",
            params![
                invitation.id.to_string(),
                hash(invitation.key.as_bytes()),
                expires
            ],
        )?;
        audit(&tx, None, "invitation_created", now)?;
        tx.commit()?;
        Ok(invitation)
    }

    pub fn challenge(
        &mut self,
        operation: ProofOperation,
        invitation_key: Option<&str>,
        now: i64,
    ) -> Result<Challenge> {
        self.observe_time(now)?;
        let mut challenge = Challenge {
            version: 2,
            id: Uuid::new_v4(),
            origin: self.origin.clone(),
            expires_at_ms: now + CHALLENGE_MS,
            server_tag: vec![0; 32],
            operation,
        };
        challenge
            .validate(&self.origin, now)
            .map_err(|_| EnrollmentError::Invalid)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_clock(&tx, now)?;
        authorization_key(&tx, &challenge.operation, invitation_key, now)?;
        // Stateless issuance: public machine identifiers cannot reserve victim
        // slots. Only successfully authenticated completions allocate replay IDs.
        challenge.server_tag.clear();
        let bytes = challenge
            .signing_bytes()
            .map_err(|_| EnrollmentError::Invalid)?;
        challenge.server_tag = ring::hmac::sign(&self.challenge_key, &bytes)
            .as_ref()
            .to_vec();
        tx.commit()?;
        Ok(challenge)
    }

    /// Consume a fresh MAC-bound client proof atomically with its effect. Lost
    /// responses recover using the original transaction and a new challenge.
    pub fn complete(
        &mut self,
        proof: &SignedChallenge,
        invitation_key: Option<&str>,
        now: i64,
    ) -> Result<Receipt> {
        self.observe_time(now)?;
        proof
            .challenge
            .validate(&self.origin, now)
            .map_err(|_| EnrollmentError::Denied)?;
        let mut unsigned = proof.challenge.clone();
        unsigned.server_tag.clear();
        let bytes = unsigned
            .signing_bytes()
            .map_err(|_| EnrollmentError::Invalid)?;
        ring::hmac::verify(&self.challenge_key, &bytes, &proof.challenge.server_tag)
            .map_err(|_| EnrollmentError::Denied)?;
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_clock(&tx, now)?;
        let used: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM challenges WHERE id=?1)",
            [proof.challenge.id.to_string()],
            |r| r.get(0),
        )?;
        if used {
            return Err(EnrollmentError::Denied);
        }
        let key = authorization_key(&tx, &proof.challenge.operation, invitation_key, now)?;
        proof
            .verify(&key, &self.origin, now)
            .map_err(|_| EnrollmentError::Denied)?;
        tx.execute("DELETE FROM challenges WHERE expires<=?1", [now])?;
        let revoking = matches!(proof.challenge.operation, ProofOperation::Revoke { .. });
        let already_resolved = recovery(&tx, &proof.challenge.operation, now)?.is_some();
        if !revoking || already_resolved {
            capacity(&tx, "challenges", MAX_CHALLENGES)?;
            let count: i64 = tx.query_row(
                "SELECT count(*) FROM challenges WHERE machine_id=?1",
                [machine_id(&proof.challenge.operation).to_string()],
                |r| r.get(0),
            )?;
            if count >= 16 {
                return Err(EnrollmentError::Capacity);
            }
        }
        let receipt = apply(&tx, &proof.challenge.operation, &key, self.owner, now)?;
        tx.execute(
            "INSERT INTO challenges VALUES(?1,?2,?3,?4,1)",
            params![
                proof.challenge.id.to_string(),
                receipt.machine_id.to_string(),
                json(&proof.challenge)?,
                proof.challenge.expires_at_ms
            ],
        )?;
        tx.commit()?;
        Ok(receipt)
    }

    /// Current epoch check is required on EVERY connection lease and operation,
    /// not just at the start of a long-lived socket. Never persist a positive cache.
    pub fn current(&self, id: Uuid, epoch: u64) -> Result<Receipt> {
        let m = machine(&self.db, id)?;
        if m.revoked || m.epoch as u64 != epoch {
            return Err(EnrollmentError::Denied);
        }
        Ok(Receipt {
            machine_id: id,
            owner_id: self.owner,
            epoch,
            revoked: false,
        })
    }

    /// Separate operator-authenticated authority, usable when the machine is lost.
    pub fn revoke(
        &mut self,
        id: Uuid,
        expected_epoch: u64,
        transaction_id: Uuid,
        now: i64,
    ) -> Result<Receipt> {
        if id.is_nil()
            || transaction_id.is_nil()
            || expected_epoch == 0
            || expected_epoch >= i64::MAX as u64
        {
            return Err(EnrollmentError::Invalid);
        }
        self.observe_time(now)?;
        let op = ProofOperation::Revoke {
            machine_id: id,
            epoch: expected_epoch,
            transaction_id,
        };
        let tx = self
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_clock(&tx, now)?;
        let key = authorization_key(&tx, &op, None, now)?;
        let receipt = apply(&tx, &op, &key, self.owner, now)?;

        tx.commit()?;
        Ok(receipt)
    }
}

#[cfg(windows)]
fn storage_error(error: std::io::Error) -> EnrollmentError {
    match error.kind() {
        std::io::ErrorKind::Unsupported => EnrollmentError::Unsupported,
        _ => EnrollmentError::Storage,
    }
}

fn check_clock(tx: &Transaction<'_>, now: i64) -> Result<()> {
    let previous: i64 = tx.query_row(
        "SELECT last_time FROM enrollment_schema WHERE id=1",
        [],
        |r| r.get(0),
    )?;
    if now < 0 || now < previous || now > i64::MAX - 900_000 {
        return Err(EnrollmentError::Invalid);
    }
    tx.execute(
        "UPDATE enrollment_schema SET last_time=?1 WHERE id=1",
        [now],
    )?;
    Ok(())
}
fn capacity(tx: &Transaction<'_>, table: &str, max: i64) -> Result<()> {
    // table names are exclusively internal constants, never request input.
    let n: i64 = tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
    if n >= max {
        Err(EnrollmentError::Capacity)
    } else {
        Ok(())
    }
}
fn audit(tx: &Transaction<'_>, id: Option<Uuid>, kind: &str, now: i64) -> Result<()> {
    capacity(
        tx,
        "audit",
        MAX_RECORDS * if kind == "revoked" { 11 } else { 10 },
    )?;
    tx.execute(
        "INSERT INTO audit(machine_id,kind,time) VALUES(?1,?2,?3)",
        params![id.map(|x| x.to_string()), kind, now],
    )?;
    Ok(())
}
fn json(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(|_| EnrollmentError::Invalid)
}
fn hash(value: &[u8]) -> Vec<u8> {
    Sha256::digest(value).to_vec()
}
fn machine_id(op: &ProofOperation) -> Uuid {
    match op {
        ProofOperation::Enroll { machine_id, .. }
        | ProofOperation::Connect { machine_id, .. }
        | ProofOperation::Rotate { machine_id, .. }
        | ProofOperation::Revoke { machine_id, .. } => *machine_id,
    }
}
fn transaction_id(op: &ProofOperation) -> Option<Uuid> {
    match op {
        ProofOperation::Enroll { transaction_id, .. }
        | ProofOperation::Rotate { transaction_id, .. }
        | ProofOperation::Revoke { transaction_id, .. } => Some(*transaction_id),
        _ => None,
    }
}
fn machine(db: &Connection, id: Uuid) -> Result<Machine> {
    let row: Option<(Vec<u8>, i64, bool)> = db
        .query_row(
            "SELECT public_key,epoch,revoked FROM machines WHERE id=?1",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((key, epoch, revoked)) = row else {
        return Err(EnrollmentError::Denied);
    };
    if epoch <= 0 {
        return Err(EnrollmentError::Storage);
    }
    Ok(Machine {
        key: key.try_into().map_err(|_| EnrollmentError::Storage)?,
        epoch,
        revoked,
    })
}
fn recovery(tx: &Transaction<'_>, op: &ProofOperation, now: i64) -> Result<Option<Recovery>> {
    let Some(id) = transaction_id(op) else {
        return Ok(None);
    };
    type RecoveryRow = (Vec<u8>, Vec<u8>, String, i64, bool, i64);
    let row: Option<RecoveryRow> = tx.query_row("SELECT digest,authorization_key,machine_id,epoch,revoked,expires FROM receipts WHERE id=?1",[id.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    let Some((digest, key, machine_id, epoch, revoked, expires)) = row else {
        return Ok(None);
    };
    let value = Recovery {
        digest,
        key: key.try_into().map_err(|_| EnrollmentError::Storage)?,
        machine_id: Uuid::parse_str(&machine_id).map_err(|_| EnrollmentError::Storage)?,
        epoch,
        revoked,
        expires,
    };
    if value.expires <= now || value.digest != hash(json(op)?.as_bytes()) {
        return Err(EnrollmentError::Conflict);
    }
    let m = machine(tx, value.machine_id)?;
    if m.epoch != value.epoch || m.revoked != value.revoked {
        return Err(EnrollmentError::Denied);
    }
    Ok(Some(value))
}
fn authorization_key(
    tx: &Transaction<'_>,
    op: &ProofOperation,
    invitation_key: Option<&str>,
    now: i64,
) -> Result<[u8; 32]> {
    if let Some(r) = recovery(tx, op, now)? {
        return Ok(r.key);
    }
    match op {
        ProofOperation::Enroll {
            invitation_id,
            public_key,
            ..
        } => {
            let key = invitation_key
                .filter(|key| key.len() == 43)
                .ok_or(EnrollmentError::Denied)?;
            let row: Option<(Vec<u8>, i64, Option<String>)> = tx
                .query_row(
                    "SELECT key_hash,expires,transaction_id FROM invitations WHERE id=?1",
                    [invitation_id.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((expected, expires, used)) = row else {
                return Err(EnrollmentError::Denied);
            };
            // Hash equality does not compare secrets; both digests have fixed size.
            let supplied = hash(key.as_bytes());
            if used.is_some()
                || expires <= now
                || expected.len() != 32
                || !expected
                    .iter()
                    .zip(&supplied)
                    .fold(true, |same, (a, b)| same & (a == b))
            {
                return Err(EnrollmentError::Denied);
            }
            Ok(*public_key)
        }
        ProofOperation::Connect { machine_id, epoch }
        | ProofOperation::Rotate {
            machine_id, epoch, ..
        }
        | ProofOperation::Revoke {
            machine_id, epoch, ..
        } => {
            if invitation_key.is_some() {
                return Err(EnrollmentError::Invalid);
            }
            let m = machine(tx, *machine_id)?;
            if m.revoked || m.epoch as u64 != *epoch {
                return Err(EnrollmentError::Denied);
            }
            Ok(m.key)
        }
    }
}
fn apply(
    tx: &Transaction<'_>,
    op: &ProofOperation,
    key: &[u8; 32],
    owner: Uuid,
    now: i64,
) -> Result<Receipt> {
    if let Some(r) = recovery(tx, op, now)? {
        return Ok(Receipt {
            machine_id: r.machine_id,
            owner_id: owner,
            epoch: r.epoch as u64,
            revoked: r.revoked,
        });
    }
    let id = machine_id(op);
    let (epoch, revoked, kind) = match op {
        ProofOperation::Enroll {
            invitation_id,
            transaction_id,
            public_key,
            ..
        } => {
            capacity(tx, "machines", MAX_RECORDS)?;
            let collision: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM machines WHERE id=?1 OR public_key=?2)",
                params![id.to_string(), public_key.as_slice()],
                |r| r.get(0),
            )?;
            if collision {
                return Err(EnrollmentError::Conflict);
            }
            tx.execute(
                "INSERT INTO machines VALUES(?1,?2,1,0)",
                params![id.to_string(), public_key.as_slice()],
            )?;
            let n = tx.execute(
                "UPDATE invitations SET transaction_id=?1 WHERE id=?2 AND transaction_id IS NULL",
                params![transaction_id.to_string(), invitation_id.to_string()],
            )?;
            if n != 1 {
                return Err(EnrollmentError::Conflict);
            }
            (1, false, "enrolled")
        }
        ProofOperation::Connect { epoch, .. } => (*epoch as i64, false, "authenticated"),
        ProofOperation::Rotate {
            epoch,
            new_public_key,
            ..
        } => {
            if new_public_key == key || *epoch >= i64::MAX as u64 - 1 {
                return Err(EnrollmentError::Invalid);
            }
            let next = i64::try_from(*epoch)
                .ok()
                .and_then(|x| x.checked_add(1))
                .ok_or(EnrollmentError::Invalid)?;
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM machines WHERE public_key=?1)",
                [new_public_key.as_slice()],
                |r| r.get(0),
            )?;
            if exists {
                return Err(EnrollmentError::Conflict);
            }
            tx.execute(
                "UPDATE machines SET public_key=?1,epoch=?2 WHERE id=?3",
                params![new_public_key.as_slice(), next, id.to_string()],
            )?;
            (next, false, "rotated")
        }
        ProofOperation::Revoke { epoch, .. } => {
            let next = i64::try_from(*epoch)
                .ok()
                .and_then(|x| x.checked_add(1))
                .ok_or(EnrollmentError::Invalid)?;
            tx.execute(
                "UPDATE machines SET revoked=1,epoch=?1 WHERE id=?2",
                params![next, id.to_string()],
            )?;
            (next, true, "revoked")
        }
    };
    if let Some(transaction_id) = transaction_id(op) {
        capacity(tx, "receipts", MAX_RECORDS * if revoked { 2 } else { 1 })?;
        tx.execute(
            "INSERT INTO receipts VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                transaction_id.to_string(),
                hash(json(op)?.as_bytes()),
                key.as_slice(),
                id.to_string(),
                epoch,
                revoked,
                now + RECOVERY_MS
            ],
        )?;
    }
    audit(tx, Some(id), kind, now)?;
    Ok(Receipt {
        machine_id: id,
        owner_id: owner,
        epoch: epoch as u64,
        revoked,
    })
}

// Also compiled in Unix tests to exercise SQLite bootstrap ordering locally.
#[cfg(any(windows, test))]
fn configure_private_journal(db: &Connection) -> rusqlite::Result<()> {
    // Preparing journal_mode may read the schema and recover a hot journal before
    // PERSIST takes effect. Temporary exclusive mode retains that journal during
    // recovery; restore NORMAL before any authority transaction to keep independent
    // connections usable. locking_mode itself does not read the schema.
    db.execute_batch(
        "PRAGMA locking_mode=EXCLUSIVE; PRAGMA journal_mode=PERSIST;
         PRAGMA locking_mode=NORMAL; PRAGMA temp_store=MEMORY;",
    )
}

#[cfg(test)]
mod tests;
