//! Private durable consent declarations. Snapshots, previews, hashes, and receipts
//! are inert metadata, not authentication or dispatch/publication authority.
use super::{Disclosure, SharingSettings};
use crate::attachment::{
    client::validate_origin,
    local_actor::{LocalActor, storage},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, path::Path};
use uuid::Uuid;

const VERSION: u32 = 1;
const MAX_RECORDS: usize = 4096;
const MAX_RECORD: usize = 16 * 1024;
const HEADER: &str = "consent-header.json";
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
type Result<T> = std::result::Result<T, ConsentError>;
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConsentError {
    #[error("invalid local consent request")]
    Invalid,
    #[error("local consent revision or identity conflict")]
    Conflict,
    #[error("local consent publication pending; resume exact operation")]
    Pending,
    #[error("local consent evidence is invalid")]
    Evidence,
    #[error("local consent capacity exhausted")]
    Capacity,
    #[error("local consent storage is busy")]
    Busy,
    #[error("private local consent storage unavailable")]
    Storage,
}
fn storage_error(error: anyhow::Error) -> ConsentError {
    if error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|error| error.kind() == std::io::ErrorKind::WouldBlock)
    {
        ConsentError::Busy
    } else {
        ConsentError::Storage
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentDestination {
    pub origin: String,
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
}
impl ConsentDestination {
    /// Caller derives this scope from verified enrollment, not request claims.
    pub fn new(
        origin: &str,
        machine_id: Uuid,
        owner_id: Uuid,
        epoch: u64,
        allow_loopback_http: bool,
    ) -> Result<Self> {
        let value = Self {
            origin: validate_origin(origin, allow_loopback_http)
                .map_err(|_| ConsentError::Invalid)?,
            machine_id,
            owner_id,
            epoch,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        if self.machine_id.is_nil()
            || self.owner_id.is_nil()
            || !(1..i64::MAX as u64).contains(&self.epoch)
            || validate_origin(&self.origin, true).ok().as_deref() != Some(self.origin.as_str())
        {
            return Err(ConsentError::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentChange {
    pub operation_id: Uuid,
    pub actor: LocalActor,
    pub session_id: Uuid,
    pub expected_revision: u64,
    pub destination: Option<ConsentDestination>,
    pub settings: SharingSettings,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentSnapshot {
    pub session_id: Uuid,
    pub revision: u64,
    pub destination: Option<ConsentDestination>,
    pub settings: SharingSettings,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConsentPreview {
    pub operation_id: Uuid,
    pub previous: Option<ConsentSnapshot>,
    pub proposed: ConsentSnapshot,
    pub confirmation_digest: String,
    pub already_committed: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConsentReceipt {
    pub operation_id: Uuid,
    pub sequence: u64,
    pub snapshot: ConsentSnapshot,
    pub confirmation_digest: String,
    pub duplicate: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConsentPage {
    pub sessions: Vec<ConsentSnapshot>,
    pub next_after: Option<Uuid>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConsentAuditPage {
    pub entries: Vec<ConsentAuditEntry>,
    pub next_after: Option<u64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ConsentAuditEntry {
    pub sequence: u64,
    pub operation_id: Uuid,
    pub actor: LocalActor,
    pub previous: Option<ConsentSnapshot>,
    pub proposed: ConsentSnapshot,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    actor: LocalActor,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    sequence: u64,
    previous_hash: String,
    change: ConsentChange,
    previous: Option<ConsentSnapshot>,
    proposed: ConsentSnapshot,
    confirmation_digest: String,
}
impl Record {
    fn receipt(&self, duplicate: bool) -> ConsentReceipt {
        ConsentReceipt {
            operation_id: self.change.operation_id,
            sequence: self.sequence,
            snapshot: self.proposed.clone(),
            confirmation_digest: self.confirmation_digest.clone(),
            duplicate,
        }
    }
    fn preview(&self, already_committed: bool) -> ConsentPreview {
        ConsentPreview {
            operation_id: self.change.operation_id,
            previous: self.previous.clone(),
            proposed: self.proposed.clone(),
            confirmation_digest: self.confirmation_digest.clone(),
            already_committed,
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
fn witness_name(sequence: usize) -> String {
    format!("witness-{sequence:04}.json")
}
fn witness_bytes(sequence: u64, record: &[u8]) -> Result<Vec<u8>> {
    encoded(&Witness {
        version: VERSION,
        sequence,
        record_hash: digest(record),
    })
}
struct Loaded {
    records: Vec<Record>,
    sessions: BTreeMap<Uuid, ConsentSnapshot>,
    hash: String,
    pending: Option<Record>,
}
/// Each operation takes a bounded, nonblocking private storage lock and rereads
/// durable declarations. Resolve actor/enrollment before this lock; never hold it
/// over network/provider/Journal operations. No authorization adapter is supplied.
pub struct ConsentStore {
    directory: storage::Directory,
    actor: LocalActor,
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
fn valid_actor(actor: LocalActor) -> bool {
    !actor.installation_id.is_nil()
        && !actor.principal_id.is_nil()
        && actor.installation_id != actor.principal_id
}
fn encoded<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value).map_err(|_| ConsentError::Evidence)?;
    if bytes.len() > MAX_RECORD {
        return Err(ConsentError::Capacity);
    }
    Ok(bytes)
}
fn record_name(sequence: usize) -> String {
    format!("consent-{sequence:04}.json")
}
fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
impl ConsentStore {
    pub fn open(directory: &Path, actor: LocalActor) -> Result<Self> {
        if !valid_actor(actor) {
            return Err(ConsentError::Invalid);
        }
        let directory = storage::Directory::open(directory).map_err(storage_error)?;
        let lock = directory.lock().map_err(storage_error)?;
        let expected = encoded(&Header {
            version: VERSION,
            actor,
        })?;
        match directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
        {
            Some(bytes) => {
                if bytes != expected {
                    return Err(ConsentError::Conflict);
                }
            }
            None => {
                if directory
                    .read_bounded(&record_name(1), MAX_RECORD)
                    .map_err(storage_error)?
                    .is_some()
                    || directory
                        .read_bounded("actor.json", MAX_RECORD)
                        .map_err(storage_error)?
                        .is_some()
                {
                    return Err(ConsentError::Evidence);
                }
                directory
                    .publish_new(HEADER, &expected)
                    .map_err(storage_error)?;
            }
        }
        directory.sync_file(HEADER).map_err(storage_error)?;
        directory.sync().map_err(storage_error)?;
        directory.verify().map_err(storage_error)?;
        drop(lock);
        Ok(Self { directory, actor })
    }
    fn validate_change(&self, change: &ConsentChange) -> Result<()> {
        if change.actor != self.actor
            || !valid_actor(change.actor)
            || change.operation_id.is_nil()
            || change.session_id.is_nil()
            || change.expected_revision >= i64::MAX as u64
        {
            return Err(ConsentError::Invalid);
        }
        if change.settings.disclosure == Disclosure::None {
            if change.destination.is_some()
                || change.settings.approve_write
                || change.settings.approve_command
            {
                return Err(ConsentError::Invalid);
            }
        } else {
            change
                .destination
                .as_ref()
                .ok_or(ConsentError::Invalid)?
                .validate()?;
        }
        Ok(())
    }
    fn proposal(
        &self,
        change: &ConsentChange,
        previous: Option<&ConsentSnapshot>,
    ) -> Result<(ConsentSnapshot, String)> {
        self.validate_change(change)?;
        if change.expected_revision != previous.map_or(0, |snapshot| snapshot.revision) {
            return Err(ConsentError::Conflict);
        }
        let proposed = ConsentSnapshot {
            session_id: change.session_id,
            revision: change.expected_revision + 1,
            destination: change.destination.clone(),
            settings: change.settings,
        };
        let confirmation = digest(&encoded(&(change, previous, &proposed))?);
        Ok((proposed, confirmation))
    }
    fn check_record(&self, record: &Record, state: &Loaded) -> Result<()> {
        if record.version != VERSION
            || record.sequence != state.records.len() as u64 + 1
            || record.previous_hash != state.hash
            || state
                .records
                .iter()
                .any(|prior| prior.change.operation_id == record.change.operation_id)
            || record.previous.as_ref() != state.sessions.get(&record.change.session_id)
        {
            return Err(ConsentError::Evidence);
        }
        let (proposed, confirmation) = self
            .proposal(&record.change, record.previous.as_ref())
            .map_err(|_| ConsentError::Evidence)?;
        if record.proposed != proposed || record.confirmation_digest != confirmation {
            return Err(ConsentError::Evidence);
        }
        Ok(())
    }
    fn load(&self) -> Result<Loaded> {
        if self
            .directory
            .read_bounded(HEADER, MAX_RECORD)
            .map_err(storage_error)?
            != Some(encoded(&Header {
                version: VERSION,
                actor: self.actor,
            })?)
        {
            return Err(ConsentError::Evidence);
        }
        let mut state = Loaded {
            records: Vec::new(),
            sessions: BTreeMap::new(),
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
                    return Err(ConsentError::Evidence);
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
                    return Err(ConsentError::Evidence);
                }
                let record: Record =
                    serde_json::from_slice(&bytes).map_err(|_| ConsentError::Evidence)?;
                if encoded(&record)? != bytes {
                    return Err(ConsentError::Evidence);
                }
                self.check_record(&record, &state)?;
                state.hash = digest(&bytes);
                state
                    .sessions
                    .insert(record.change.session_id, record.proposed.clone());
                state.records.push(record);
            } else {
                gap = true;
                if let Some(witness) = witness {
                    if missing_record_witness
                        .replace((sequence, witness))
                        .is_some()
                    {
                        return Err(ConsentError::Evidence);
                    }
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
                return Err(ConsentError::Evidence);
            }
            let record: Record =
                serde_json::from_slice(&bytes).map_err(|_| ConsentError::Evidence)?;
            if encoded(&record)? != bytes {
                return Err(ConsentError::Evidence);
            }
            self.check_record(&record, &state)?;
            if let Some((witness_sequence, witness)) = missing_record_witness {
                if witness_sequence != next || witness != witness_bytes(next as u64, &bytes)? {
                    return Err(ConsentError::Evidence);
                }
            }
            state.pending = Some(record);
        } else if missing_record_witness.is_some() {
            return Err(ConsentError::Evidence);
        }
        self.directory.verify().map_err(storage_error)?;
        Ok(state)
    }
    pub fn preview_local(&self, change: &ConsentChange) -> Result<ConsentPreview> {
        let _lock = self.directory.lock().map_err(storage_error)?;
        self.validate_change(change)?;
        let state = self.load()?;
        if let Some(record) = state
            .records
            .iter()
            .find(|record| record.change.operation_id == change.operation_id)
        {
            if record.change != *change {
                return Err(ConsentError::Conflict);
            }
            return Ok(record.preview(true));
        }
        if let Some(record) = state.pending {
            if record.change != *change {
                return Err(ConsentError::Pending);
            }
            return Ok(record.preview(false));
        }
        if state.records.len() >= MAX_RECORDS {
            return Err(ConsentError::Capacity);
        }
        let previous = state.sessions.get(&change.session_id).cloned();
        let (proposed, confirmation_digest) = self.proposal(change, previous.as_ref())?;
        Ok(ConsentPreview {
            operation_id: change.operation_id,
            previous,
            proposed,
            confirmation_digest,
            already_committed: false,
        })
    }
    pub fn commit_local(
        &self,
        change: &ConsentChange,
        confirmation_digest: &str,
    ) -> Result<ConsentReceipt> {
        self.commit_with(change, confirmation_digest, |_| Ok(()))
    }
    fn commit_with(
        &self,
        change: &ConsentChange,
        confirmation: &str,
        mut boundary: impl FnMut(Boundary) -> Result<()>,
    ) -> Result<ConsentReceipt> {
        let _lock = self.directory.lock().map_err(storage_error)?;
        self.validate_change(change)?;
        let state = self.load()?;
        if let Some(record) = state
            .records
            .iter()
            .find(|record| record.change.operation_id == change.operation_id)
        {
            if record.change != *change || record.confirmation_digest != confirmation {
                return Err(ConsentError::Conflict);
            }
            self.directory
                .sync_file(&record_name(record.sequence as usize))
                .map_err(storage_error)?;
            self.directory.sync().map_err(storage_error)?;
            self.directory.verify().map_err(storage_error)?;
            return Ok(record.receipt(true));
        }
        if state.records.len() >= MAX_RECORDS {
            return Err(ConsentError::Capacity);
        }
        let previous = state.sessions.get(&change.session_id).cloned();
        let (proposed, expected) = self.proposal(change, previous.as_ref())?;
        if confirmation != expected {
            return Err(ConsentError::Conflict);
        }
        let record = Record {
            version: VERSION,
            sequence: state.records.len() as u64 + 1,
            previous_hash: state.hash,
            change: change.clone(),
            previous,
            proposed,
            confirmation_digest: expected,
        };
        let bytes = encoded(&record)?;
        if let Some(pending) = state.pending {
            if encoded(&pending)? != bytes {
                return Err(ConsentError::Pending);
            }
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
            file.write_all(&bytes).map_err(|_| ConsentError::Storage)?;
            boundary(Boundary::CandidateWritten)?;
            file.sync_all().map_err(|_| ConsentError::Storage)?;
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
                    return Err(ConsentError::Evidence);
                }
            }
            None => {
                let mut file = self
                    .directory
                    .create(&witness_name)
                    .map_err(storage_error)?;
                file.write_all(&witness)
                    .map_err(|_| ConsentError::Storage)?;
                file.sync_all().map_err(|_| ConsentError::Storage)?;
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
            return Err(ConsentError::Evidence);
        }
        self.directory.verify().map_err(storage_error)?;
        boundary(Boundary::Verified)?;
        Ok(record.receipt(false))
    }
    pub fn inspect_local(&self, session_id: Uuid) -> Result<Option<ConsentSnapshot>> {
        let _lock = self.directory.lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(ConsentError::Pending);
        }
        Ok(state.sessions.get(&session_id).cloned())
    }
    pub fn list_local(&self, after: Option<Uuid>, limit: usize) -> Result<ConsentPage> {
        if !(1..=100).contains(&limit) {
            return Err(ConsentError::Invalid);
        }
        let _lock = self.directory.lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(ConsentError::Pending);
        }
        let mut sessions: Vec<_> = state
            .sessions
            .into_values()
            .filter(|snapshot| after.is_none_or(|id| snapshot.session_id > id))
            .take(limit + 1)
            .collect();
        let more = sessions.len() > limit;
        sessions.truncate(limit);
        let next_after = more.then(|| sessions.last().expect("nonempty page").session_id);
        Ok(ConsentPage {
            sessions,
            next_after,
        })
    }
    pub fn audit_local(&self, after: u64, limit: usize) -> Result<ConsentAuditPage> {
        if !(1..=100).contains(&limit) {
            return Err(ConsentError::Invalid);
        }
        let _lock = self.directory.lock().map_err(storage_error)?;
        let state = self.load()?;
        if state.pending.is_some() {
            return Err(ConsentError::Pending);
        }
        let mut entries: Vec<_> = state
            .records
            .into_iter()
            .filter(|record| record.sequence > after)
            .take(limit + 1)
            .map(|record| ConsentAuditEntry {
                sequence: record.sequence,
                operation_id: record.change.operation_id,
                actor: record.change.actor,
                previous: record.previous,
                proposed: record.proposed,
            })
            .collect();
        let more = entries.len() > limit;
        entries.truncate(limit);
        let next_after = more.then(|| entries.last().expect("nonempty page").sequence);
        Ok(ConsentAuditPage {
            entries,
            next_after,
        })
    }
}
#[cfg(test)]
mod tests;
