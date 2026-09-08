use super::super::registry;
use anyhow::{Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use uuid::Uuid;
use voyage_protocol::process::*;

pub(crate) fn now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}
pub(super) fn directory(root: &Path) -> PathBuf {
    root.join("access")
}
pub(crate) fn grant_path(root: &Path, id: Uuid) -> PathBuf {
    directory(root).join("grants").join(format!("{id}.json"))
}
pub(super) fn initialize(root: &Path) -> Result<()> {
    registry::private_directory(&directory(root))?;
    registry::private_directory(&directory(root).join("grants"))?;
    registry::private_directory(&directory(root).join("credentials"))
}
pub(crate) fn load<T: DeserializeOwned>(path: &Path) -> Result<T> {
    load_bounded(path, 16384)
}
pub(crate) fn load_bounded<T: DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= limit,
        "invalid private access record"
    );
    Ok(serde_json::from_reader(file)?)
}
pub(crate) fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    save_bounded(path, value, 16384)
}
pub(crate) fn save_bounded<T: Serialize>(path: &Path, value: &T, limit: usize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing access directory"))?;
    let tmp = parent.join(format!(".pending-{}", Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&tmp)?;
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= limit, "access record too large");
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub(crate) fn hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
pub(super) fn authenticate(root: &Path, id: Uuid, token: &str) -> Result<ProcessGrant> {
    ensure!(token.len() == 64, "access denied");
    let grant: ProcessGrant = load(&grant_path(root, id))?;
    let actual = hash(token);
    ensure!(
        actual.len() == grant.token_hash.len()
            && actual
                .as_bytes()
                .iter()
                .zip(grant.token_hash.as_bytes())
                .fold(0u8, |a, (x, y)| a | (x ^ y))
                == 0,
        "access denied"
    );
    current(&grant)?;
    Ok(grant)
}
pub(super) fn current(grant: &ProcessGrant) -> Result<()> {
    ensure!(!grant.revoked, "access revoked");
    ensure!(grant.expires_at_ms > now()?, "access expired");
    if let Some(identity) = &grant.enrollment {
        enrollment(identity)?;
    }
    Ok(())
}
pub(super) fn enrollment(identity: &EnrollmentIdentity) -> Result<()> {
    ensure!(
        identity.database_path.is_absolute() && !identity.machine_id.is_nil() && identity.epoch > 0,
        "invalid enrollment binding"
    );
    let metadata = std::fs::symlink_metadata(&identity.database_path)?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe enrollment database"
    );
    let db = rusqlite::Connection::open_with_flags(
        &identity.database_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(std::time::Duration::from_millis(100))?;
    let (epoch, revoked): (i64, bool) = db.query_row(
        "SELECT epoch,revoked FROM machines WHERE id=?1",
        [identity.machine_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    ensure!(
        !revoked && u64::try_from(epoch)? == identity.epoch,
        "enrolled machine revoked or replaced"
    );
    Ok(())
}
pub(super) fn credential_path(root: &Path, id: Uuid) -> PathBuf {
    directory(root)
        .join("credentials")
        .join(format!("{id}.json"))
}

pub(crate) fn connection_path(root: &Path, id: Uuid) -> PathBuf {
    directory(root)
        .join("connections")
        .join(format!("{id}.json"))
}

pub(super) fn authenticate_connection(
    root: &Path,
    id: Uuid,
    token: &str,
) -> Result<ConnectionGrant> {
    use subtle::ConstantTimeEq;
    ensure!(token.len() == 64, "access denied");
    let grant: ConnectionGrant = load(&connection_path(root, id))?;
    ensure!(
        bool::from(hash(token).as_bytes().ct_eq(grant.token_hash.as_bytes())),
        "access denied"
    );
    current_connection(root, &grant)?;
    ensure!(grant.grant_id == id, "connection identity mismatch");
    Ok(grant)
}

pub(crate) fn current_connection(root: &Path, grant: &ConnectionGrant) -> Result<()> {
    ensure!(
        grant.schema_version == 1
            && !grant.grant_id.is_nil()
            && !grant.principal_id.is_nil()
            && grant.revision > 0,
        "invalid connection authority"
    );
    ensure!(!grant.revoked, "access revoked");
    ensure!(grant.expires_at_ms > now()?, "access expired");
    ensure!(
        grant.vessel_id == super::super::identity::public(root)?.vessel_id,
        "Vessel identity changed"
    );
    ensure!(
        !grant.workspaces.is_empty() && grant.workspaces.len() <= 32,
        "invalid workspace scope"
    );
    let latest: ConnectionGrant = load(&connection_path(root, grant.grant_id))?;
    ensure!(!latest.revoked, "access revoked");
    ensure!(latest.expires_at_ms > now()?, "access expired");
    ensure!(
        latest.schema_version == 1
            && latest.grant_id == grant.grant_id
            && latest.principal_id == grant.principal_id
            && latest.vessel_id == grant.vessel_id
            && latest.revision == grant.revision
            && !latest.revoked
            && latest.expires_at_ms == grant.expires_at_ms
            && latest.rights == grant.rights
            && latest.workspaces == grant.workspaces,
        "connection authority changed"
    );
    Ok(())
}
