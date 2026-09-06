//! Bounded private metadata intents. Canonical sessions are never rewritten here.
use crate::attachment::local_actor::storage;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;
use voyage_protocol::control::{Address, ClientOperation, Machine};
const FILE: &str = "coordination-intents.json";
const LIMIT: usize = 65_536;
const CAPACITY: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Intent {
    pub origin: String,
    pub owner_id: Uuid,
    pub machine: Machine,
    pub operation: ClientOperation,
}
impl Intent {
    pub fn address(&self) -> Address {
        let ClientOperation::Register { address, .. } = self.operation else {
            unreachable!()
        };
        address
    }
    pub fn id(&self) -> Uuid {
        let ClientOperation::Register { command_id, .. } = self.operation else {
            unreachable!()
        };
        command_id
    }
    pub fn observe(&self) -> ClientOperation {
        let ClientOperation::Register {
            command_id,
            expires_at_ms,
            address,
            ..
        } = self.operation
        else {
            unreachable!()
        };
        ClientOperation::ObserveRegistration {
            command_id,
            expires_at_ms,
            address,
        }
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            self.origin.len() <= 2048 && !self.owner_id.is_nil(),
            "invalid control intent"
        );
        ensure!(
            matches!(self.operation, ClientOperation::Register { .. }),
            "invalid intent operation"
        );
        self.machine.validate().map_err(anyhow::Error::msg)?;
        self.operation.validate().map_err(anyhow::Error::msg)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    entries: Vec<Intent>,
}
pub(super) struct Store {
    directory: storage::Directory,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            directory: storage::Directory::open_existing(path)?,
        })
    }
    fn read(&self) -> Result<(Journal, Option<Vec<u8>>)> {
        self.directory.verify()?;
        let bytes = self.directory.read_bounded(FILE, LIMIT)?;
        let journal = match &bytes {
            Some(bytes) => serde_json::from_slice::<Journal>(bytes)?,
            None => Journal {
                version: 1,
                entries: Vec::new(),
            },
        };
        ensure!(
            journal.version == 1 && journal.entries.len() <= CAPACITY,
            "unsupported control intents"
        );
        for (index, entry) in journal.entries.iter().enumerate() {
            entry.validate()?;
            ensure!(
                !journal.entries[..index]
                    .iter()
                    .any(|old| old.id() == entry.id()),
                "duplicate control intent"
            );
        }
        Ok((journal, bytes))
    }
    pub fn latest(&self, address: Address) -> Result<Option<Intent>> {
        let _lock = self.directory.lock()?;
        Ok(self
            .read()?
            .0
            .entries
            .into_iter()
            .rev()
            .find(|entry| entry.address() == address))
    }
    /// Append only after the caller checked the selected session and any prior
    /// expired intent's authenticated non-admission. Preserve every original.
    pub fn append(&self, expected: Option<&Intent>, entry: &Intent) -> Result<()> {
        entry.validate()?;
        let _lock = self.directory.lock()?;
        let (mut journal, before) = self.read()?;
        let latest = journal
            .entries
            .iter()
            .rev()
            .find(|old| old.address() == entry.address());
        if latest == Some(entry) {
            self.directory.sync_file(FILE)?;
            self.directory.sync()?;
            self.directory.verify()?;
            return Ok(());
        }
        ensure!(latest == expected, "control intent changed");
        ensure!(
            journal.entries.len() < CAPACITY
                && !journal.entries.iter().any(|old| old.id() == entry.id()),
            "control intent capacity or identity conflict"
        );
        journal.entries.push(entry.clone());
        let bytes = serde_json::to_vec(&journal)?;
        ensure!(bytes.len() <= LIMIT, "control intent capacity");
        ensure!(
            self.directory.read_bounded(FILE, LIMIT)? == before,
            "control intent changed"
        );
        self.directory.publish(FILE, &bytes)?;
        self.directory.sync()?;
        ensure!(
            self.directory.read_bounded(FILE, LIMIT)?.as_deref() == Some(bytes.as_slice()),
            "uncertain control intent publication"
        );
        self.directory.verify()
    }
}
