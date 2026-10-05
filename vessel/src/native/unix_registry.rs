//! Private ordinary Unix registry. No root/bound identity adoption.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};
use voyage_protocol::process::ProcessRegistration;

pub fn directory(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "Absolute private registry required");
    let uid = unsafe { libc::geteuid() };
    ensure!(
        uid != 0 && uid == unsafe { libc::getuid() },
        "Ordinary native registry refuses administrator identity"
    );
    for ancestor in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(ancestor) {
            ensure!(
                !meta.file_type().is_symlink()
                    && (meta.uid() == uid || meta.uid() == 0)
                    && meta.mode() & 0o022 == 0,
                "Unsafe registry ancestor"
            );
        }
    }
    if !path.exists() {
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0,
        "Native registry not private owned directory"
    );
    Ok(())
}
pub fn lock(path: &Path) -> Result<File> {
    directory(path)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path.join("native-supervisor.lock"))?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "Unsafe native lock"
    );
    ensure!(
        unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&file),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } == 0,
        "Another native supervisor owns registry"
    );
    Ok(file)
}
pub fn publish(path: &Path, registration: &ProcessRegistration) -> Result<()> {
    directory(path)?;
    ensure!(
        registration.peer_uids.is_none() && registration.token.len() >= 32,
        "Native registry only accepts ordinary private runtime binding"
    );
    let bytes = serde_json::to_vec(registration)?;
    ensure!(bytes.len() <= 16384, "Native registration exceeds bound");
    let temporary = path.join(format!("registration.{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path.join("registration.json"))?;
    File::open(path)?.sync_all()?;
    Ok(())
}
pub fn load(path: &Path) -> Result<ProcessRegistration> {
    directory(path)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.join("registration.json"))?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.nlink() == 1
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= 16384,
        "Unsafe native registration"
    );
    let mut bytes = Vec::new();
    file.take(16385).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16384, "Native registration grew during read");
    let registration: ProcessRegistration =
        serde_json::from_slice(&bytes).context("Invalid native registration")?;
    ensure!(
        registration.peer_uids.is_none() && registration.token.len() >= 32,
        "Invalid native ordinary binding"
    );
    Ok(registration)
}
