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

#[cfg(test)]
mod tests {
    use super::*;
    fn intent(address: Address) -> Intent {
        Intent {
            origin: "https://vessel.example".into(),
            owner_id: Uuid::new_v4(),
            machine: Machine {
                machine_id: Uuid::new_v4(),
                epoch: 1,
            },
            operation: ClientOperation::Register {
                command_id: Uuid::new_v4(),
                expires_at_ms: 12345,
                address,
                source_revision: 3,
            },
        }
    }
    #[test]
    fn private_intents_preserve_original_and_require_exact_predecessor() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("installation");
        let actor = crate::attachment::local_actor::LocalActorStore::open(&directory)
            .unwrap()
            .identity()
            .unwrap();
        let store = Store::open(&directory).unwrap();
        let address = Address {
            installation_id: actor.installation_id,
            session_id: Uuid::new_v4(),
        };
        let first = intent(address);
        store.append(None, &first).unwrap();
        let original = std::fs::read(directory.join(FILE)).unwrap();
        store.append(None, &first).unwrap();
        assert_eq!(std::fs::read(directory.join(FILE)).unwrap(), original);
        let next = intent(address);
        assert!(store.append(None, &next).is_err());
        assert_eq!(std::fs::read(directory.join(FILE)).unwrap(), original);
        store.append(Some(&first), &next).unwrap();
        assert_eq!(store.latest(address).unwrap(), Some(next));
        let parsed: Journal =
            serde_json::from_slice(&std::fs::read(directory.join(FILE)).unwrap()).unwrap();
        assert_eq!(parsed.entries[0], first);
        std::fs::write(directory.join(FILE), b"corrupt evidence").unwrap();
        assert!(store.latest(address).is_err());
        assert!(store.append(None, &intent(address)).is_err());
        assert_eq!(
            std::fs::read(directory.join(FILE)).unwrap(),
            b"corrupt evidence"
        );
    }
    #[cfg(unix)]
    #[test]
    fn symlink_intent_is_rejected_without_touching_target() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("installation");
        crate::attachment::local_actor::LocalActorStore::open(&directory).unwrap();
        let store = Store::open(&directory).unwrap();
        let target = temp.path().join("canary");
        std::fs::write(&target, b"private canary").unwrap();
        std::os::unix::fs::symlink(&target, directory.join(FILE)).unwrap();
        assert!(
            store
                .latest(Address {
                    installation_id: Uuid::new_v4(),
                    session_id: Uuid::new_v4()
                })
                .is_err()
        );
        assert_eq!(std::fs::read(target).unwrap(), b"private canary");
    }
}
