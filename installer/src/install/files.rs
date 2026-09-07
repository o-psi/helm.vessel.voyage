use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
};

pub fn safe(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute()
            && !path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Installation requires absolute normalized paths"
    );
    let uid = unsafe { libc::geteuid() };
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) => ensure!(
                !m.file_type().is_symlink()
                    && (m.uid() == uid || m.uid() == 0)
                    && (m.mode() & 0o022 == 0
                        || (m.uid() == 0 && m.is_dir() && m.mode() & 0o1000 != 0)),
                "Unsafe installation path {}",
                p.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub fn directory(path: &Path) -> Result<()> {
    safe(path)?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            directory(parent)?;
        }
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    ensure!(fs::metadata(path)?.is_dir(), "Expected directory");
    Ok(())
}
pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let m = f.metadata()?;
    ensure!(
        m.is_file() && m.len() <= limit,
        "Invalid or oversized installation file"
    );
    let mut bytes = Vec::new();
    f.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "File grew during read");
    Ok(bytes)
}
pub fn hash(path: &Path) -> Result<String> {
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(f.metadata()?.is_file(), "Expected regular binary");
    let mut h = Sha256::new();
    let mut b = [0u8; 65536];
    loop {
        let n = f.read(&mut b)?;
        if n == 0 {
            break;
        }
        h.update(&b[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    sync(path)
}
pub fn sync(path: &Path) -> Result<()> {
    File::open(path.parent().context("No parent")?)?.sync_all()?;
    Ok(())
}
pub fn atomic_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let temp = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    write_new(&temp, &serde_json::to_vec_pretty(value)?)?;
    fs::rename(&temp, path)?;
    sync(path)
}
pub fn lock(path: &Path) -> Result<File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(
        unsafe {
            libc::flock(
                std::os::fd::AsRawFd::as_raw_fd(&f),
                libc::LOCK_EX | libc::LOCK_NB,
            )
        } == 0,
        "Another installer owns the installation lock"
    );
    Ok(f)
}

pub fn exchange(left: &Path, right: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let a = CString::new(left.as_os_str().as_bytes())?;
    let b = CString::new(right.as_os_str().as_bytes())?;
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            a.as_ptr(),
            libc::AT_FDCWD,
            b.as_ptr(),
            libc::RENAME_EXCHANGE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    sync(right)
}

pub fn private_directory(path: &Path) -> Result<()> {
    directory(path)?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0,
        "Installation directory must be private and owned: {}",
        path.display()
    );
    Ok(())
}
