//! Descriptor-relative history publication on Unix. The trusted data directory's
//! native ACL applies on Windows, as for the existing completion ledger.
use anyhow::{Result, ensure};
#[cfg(not(unix))]
use std::fs;
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

#[cfg(unix)]
mod native {
    use super::*;
    use std::{
        ffi::{CString, OsStr},
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::ffi::OsStrExt,
        },
        path::Component,
    };
    pub struct Directory {
        file: File,
        pub name: CString,
    }
    fn string(name: &OsStr) -> Result<CString> {
        Ok(CString::new(name.as_bytes())?)
    }
    fn opened(fd: libc::c_int) -> std::io::Result<File> {
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: successful open/openat returns a newly owned descriptor.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    impl Directory {
        pub fn open(path: &Path, create: bool) -> Result<Self> {
            let path = if path.is_absolute() {
                path.to_owned()
            } else {
                std::env::current_dir()?.join(path)
            };
            let parent = path
                .parent()
                .ok_or_else(|| anyhow::anyhow!("invalid history path"))?;
            let mut file = File::open("/")?;
            for component in parent.components() {
                let Component::Normal(name) = component else {
                    ensure!(
                        matches!(component, Component::RootDir | Component::CurDir),
                        "invalid history directory traversal"
                    );
                    continue;
                };
                let name = string(name)?;
                let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
                // SAFETY: descriptor and NUL-terminated name remain live for call.
                let mut next =
                    opened(unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) });
                if create
                    && next
                        .as_ref()
                        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                {
                    // SAFETY: creates one private child beneath the held directory.
                    let result = unsafe { libc::mkdirat(file.as_raw_fd(), name.as_ptr(), 0o700) };
                    if result < 0
                        && std::io::Error::last_os_error().kind()
                            != std::io::ErrorKind::AlreadyExists
                    {
                        return Err(std::io::Error::last_os_error().into());
                    }
                    file.sync_all()?;
                    // SAFETY: same checked descriptor-relative open as above.
                    next = opened(unsafe { libc::openat(file.as_raw_fd(), name.as_ptr(), flags) });
                }
                file = next?;
            }
            Ok(Self {
                file,
                name: string(
                    path.file_name()
                        .ok_or_else(|| anyhow::anyhow!("missing history filename"))?,
                )?,
            })
        }
        pub fn open_file(&self, name: &CString, create: bool) -> std::io::Result<File> {
            let flags = libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if create {
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL
                } else {
                    libc::O_RDONLY
                };
            // SAFETY: held descriptor and valid name, newly owned file on success.
            opened(unsafe { libc::openat(self.file.as_raw_fd(), name.as_ptr(), flags, 0o600) })
        }
        pub fn regular(&self) -> Result<()> {
            match self.open_file(&self.name, false) {
                Ok(file) => ensure!(file.metadata()?.is_file(), "invalid history destination"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            Ok(())
        }
        pub fn replace(&self, name: &CString) -> Result<()> {
            self.regular()?;
            // SAFETY: both names are relative to the same held directory; rename
            // replaces a directory entry and cannot follow a destination symlink.
            if unsafe {
                libc::renameat(
                    self.file.as_raw_fd(),
                    name.as_ptr(),
                    self.file.as_raw_fd(),
                    self.name.as_ptr(),
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            self.file.sync_all()?;
            Ok(())
        }
        pub fn remove(&self, name: &CString) {
            // SAFETY: removes only the generated temporary name in this directory.
            unsafe {
                libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0);
            }
        }
    }
}

pub fn read(path: &Path, max: u64) -> Result<Option<Vec<u8>>> {
    #[cfg(unix)]
    let opened = match native::Directory::open(path, false) {
        Ok(directory) => directory.open_file(&directory.name, false),
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(e) => return Err(e),
    };
    #[cfg(not(unix))]
    let opened = {
        regular(path)?;
        File::open(path)
    };
    let file = match opened {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= max,
        "invalid history file size/type"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.nlink() == 1 && metadata.permissions().mode() & 0o077 == 0,
            "history file is not private"
        );
    }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= max, "history exceeds size limit");
    Ok(Some(bytes))
}

pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        let directory = native::Directory::open(path, true)?;
        directory.regular()?;
        let name = std::ffi::CString::new(format!(".events-{}.tmp", uuid::Uuid::new_v4()))?;
        let mut file = directory.open_file(&name, true)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            directory.replace(&name)
        })();
        if result.is_err() {
            directory.remove(&name);
        }
        result
    }
    #[cfg(not(unix))]
    {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid history path"))?;
        fs::create_dir_all(parent)?;
        ensure!(
            fs::symlink_metadata(parent)?.is_dir()
                && !fs::symlink_metadata(parent)?.file_type().is_symlink(),
            "invalid history directory"
        );
        regular(path)?;
        let temp = parent.join(format!(".events-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            regular(path)?;
            fs::rename(&temp, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }
}
#[cfg(not(unix))]
fn regular(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) => ensure!(
            m.is_file() && !m.file_type().is_symlink(),
            "invalid history file"
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    #[test]
    fn history_rejects_symlinked_ancestors_and_anchors_rename_to_open_directory() {
        let dir = tempfile::tempdir().unwrap();
        let actual = dir.path().join("actual");
        std::fs::create_dir(&actual).unwrap();
        let alias = dir.path().join("alias");
        symlink(&actual, &alias).unwrap();
        assert!(write(&alias.join("history"), b"secret").is_err());
        assert!(read(&alias.join("history"), 1024).is_err());
        assert!(std::fs::read_dir(&actual).unwrap().next().is_none());
        let held = native::Directory::open(&actual.join("history"), false).unwrap();
        let moved = dir.path().join("moved");
        std::fs::rename(&actual, &moved).unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, &actual).unwrap();
        let temp = std::ffi::CString::new("temporary").unwrap();
        let mut file = held.open_file(&temp, true).unwrap();
        file.write_all(b"evidence").unwrap();
        file.sync_all().unwrap();
        drop(file);
        held.replace(&temp).unwrap();
        assert_eq!(std::fs::read(moved.join("history")).unwrap(), b"evidence");
        assert!(std::fs::read_dir(outside).unwrap().next().is_none());
    }
    #[test]
    fn failed_history_replacement_preserves_prior_file_and_cleans_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");
        write(&path, b"previous").unwrap();
        let held = native::Directory::open(&path, false).unwrap();
        let temp = std::ffi::CString::new("temporary").unwrap();
        let mut file = held.open_file(&temp, true).unwrap();
        file.write_all(b"next").unwrap();
        file.sync_all().unwrap();
        drop(file);
        let previous = dir.path().join("previous");
        std::fs::rename(&path, &previous).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(held.replace(&temp).is_err());
        held.remove(&temp);
        assert_eq!(std::fs::read(previous).unwrap(), b"previous");
        assert!(!dir.path().join("temporary").exists());
    }
}
