//! Bounded OAuth records using the existing native private-directory boundary.
use crate::attachment::local_actor::storage::Directory;
use anyhow::{Context, Result, ensure};
use std::{io, path::Path};

pub(super) const LIMIT: usize = 65_536;

fn parent_and_name(path: &Path) -> Result<(&Path, &str)> {
    ensure!(path.is_absolute(), "credential path must be absolute");
    let parent = path.parent().context("credential path needs a parent")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("invalid credential filename")?;
    ensure!(
        !matches!(name, "actor.lock" | "publication.json"),
        "credential filename is reserved"
    );
    Ok((parent, name))
}

fn existing(path: &Path) -> Result<Option<Directory>> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(Directory::open_existing(path)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

// Existing ordinary ancestors may be traversable; only new directories and the
// cache's immediate parent must be private. Directory pins and verifies the full
// path without following symlinks before opening or publishing any credential.
fn create_missing_ancestors(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(metadata.is_dir(), "credential ancestor is not a directory");
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_missing_ancestors(path.parent().context("credential ancestor missing")?)?;
            Directory::open(path)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

pub(super) fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    let path = std::path::absolute(path)?;
    let (parent, name) = parent_and_name(&path)?;
    let Some(directory) = existing(parent)? else {
        return Ok(None);
    };
    directory.read_bounded(name, LIMIT)
}

pub(super) fn write(path: &Path, bytes: Option<&[u8]>) -> Result<()> {
    let path = std::path::absolute(path)?;
    let (parent, name) = parent_and_name(&path)?;
    let directory = if bytes.is_some() {
        create_missing_ancestors(parent.parent().context("credential parent missing")?)?;
        Directory::open(parent)?
    } else {
        let Some(directory) = existing(parent)? else {
            return Ok(());
        };
        directory
    };
    let _lock = directory.lock()?;
    let previous = directory.read_bounded(name, LIMIT)?;
    if bytes.is_none() && previous.is_none() {
        return Ok(());
    }
    // A null record is a durable logout tombstone, readable by load as None.
    // Reuse atomic private replacement instead of path-based unlink operations.
    directory.publish(name, bytes.unwrap_or(b"null"))
}
