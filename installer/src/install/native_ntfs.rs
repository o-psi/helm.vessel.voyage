//! NTFS receipt persistence. Retained handles pin ACL-checked ancestors.
//! No UID/SID translation, elevation, command shim replacement or SCM effects.
use super::native_receipt::Receipt;
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};
use voyage_storage::PrivateDirectory;

pub struct Store {
    directory: PrivateDirectory,
    _lock: std::fs::File,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let directory = PrivateDirectory::open(path).context(
            "Native install root must be local NTFS with exact owner ACLs and no reparse ancestors",
        )?;
        let lock = directory.lock("operation.lock")?;
        Ok(Self {
            directory,
            _lock: lock,
        })
    }
    pub fn receipt(&self) -> Result<Option<Receipt>> {
        let file = match self.directory.open_file("operation.json", false) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        ensure!(
            file.metadata()?.len() <= 64 * 1024,
            "Oversized native operation receipt"
        );
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 64 * 1024, "Native receipt grew during read");
        let receipt: Receipt = serde_json::from_slice(&bytes)?;
        receipt.validate()?;
        Ok(Some(receipt))
    }
    pub fn begin(&self, receipt: &Receipt) -> Result<()> {
        receipt.validate()?;
        ensure!(
            receipt.phase == super::native_receipt::Phase::Reviewed,
            "Initial native receipt must be reviewed"
        );
        ensure!(
            self.receipt()?.is_none(),
            "Retained native receipt requires observed recovery, not replay"
        );
        self.directory
            .publish_new("operation.json", &serde_json::to_vec(receipt)?)?;
        Ok(())
    }
    pub fn advance(&self, expected: &Receipt, next: &Receipt) -> Result<()> {
        ensure!(
            self.receipt()?.as_ref() == Some(expected),
            "Native receipt changed before transition"
        );
        let mut permitted = expected.clone();
        permitted.transition(next.phase.clone())?;
        ensure!(
            &permitted == next,
            "Native transition changed pinned identity/release/service facts"
        );
        // Publication errors may follow the rename. Caller retains operation ID
        // and reopens/observes this exact receipt; never retries under a new ID.
        self.directory
            .publish("operation.json", &serde_json::to_vec(next)?)?;
        Ok(())
    }
}
