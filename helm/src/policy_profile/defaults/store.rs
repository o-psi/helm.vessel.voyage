//! Private defaults administration log. Historical rules support transition comparison, never current authority.
use crate::attachment::local_actor::storage;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;
const MAX_RECORDS: usize = 1024;
const MAX_RECORD: usize = 65_536;
const HEADER: &str = "defaults-header.json";
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
type Result<T> = std::result::Result<T, StoreError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("invalid policy defaults request")]
    Invalid,
    #[error("policy defaults revision or operation conflict")]
    Conflict,
    #[error("policy defaults publication pending; retry exact operation")]
    Pending,
    #[error("policy defaults evidence is invalid")]
    Evidence,
    #[error("policy defaults history capacity exhausted")]
    Capacity,
    #[error("policy defaults storage is busy")]
    Busy,
    #[error("private policy defaults storage unavailable")]
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
#[serde(deny_unknown_fields)]
pub struct DefaultsSource {
    pub directory: PathBuf,
    pub store_id: Uuid,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRef {
    pub directory: PathBuf,
    pub name: String,
    pub revision: u64,
    pub digest: String,
    /// Immutable reviewed bytes for comparing future fallback; active selection still rereads its source.
    pub snapshot: crate::policy_profile::store::ProfileSnapshot,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefaultScope {
    Global {},
    Workspace { workspace: PathBuf },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefaultKey {
    Preference { scope: DefaultScope },
    Activation { workspace: PathBuf },
}
impl DefaultKey {
    pub fn digest(&self) -> Result<String> {
        Ok(digest(&encoded(self)?))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DefaultValue {
    Preference {
        profile: Option<ProfileRef>,
    },
    Activation {
        candidate_digest: String,
        transition_digest: String,
        context_digest: String,
        effective: crate::policy_profile::Rules,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultsChange {
    pub operation_id: Uuid,
    pub key: DefaultKey,
    pub expected_revision: u64,
    pub value: DefaultValue,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DefaultsSnapshot {
    pub key: DefaultKey,
    pub revision: u64,
    pub value: DefaultValue,
}
impl DefaultsSnapshot {
    pub fn digest(&self) -> Result<String> {
        Ok(digest(&encoded(self)?))
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    kind: String,
    store_id: Uuid,
}
#[derive(Clone, Debug, Serialize)]
pub struct DefaultsReceipt {
    pub operation_id: Uuid,
    pub sequence: u64,
    pub snapshot: DefaultsSnapshot,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct DefaultsPage {
    pub profiles: Vec<DefaultsSnapshot>,
    pub next_after: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    sequence: u64,
    previous_hash: String,
    change: DefaultsChange,
    snapshot: DefaultsSnapshot,
}
impl Record {
    fn receipt(&self, duplicate: bool) -> DefaultsReceipt {
        DefaultsReceipt {
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
    profiles: BTreeMap<String, DefaultsSnapshot>,
    hash: String,
    pending: Option<Record>,
}
/// Each operation holds only a nonblocking private filesystem lock. Never retain
/// it across provider, Journal, approval, or network work. Reads do not recover a
/// pending publication into effective selection; only the exact change can do so.
pub struct DefaultsStore {
    directory: storage::Directory,
    header: Vec<u8>,
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
    format!("defaults-{sequence:04}.json")
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
fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|s| s.len() <= 4096 && !s.chars().any(char::is_control))
        && !path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
}
// These are normalized trusted Config rules, not an imported profile. Keep
// legacy environment names/counts valid; the enclosing encoded-record bound
// still limits storage. Never interpret relative or placeholder roots here.
fn valid_effective(rules: &super::super::Rules) -> bool {
    rules
        .read_roots
        .iter()
        .chain(&rules.write_roots)
        .all(|root| valid_path(Path::new(root)))
        && rules
            .deny_commands
            .iter()
            .chain(&rules.inherit_env)
            .all(|value| !value.contains('\0'))
}
fn validate_change(change: &DefaultsChange) -> Result<()> {
    if change.operation_id.is_nil() || change.expected_revision >= i64::MAX as u64 {
        return Err(StoreError::Invalid);
    }
    match &change.key {
        DefaultKey::Preference {
            scope: DefaultScope::Workspace { workspace },
        }
        | DefaultKey::Activation { workspace }
            if !valid_path(workspace) =>
        {
            return Err(StoreError::Invalid);
        }
        _ => (),
    }
    match (&change.key, &change.value) {
        (DefaultKey::Preference { .. }, DefaultValue::Preference { profile }) => {
            if let Some(p) = profile
                && (!valid_path(&p.directory)
                    || !super::super::label(&p.name)
                    || p.revision == 0
                    || p.revision > i64::MAX as u64
                    || !valid_hash(&p.digest)
                    || p.snapshot.name != p.name
                    || p.snapshot.revision != p.revision
                    || p.snapshot.identity.is_nil()
                    || p.snapshot
                        .rules
                        .as_ref()
                        .is_none_or(|rules| rules.validate().is_err())
                    || p.snapshot.digest().ok().as_ref() != Some(&p.digest))
            {
                return Err(StoreError::Invalid);
            }
        }
        (
            DefaultKey::Activation { .. },
            DefaultValue::Activation {
                candidate_digest,
                transition_digest,
                context_digest,
                effective,
            },
        ) if valid_hash(candidate_digest)
            && valid_hash(transition_digest)
            && valid_hash(context_digest)
            && valid_effective(effective) => {}
        _ => return Err(StoreError::Invalid),
    }
    if encoded(change)?.len() > 30 * 1024 {
        return Err(StoreError::Capacity);
    }
    Ok(())
}
fn proposal(
    change: &DefaultsChange,
    previous: Option<&DefaultsSnapshot>,
) -> Result<DefaultsSnapshot> {
    validate_change(change)?;
    if change.expected_revision != previous.map_or(0, |p| p.revision) {
        return Err(StoreError::Conflict);
    }
    Ok(DefaultsSnapshot {
        key: change.key.clone(),
        revision: change.expected_revision + 1,
        value: change.value.clone(),
    })
}
impl DefaultsStore {
    fn header(anchor: &DefaultsSource) -> Result<Vec<u8>> {
        if !valid_path(&anchor.directory) || anchor.store_id.is_nil() {
            return Err(StoreError::Invalid);
        }
        encoded(&Header {
            version: 1,
            kind: "helm-policy-defaults".into(),
            store_id: anchor.store_id,
        })
    }

    pub fn create(anchor: &DefaultsSource) -> Result<Self> {
        let header = Self::header(anchor)?;
        let path = &anchor.directory;
        let directory = storage::Directory::open(path).map_err(storage_error)?;
        let lock = directory.lock().map_err(storage_error)?;
        for foreign in ["profile-header.json", "consent-header.json", "actor.json"] {
            if directory
                .read_bounded(foreign, MAX_RECORD)
                .map_err(storage_error)?
                .is_some()
            {
                return Err(StoreError::Evidence);
            }
        }

        match directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
        {
            Some(bytes) if bytes == header => (),
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
                    .publish_new(HEADER, &header)
                    .map_err(storage_error)?;
            }
        }
        directory.sync_file(HEADER).map_err(storage_error)?;
        directory.sync().map_err(storage_error)?;
        directory.verify().map_err(storage_error)?;
        drop(lock);
        Ok(Self { directory, header })
    }
    /// Selection freshness never bootstraps missing state or publishes pending data.
    pub fn open_existing(anchor: &DefaultsSource) -> Result<Self> {
        let header = Self::header(anchor)?;
        let path = &anchor.directory;
        let directory = storage::Directory::open_existing(path).map_err(storage_error)?;
        let lock = directory.read_lock().map_err(storage_error)?;
        if directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
            .as_deref()
            != Some(header.as_slice())
        {
            return Err(StoreError::Evidence);
        }
        directory.verify().map_err(storage_error)?;
        drop(lock);
        Ok(Self { directory, header })
    }
    fn check_record(&self, record: &Record, state: &Loaded) -> Result<()> {
        if record.version != 1
            || record.sequence != state.records.len() as u64 + 1
            || record.previous_hash != state.hash
            || state
                .records
                .iter()
                .any(|r| r.change.operation_id == record.change.operation_id)
            || proposal(
                &record.change,
                state.profiles.get(&record.change.key.digest()?),
            )
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
            != Some(self.header.as_slice())
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
                    .insert(record.change.key.digest()?, record.snapshot.clone());
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
    pub fn change(&self, change: &DefaultsChange) -> Result<DefaultsReceipt> {
        self.commit_with(change, |_| Ok(()))
    }
    fn commit_with(
        &self,
        change: &DefaultsChange,
        mut boundary: impl FnMut(Boundary) -> Result<()>,
    ) -> Result<DefaultsReceipt> {
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
        let snapshot = proposal(change, state.profiles.get(&change.key.digest()?))?;
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
    pub(super) fn operation(&self, id: Uuid) -> Result<Option<DefaultsReceipt>> {
        let _lock = self.directory.read_lock().map_err(storage_error)?;
        let state = self.load()?;
        Ok(state
            .records
            .iter()
            .find(|r| r.change.operation_id == id)
            .map(|r| r.receipt(true)))
    }
    /// One shared-lock history observation; mutations never imply adoption.
    pub(super) fn history(&self) -> Result<Vec<(u64, DefaultsSnapshot)>> {
        let _lock = self.directory.read_lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(StoreError::Pending);
        }
        Ok(state
            .records
            .into_iter()
            .map(|r| (r.sequence, r.snapshot))
            .collect())
    }
    pub fn inspect(&self, key: &DefaultKey) -> Result<Option<DefaultsSnapshot>> {
        let _lock = self.directory.read_lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(StoreError::Pending);
        }
        Ok(state.profiles.get(&key.digest()?).cloned())
    }
    pub fn list(&self, after: Option<&str>, limit: usize) -> Result<DefaultsPage> {
        if !(1..=100).contains(&limit) || after.is_some_and(|s| !valid_hash(s)) {
            return Err(StoreError::Invalid);
        }
        let _lock = self.directory.read_lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(StoreError::Pending);
        }
        let mut profiles: Vec<_> = state
            .profiles
            .into_iter()
            .filter(|(k, _)| after.is_none_or(|a| k.as_str() > a))
            .take(limit + 1)
            .collect();
        let more = profiles.len() > limit;
        profiles.truncate(limit);
        let next_after = more.then(|| profiles.last().unwrap().0.clone());
        Ok(DefaultsPage {
            profiles: profiles.into_iter().map(|(_, v)| v).collect(),
            next_after,
        })
    }
}
