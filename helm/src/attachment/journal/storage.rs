//! Native Windows storage; no frontend or transport authority is granted here.
use anyhow::Result;
use rusqlite::Connection;
use std::{io::ErrorKind, path::PathBuf, sync::Arc};
use voyage_storage::PrivateDirectory;

pub(crate) fn prepare(directory: PathBuf) -> Result<(PathBuf, Arc<PrivateDirectory>)> {
    let private = Arc::new(PrivateDirectory::open(&directory)?);
    // Preserve explicit user ownership even when the token's default owner is a
    // group. SQLite must never recreate a rollback journal with default security.
    drop(private.open_file("journal.sqlite3", true)?);
    drop(private.open_file("journal.sqlite3-journal", true)?);
    for name in ["journal.sqlite3-wal", "journal.sqlite3-shm"] {
        match private.open_file(name, false) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok((private.path().to_owned(), private))
}

pub(crate) fn configure(connection: &Connection) -> Result<()> {
    // journal_mode preparation can recover a hot journal before PERSIST takes
    // effect. Exclusive bootstrap retains it; NORMAL restores independent SQLite
    // clients before schema reads/migrations or authority transactions proceed.
    connection.execute_batch(
        "PRAGMA locking_mode=EXCLUSIVE; PRAGMA journal_mode=PERSIST;
         PRAGMA locking_mode=NORMAL; PRAGMA temp_store=MEMORY;",
    )?;
    Ok(())
}

pub(crate) fn verify(private: &PrivateDirectory) -> Result<()> {
    drop(private.open_file("journal.sqlite3", false)?);
    drop(private.open_file("journal.sqlite3-journal", false)?);
    Ok(())
}
