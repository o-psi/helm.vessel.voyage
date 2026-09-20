//! Cross-process browser capacity. Reservations survive uncertain cleanup/crashes.
//! A dropped handle does not prove that browser descendants stopped.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug)]
pub struct Capacity {
    path: PathBuf,
    file: File,
    owner: Uuid,
}
impl Capacity {
    pub fn acquire(root: &Path, owner: Uuid, slots: usize) -> Result<Self> {
        ensure!(
            !owner.is_nil() && (1..=64).contains(&slots),
            "invalid browser capacity configuration"
        );
        let private = crate::attachment::local_actor::storage::Directory::open(root)?;
        private.verify()?;
        for index in 0..slots {
            let path = root.join(format!("browser-slot-{index}"));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            }
            match options.open(&path) {
                Ok(mut file) => {
                    // A failed write retains the slot: uncertain ownership is not free capacity.
                    file.write_all(owner.to_string().as_bytes())?;
                    file.sync_all()?;
                    #[cfg(unix)]
                    File::open(root)?.sync_all()?;
                    return Ok(Self { path, file, owner });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
        anyhow::bail!("browser capacity is in use or awaiting observed cleanup")
    }
    /// Only call after worker + browser cleanup has been positively observed.
    pub fn release(self) -> Result<()> {
        let current = fs::symlink_metadata(&self.path)?;
        let held = self.file.metadata()?;
        ensure!(
            current.is_file() && !current.file_type().is_symlink(),
            "browser capacity reservation changed"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                current.dev() == held.dev()
                    && current.ino() == held.ino()
                    && current.uid() == unsafe { libc::geteuid() }
                    && current.mode() & 0o077 == 0,
                "browser capacity reservation replaced"
            );
        }
        ensure!(
            fs::read(&self.path).context("browser capacity reservation unavailable")?
                == self.owner.to_string().as_bytes(),
            "browser capacity owner changed"
        );
        fs::remove_file(&self.path)?;
        #[cfg(unix)]
        File::open(self.path.parent().unwrap())?.sync_all()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reservations_are_exclusive_and_drop_never_attests_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("capacity");
        let first = Capacity::acquire(&root, Uuid::new_v4(), 1).unwrap();
        assert!(Capacity::acquire(&root, Uuid::new_v4(), 1).is_err());
        first.release().unwrap();
        let second = Capacity::acquire(&root, Uuid::new_v4(), 1).unwrap();
        drop(second);
        assert!(Capacity::acquire(&root, Uuid::new_v4(), 1).is_err());
    }
    #[test]
    fn changed_reservation_is_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("capacity");
        let slot = Capacity::acquire(&root, Uuid::new_v4(), 1).unwrap();
        fs::write(&slot.path, b"replacement").unwrap();
        assert!(slot.release().is_err());
        assert_eq!(
            fs::read(root.join("browser-slot-0")).unwrap(),
            b"replacement"
        );
    }
}
