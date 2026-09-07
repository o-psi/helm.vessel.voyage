use anyhow::{Context, Result, bail};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::{Component, Path},
};

pub fn check_path(path: &Path, uid: u32) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        bail!("Installation paths must be absolute without dot components");
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                if meta.file_type().is_symlink()
                    || (meta.uid() != uid && meta.uid() != 0)
                    || meta.mode() & 0o022 != 0
                {
                    bail!(
                        "Unsafe ownership, permissions or symlink at {}",
                        ancestor.display()
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

pub fn directory(path: &Path, uid: u32, private: bool) -> Result<()> {
    check_path(path, uid)?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            directory(parent, uid, false)?;
        }
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || (private && (meta.uid() != uid || meta.mode() & 0o077 != 0)) {
        bail!("Expected a private owned directory at {}", path.display());
    }
    Ok(())
}

pub fn executable(path: &Path, uid: u32) -> Result<()> {
    check_path(path, uid)?;
    let meta = fs::symlink_metadata(path)
        .with_context(|| format!("Missing release binary {}", path.display()))?;
    if !meta.is_file() || meta.mode() & 0o111 == 0 || meta.mode() & 0o6000 != 0 {
        bail!("Expected an ordinary executable at {}", path.display());
    }
    Ok(())
}

/// Atomically exchange reviewed content; displaced files remain recoverable backups.
pub(super) fn replace(path: &Path, content: &str, expected: Option<&str>) -> Result<()> {
    let parent = path.parent().context("Unit path has no parent")?;
    let temporary = temporary(parent, "backup")?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    output.write_all(content.as_bytes())?;
    output.sync_all()?;
    if let Some(expected) = expected {
        rename(&temporary, path, libc::RENAME_EXCHANGE)?;
        let displaced = read(&temporary);
        if displaced.as_deref().ok() != Some(expected) {
            // Restore the displaced user edit. Keep the other inode as evidence,
            // including any edit made concurrently after our initial exchange.
            rename(&temporary, path, libc::RENAME_EXCHANGE)?;
            File::open(parent)?.sync_all()?;
            bail!(
                "Service unit changed concurrently; restored its contents and retained {}",
                temporary.display()
            );
        }
    } else if let Err(error) = rename(&temporary, path, libc::RENAME_NOREPLACE) {
        let _ = fs::remove_file(&temporary);
        return Err(error.context("Service unit appeared concurrently; refusing replacement"));
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}
pub(super) fn remove_reviewed(path: &Path, expected: &str) -> Result<()> {
    let parent = path.parent().context("Unit path has no parent")?;
    let backup = temporary(parent, "removed")?;
    rename(path, &backup, libc::RENAME_NOREPLACE)?;
    if read(&backup)?.as_str() != expected {
        let restored = rename(&backup, path, libc::RENAME_NOREPLACE);
        bail!(
            "Service unit changed concurrently; preserved {} (restoration: {})",
            backup.display(),
            restored.is_ok()
        );
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn temporary(parent: &Path, kind: &str) -> Result<std::path::PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    Ok(parent.join(format!(
        ".voyage-unit-{kind}-{}-{stamp}",
        std::process::id()
    )))
}
fn read(path: &Path) -> Result<String> {
    use std::io::Read;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > 16384 {
        bail!("Unsafe displaced service unit")
    }
    let mut value = String::new();
    file.take(16385).read_to_string(&mut value)?;
    Ok(value)
}
fn rename(from: &Path, to: &Path, flags: u32) -> Result<()> {
    let from = std::ffi::CString::new(from.as_os_str().as_encoded_bytes())?;
    let to = std::ffi::CString::new(to.as_os_str().as_encoded_bytes())?;
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            flags,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
