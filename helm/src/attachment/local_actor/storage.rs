use super::MAX_BYTES;
use anyhow::{Context, Result, ensure};
use std::{fs::File, io::Read, path::Path};

pub(crate) struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        // Explicit release also releases the shared OS lock description if a
        // concurrent fork briefly inherited it before close-on-exec.
        let _ = self.0.unlock();
    }
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::Directory;

#[cfg(windows)]
pub(crate) struct Directory(voyage_storage::PrivateDirectory);
#[cfg(windows)]
impl Directory {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "local actor directory must be absolute");
        Ok(Self(voyage_storage::PrivateDirectory::open(path)?))
    }
    pub(crate) fn open_existing(path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "local actor directory must be absolute");
        Ok(Self(voyage_storage::PrivateDirectory::open_existing(path)?))
    }
    pub(crate) fn read_lock(&self) -> Result<Lock> {
        let file = self.0.open_file("actor.lock", false)?;
        file.try_lock_shared().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => {
                std::io::Error::from(std::io::ErrorKind::WouldBlock)
            }
            std::fs::TryLockError::Error(error) => error,
        })?;
        Ok(Lock(file))
    }
    pub(crate) fn verify(&self) -> Result<()> {
        // Retained native directory handles deny rename/delete of the directory
        // and its ancestors; each child open separately verifies owner-only ACLs.
        Ok(())
    }
    pub(crate) fn lock(&self) -> Result<Lock> {
        Ok(Lock(self.0.lock("actor.lock")?))
    }
    pub(crate) fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read_bounded(name, MAX_BYTES)
    }
    pub(crate) fn read_bounded(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        validate_name(name)?;
        validate_limit(limit)?;
        read_bounded(self.0.open_file(name, false), limit)
    }
    pub(crate) fn create(&self, name: &str) -> Result<File> {
        validate_name(name)?;
        Ok(self.0.create_file(name)?)
    }
    pub(crate) fn sync_file(&self, name: &str) -> Result<()> {
        validate_name(name)?;
        self.0.sync_file(name)?;
        Ok(())
    }
    pub(crate) fn sync(&self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn publish_new(&self, name: &str, bytes: &[u8]) -> Result<()> {
        validate_name(name)?;
        validate_limit(bytes.len())?;
        Ok(self.0.publish_new(name, bytes)?)
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("local actor storage requires a supported private filesystem implementation");

const MAX_PRIVATE_BYTES: usize = 65_536;
fn validate_limit(limit: usize) -> Result<()> {
    ensure!(
        (1..=MAX_PRIVATE_BYTES).contains(&limit),
        "invalid private record bound"
    );
    Ok(())
}
fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.len() <= 128
            && name != "."
            && name != ".."
            && !name.ends_with('.')
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-')),
        "invalid private record name"
    );
    Ok(())
}
pub(crate) fn publication_name(name: &str) -> Result<String> {
    validate_name(name)?;
    #[cfg(windows)]
    let temporary = format!(".new-{name}");
    #[cfg(unix)]
    let temporary = "publication.json".to_owned();
    validate_name(&temporary)?;
    Ok(temporary)
}
fn read_bounded(file: std::io::Result<File>, limit: usize) -> Result<Option<Vec<u8>>> {
    validate_limit(limit)?;
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .context("cannot read local actor record")?;
    ensure!(bytes.len() <= limit, "local actor record exceeds limit");
    Ok(Some(bytes))
}
