use super::super::registry;
use anyhow::{Result, ensure};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
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
    let bytes = read_bounded(path, limit)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub(crate) fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
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
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "access record too large");
    Ok(bytes)
}
pub(crate) fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    save_bounded(path, value, 16384)
}
pub(crate) fn save_bounded<T: Serialize>(path: &Path, value: &T, limit: usize) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    save_bytes(path, &bytes, limit)
}
pub(crate) fn save_bytes(path: &Path, bytes: &[u8], limit: usize) -> Result<()> {
    ensure!(bytes.len() <= limit, "access record too large");
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
    file.write_all(bytes)?;
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
pub(crate) fn current(grant: &ProcessGrant) -> Result<()> {
    ensure!(!grant.revoked, "access revoked");
    ensure!(grant.expires_at_ms > now()?, "access expired");
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
            && latest.accounts == grant.accounts
            && latest.enrollment_connections == grant.enrollment_connections
            && latest.workspaces == grant.workspaces,
        "connection authority changed"
    );
    Ok(())
}
