//! Private inert named-profile revisions. A receipt is history, never authority.
use super::{Builtin, ProfileDocument, Rules};
use crate::attachment::local_actor::storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, path::Path};
use uuid::Uuid;
const MAX_RECORDS: usize = 1024;
const MAX_RECORD: usize = 65_536;
const HEADER: &str = "profile-header.json";
const HEADER_BYTES: &[u8] = b"{\"version\":1,\"kind\":\"helm-policy-profiles\"}";
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
type Result<T> = std::result::Result<T, StoreError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("invalid policy profile request")]
    Invalid,
    #[error("policy profile revision or operation conflict")]
    Conflict,
    #[error("policy profile publication pending; retry exact operation")]
    Pending,
    #[error("policy profile evidence is invalid")]
    Evidence,
    #[error("policy profile history capacity exhausted")]
    Capacity,
    #[error("policy profile storage is busy")]
    Busy,
    #[error("private policy profile storage unavailable")]
    Storage,
}
fn storage_error(error: anyhow::Error) -> StoreError {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::WouldBlock)
    {
        StoreError::Busy
    } else {
        StoreError::Storage
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Create { rules: Rules },
    Replace { rules: Rules },
    Delete {},
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileChange {
    pub operation_id: Uuid,
    pub name: String,
    pub expected_revision: u64,
    pub action: Action,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSnapshot {
    pub name: String,
    /// A recreated name has a different incarnation; revisions never reset.
    pub identity: Uuid,
    pub revision: u64,
    pub rules: Option<Rules>,
    pub builtin: bool,
}
impl ProfileSnapshot {
    pub fn digest(&self) -> Result<String> {
        Ok(digest(&encoded(self)?))
    }
    pub fn document(&self) -> Result<ProfileDocument> {
        Ok(ProfileDocument {
            schema: 1,
            name: self.name.clone(),
            revision: self.revision,
            rules: self.rules.clone().ok_or(StoreError::Conflict)?,
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ProfileReceipt {
    pub operation_id: Uuid,
    pub sequence: u64,
    pub snapshot: ProfileSnapshot,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProfilePage {
    pub profiles: Vec<ProfileSnapshot>,
    pub next_after: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    sequence: u64,
    previous_hash: String,
    change: ProfileChange,
    snapshot: ProfileSnapshot,
}
impl Record {
    fn receipt(&self, duplicate: bool) -> ProfileReceipt {
        ProfileReceipt {
            operation_id: self.change.operation_id,
            sequence: self.sequence,
            snapshot: self.snapshot.clone(),
            duplicate,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Witness {
    version: u32,
    sequence: u64,
    record_hash: String,
}
struct Loaded {
    records: Vec<Record>,
    profiles: BTreeMap<String, ProfileSnapshot>,
    hash: String,
    pending: Option<Record>,
}
/// Each operation holds only a nonblocking private filesystem lock. Never retain
/// it across provider, Journal, approval, or network work. Reads do not recover a
/// pending publication into effective selection; only the exact change can do so.
pub struct ProfileStore {
    directory: storage::Directory,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    CandidateCreated,
    CandidateWritten,
    CandidateDurable,
    BeforePublish,
    Published,
    DirectorySynced,
    Verified,
}
fn encoded(value: &impl Serialize) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| StoreError::Evidence)?;
    if bytes.len() > MAX_RECORD {
        return Err(StoreError::Capacity);
    }
    Ok(bytes)
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
fn record_name(sequence: usize) -> String {
    format!("profile-{sequence:04}.json")
}
fn witness_name(sequence: usize) -> String {
    format!("witness-{sequence:04}.json")
}
fn witness_bytes(sequence: u64, bytes: &[u8]) -> Result<Vec<u8>> {
    encoded(&Witness {
        version: 1,
        sequence,
        record_hash: digest(bytes),
    })
}
fn builtin(name: &str) -> Option<ProfileSnapshot> {
    let (preset, id) = match name {
        "restricted" => (Builtin::Restricted, 1),
        "balanced" => (Builtin::Balanced, 2),
        "autonomous" => (Builtin::Autonomous, 3),
        _ => return None,
    };
    let doc = preset.document();
    Some(ProfileSnapshot {
        name: doc.name,
        identity: Uuid::from_u128(id),
        revision: 1,
        rules: Some(doc.rules),
        builtin: true,
    })
}
fn validate_change(change: &ProfileChange) -> Result<()> {
    if change.operation_id.is_nil()
        || !super::label(&change.name)
        || builtin(&change.name).is_some()
        || change.expected_revision >= i64::MAX as u64
    {
        return Err(StoreError::Invalid);
    }
    if let Action::Create { rules } | Action::Replace { rules } = &change.action {
        rules.validate().map_err(|_| StoreError::Invalid)?;
    }
    // Leave sufficient room for the immutable receipt envelope.
    if encoded(change)?.len() > 30 * 1024 {
        return Err(StoreError::Capacity);
    }
    Ok(())
}
fn proposal(change: &ProfileChange, previous: Option<&ProfileSnapshot>) -> Result<ProfileSnapshot> {
    validate_change(change)?;
    if change.expected_revision != previous.map_or(0, |p| p.revision) {
        return Err(StoreError::Conflict);
    }
    let (identity, rules) = match &change.action {
        Action::Create { rules } if previous.is_none_or(|p| p.rules.is_none()) => {
            (change.operation_id, Some(rules.clone()))
        }
        Action::Replace { rules } if previous.is_some_and(|p| p.rules.is_some()) => {
            (previous.unwrap().identity, Some(rules.clone()))
        }
        Action::Delete {} if previous.is_some_and(|p| p.rules.is_some()) => {
            (previous.unwrap().identity, None)
        }
        _ => return Err(StoreError::Conflict),
    };
    Ok(ProfileSnapshot {
        name: change.name.clone(),
        identity,
        revision: change.expected_revision + 1,
        rules,
        builtin: false,
    })
}
impl ProfileStore {
    pub fn open(path: &Path) -> Result<Self> {
        let directory = storage::Directory::open(path).map_err(storage_error)?;
        let lock = directory.lock().map_err(storage_error)?;
        match directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
        {
            Some(bytes) if bytes == HEADER_BYTES => (),
            Some(_) => return Err(StoreError::Evidence),
            None => {
                if directory
                    .read_bounded(&record_name(1), MAX_RECORD)
                    .map_err(storage_error)?
                    .is_some()
                {
                    return Err(StoreError::Evidence);
                }
                directory
                    .publish_new(HEADER, HEADER_BYTES)
                    .map_err(storage_error)?;
            }
        }
        directory.sync_file(HEADER).map_err(storage_error)?;
        directory.sync().map_err(storage_error)?;
        directory.verify().map_err(storage_error)?;
        drop(lock);
        Ok(Self { directory })
    }
    /// Selection freshness never bootstraps missing state or publishes pending data.
    pub fn open_existing(path: &Path) -> Result<Self> {
        let directory = storage::Directory::open_existing(path).map_err(storage_error)?;
        let lock = directory.read_lock().map_err(storage_error)?;
        if directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
            .as_deref()
            != Some(HEADER_BYTES)
        {
            return Err(StoreError::Evidence);
        }
        directory.verify().map_err(storage_error)?;
        drop(lock);
        Ok(Self { directory })
    }
    fn check_record(&self, record: &Record, state: &Loaded) -> Result<()> {
        if record.version != 1
            || record.sequence != state.records.len() as u64 + 1
            || record.previous_hash != state.hash
            || state
                .records
                .iter()
                .any(|r| r.change.operation_id == record.change.operation_id)
            || proposal(&record.change, state.profiles.get(&record.change.name))
                .map_err(|_| StoreError::Evidence)?
                != record.snapshot
        {
            return Err(StoreError::Evidence);
        }
        Ok(())
    }
    fn load(&self) -> Result<Loaded> {
        if self
            .directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
            .as_deref()
            != Some(HEADER_BYTES)
        {
            return Err(StoreError::Evidence);
        }
        let mut state = Loaded {
            records: Vec::new(),
            profiles: BTreeMap::new(),
            hash: ZERO_HASH.into(),
            pending: None,
        };
        let mut gap = false;
        let mut missing_record_witness: Option<(usize, Vec<u8>)> = None;
        #[cfg(windows)]
        let mut staged: Option<(usize, Vec<u8>)> = None;
        for sequence in 1..=MAX_RECORDS {
            let name = record_name(sequence);
            let witness = self
                .directory
                .read_bounded(&witness_name(sequence), MAX_RECORD)
                .map_err(storage_error)?;
            #[cfg(windows)]
            if let Some(bytes) = self
                .directory
                .read_bounded(
                    &storage::publication_name(&name).map_err(storage_error)?,
                    MAX_RECORD,
                )
                .map_err(storage_error)?
            {
                if staged.replace((sequence, bytes)).is_some() {
                    return Err(StoreError::Evidence);
                }
            }
            if let Some(bytes) = self
                .directory
                .read_bounded(&name, MAX_RECORD)
                .map_err(storage_error)?
            {
                if gap
                    || witness.as_deref()
                        != Some(witness_bytes(sequence as u64, &bytes)?.as_slice())
                {
                    return Err(StoreError::Evidence);
                }
                let record: Record =
                    serde_json::from_slice(&bytes).map_err(|_| StoreError::Evidence)?;
                if encoded(&record)? != bytes {
                    return Err(StoreError::Evidence);
                }
                self.check_record(&record, &state)?;
                state.hash = digest(&bytes);
                state
                    .profiles
                    .insert(record.change.name.clone(), record.snapshot.clone());
                state.records.push(record);
            } else {
                gap = true;
                if let Some(witness) = witness
                    && missing_record_witness
                        .replace((sequence, witness))
                        .is_some()
                {
                    return Err(StoreError::Evidence);
                }
            }
        }
        let next = state.records.len() + 1;
        #[cfg(unix)]
        let staged = self
            .directory
            .read_bounded(
                &storage::publication_name(&record_name(next)).map_err(storage_error)?,
                MAX_RECORD,
            )
            .map_err(storage_error)?
            .map(|bytes| (next, bytes));
        if let Some((sequence, bytes)) = staged {
            if sequence != next || next > MAX_RECORDS {
                return Err(StoreError::Evidence);
            }
            let record: Record =
                serde_json::from_slice(&bytes).map_err(|_| StoreError::Evidence)?;
            if encoded(&record)? != bytes {
                return Err(StoreError::Evidence);
            }
            self.check_record(&record, &state)?;
            if let Some((witness_sequence, witness)) = missing_record_witness
                && (witness_sequence != next || witness != witness_bytes(next as u64, &bytes)?)
            {
                return Err(StoreError::Evidence);
            }
            state.pending = Some(record);
        } else if missing_record_witness.is_some() {
            return Err(StoreError::Evidence);
        }
        self.directory.verify().map_err(storage_error)?;
        Ok(state)
    }
    pub fn change(&self, change: &ProfileChange) -> Result<ProfileReceipt> {
        self.commit_with(change, |_| Ok(()))
    }
    fn commit_with(
        &self,
        change: &ProfileChange,
        mut boundary: impl FnMut(Boundary) -> Result<()>,
    ) -> Result<ProfileReceipt> {
        let _lock = self.directory.lock().map_err(storage_error)?;
        validate_change(change)?;
        let state = self.load()?;
        if let Some(record) = state
            .records
            .iter()
            .find(|record| record.change.operation_id == change.operation_id)
        {
            if record.change != *change {
                return Err(StoreError::Conflict);
            }
            self.directory
                .sync_file(&record_name(record.sequence as usize))
                .map_err(storage_error)?;
            self.directory.sync().map_err(storage_error)?;
            self.directory.verify().map_err(storage_error)?;
            return Ok(record.receipt(true));
        }
        if state.records.len() >= MAX_RECORDS {
            return Err(StoreError::Capacity);
        }
        let snapshot = proposal(change, state.profiles.get(&change.name))?;
        let record = Record {
            version: 1,
            sequence: state.records.len() as u64 + 1,
            previous_hash: state.hash,
            change: change.clone(),
            snapshot,
        };
        let bytes = encoded(&record)?;
        if let Some(pending) = state.pending
            && encoded(&pending)? != bytes
        {
            return Err(StoreError::Pending);
        }
        let name = record_name(record.sequence as usize);
        let candidate = storage::publication_name(&name).map_err(storage_error)?;
        if self
            .directory
            .read_bounded(&candidate, MAX_RECORD)
            .map_err(storage_error)?
            .is_none()
        {
            let mut file = self.directory.create(&candidate).map_err(storage_error)?;
            boundary(Boundary::CandidateCreated)?;
            file.write_all(&bytes).map_err(|_| StoreError::Storage)?;
            boundary(Boundary::CandidateWritten)?;
            file.sync_all().map_err(|_| StoreError::Storage)?;
        }
        self.directory
            .sync_file(&candidate)
            .map_err(storage_error)?;
        self.directory.sync().map_err(storage_error)?;
        boundary(Boundary::CandidateDurable)?;
        let witness_name = witness_name(record.sequence as usize);
        let witness = witness_bytes(record.sequence, &bytes)?;
        match self
            .directory
            .read_bounded(&witness_name, MAX_RECORD)
            .map_err(storage_error)?
        {
            Some(existing) => {
                if existing != witness {
                    return Err(StoreError::Evidence);
                }
            }
            None => {
                let mut file = self
                    .directory
                    .create(&witness_name)
                    .map_err(storage_error)?;
                file.write_all(&witness).map_err(|_| StoreError::Storage)?;
                file.sync_all().map_err(|_| StoreError::Storage)?;
            }
        }
        self.directory
            .sync_file(&witness_name)
            .map_err(storage_error)?;
        self.directory.sync().map_err(storage_error)?;
        boundary(Boundary::BeforePublish)?;
        self.directory
            .publish_new(&name, &bytes)
            .map_err(storage_error)?;
        boundary(Boundary::Published)?;
        self.directory.sync().map_err(storage_error)?;
        boundary(Boundary::DirectorySynced)?;
        if self
            .directory
            .read_bounded(&name, MAX_RECORD)
            .map_err(storage_error)?
            .as_deref()
            != Some(bytes.as_slice())
        {
            return Err(StoreError::Evidence);
        }
        self.directory.verify().map_err(storage_error)?;
        boundary(Boundary::Verified)?;
        Ok(record.receipt(false))
    }
    pub fn inspect(&self, name: &str) -> Result<Option<ProfileSnapshot>> {
        if !super::label(name) {
            return Err(StoreError::Invalid);
        }
        let _lock = self.directory.read_lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(StoreError::Pending);
        }
        Ok(builtin(name).or_else(|| state.profiles.get(name).cloned()))
    }
    pub fn list(&self, after: Option<&str>, limit: usize) -> Result<ProfilePage> {
        if !(1..=100).contains(&limit) || after.is_some_and(|s| !super::label(s)) {
            return Err(StoreError::Invalid);
        }
        let _lock = self.directory.read_lock().map_err(storage_error)?;
        let mut state = self.load()?;
        if state.pending.is_some() {
            return Err(StoreError::Pending);
        }
        for name in ["restricted", "balanced", "autonomous"] {
            state.profiles.insert(name.into(), builtin(name).unwrap());
        }
        let mut profiles: Vec<_> = state
            .profiles
            .into_values()
            .filter(|p| after.is_none_or(|a| p.name.as_str() > a))
            .take(limit + 1)
            .collect();
        let more = profiles.len() > limit;
        profiles.truncate(limit);
        let next_after = more.then(|| profiles.last().unwrap().name.clone());
        Ok(ProfilePage {
            profiles,
            next_after,
        })
    }
    pub fn export(&self, name: &str) -> Result<Vec<u8>> {
        self.inspect(name)?
            .ok_or(StoreError::Conflict)?
            .document()?
            .encode()
            .map_err(|_| StoreError::Invalid)
    }
}
