use super::{
    files,
    layout::{Journal, Layout},
    release::BINARIES,
};
use anyhow::{Result, ensure};
use std::{
    fs,
    os::unix::fs::{MetadataExt, symlink},
};
pub(super) fn validate(l: &Layout, j: &Journal, replace: bool) -> Result<()> {
    ensure!(
        l.pointer()? == j.current || l.pointer()? == j.pending,
        "Current release changed outside installer"
    );
    files::safe(&l.bin)?;
    for name in BINARIES {
        let p = l.bin.join(name);
        match fs::symlink_metadata(&p) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    ensure!(
                        fs::read_link(&p)? == l.root.join("current/bin").join(name),
                        "Unrecognized command link {}",
                        p.display()
                    );
                } else {
                    ensure!(
                        replace && m.is_file(),
                        "Existing command {} requires explicit replacement (regular files only)",
                        p.display()
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
pub(super) fn publish_links(l: &Layout, replace: bool) -> Result<()> {
    files::directory(&l.bin)?;
    for name in BINARIES {
        let p = l.bin.join(name);
        let target = l.root.join("current/bin").join(name);
        if let Ok(m) = fs::symlink_metadata(&p) {
            if m.file_type().is_symlink() {
                ensure!(fs::read_link(&p)? == target, "Command link changed");
                continue;
            }
            ensure!(replace && m.is_file(), "Command changed before publication");
            files::directory(&l.root.join("backups"))?;
            let digest = files::hash(&p)?;
            let backup = l.root.join("backups").join(format!("{name}-{digest}"));
            if !backup.exists() {
                fs::hard_link(&p, &backup)?;
                files::sync(&backup)?;
            }
            ensure!(
                files::hash(&backup)? == digest && files::hash(&p)? == digest,
                "Existing helm changed during backup"
            );
            let replacement = l.bin.join(format!(".voyage-{name}-replacement"));
            if fs::symlink_metadata(&replacement).is_ok() {
                ensure!(
                    fs::read_link(&replacement)? == target,
                    "Unrecognized pending Helm replacement; preserved for inspection"
                );
            } else {
                symlink(&target, &replacement)?;
            }
            files::exchange(&replacement, &p)?;
            let displaced = fs::symlink_metadata(&replacement)?;
            if displaced.dev() != m.dev()
                || displaced.ino() != m.ino()
                || displaced.mode() != m.mode()
                || files::hash(&replacement)? != digest
            {
                anyhow::bail!(
                    "Concurrent Helm change preserved at {}; installation paused",
                    replacement.display()
                );
            }
            fs::remove_file(&replacement)?;
            continue;
        }
        symlink(&target, &p)?;
        files::sync(&p)?;
    }
    Ok(())
}
