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

pub fn copy_executable(source: &Path, destination: &Path) -> Result<()> {
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(destination)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

pub fn write_new(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().context("Unit path has no parent")?;
    let temporary = parent.join(format!(".voyage-unit-{}", std::process::id()));
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        output.write_all(content.as_bytes())?;
        output.sync_all()?;
        // Publish a complete unit without replacing concurrent installation.
        fs::hard_link(&temporary, path)?;
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let cleanup = fs::remove_file(&temporary);
    result?;
    cleanup?;
    Ok(())
}
