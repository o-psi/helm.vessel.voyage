use super::MAX_BYTES;
use anyhow::{Context, Result, ensure};
use std::{fs::File, io::Read, path::Path};

pub(super) struct Lock(File);
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
pub(super) use unix::Directory;

#[cfg(windows)]
pub(super) struct Directory(voyage_storage::PrivateDirectory);
#[cfg(windows)]
impl Directory {
    pub(super) fn open(path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "local actor directory must be absolute");
        Ok(Self(voyage_storage::PrivateDirectory::open(path)?))
    }
    pub(super) fn verify(&self) -> Result<()> {
        // Retained native directory handles deny rename/delete of the directory
        // and its ancestors; each child open separately verifies owner-only ACLs.
        Ok(())
    }
    pub(super) fn lock(&self) -> Result<Lock> {
        Ok(Lock(self.0.lock("actor.lock")?))
    }
    pub(super) fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        read(self.0.open_file(name, false))
    }
    pub(super) fn create(&self, name: &str) -> Result<File> {
        Ok(self.0.create_file(name)?)
    }
    pub(super) fn sync_file(&self, name: &str) -> Result<()> {
        self.0.sync_file(name)?;
        Ok(())
    }
    pub(super) fn sync(&self) -> Result<()> {
        Ok(())
    }
    pub(super) fn publish_new(&self, name: &str, bytes: &[u8]) -> Result<()> {
        Ok(self.0.publish_new(name, bytes)?)
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("local actor storage requires a supported private filesystem implementation");

fn read(file: std::io::Result<File>) -> Result<Option<Vec<u8>>> {
    let file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("cannot read local actor record")?;
    ensure!(bytes.len() <= MAX_BYTES, "local actor record exceeds limit");
    Ok(Some(bytes))
}
