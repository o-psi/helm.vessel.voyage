use super::*;
use std::{
    ffi::CString,
    fs,
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, OpenOptionsExt},
    },
    path::{Component, PathBuf},
};

pub(in crate::attachment::local_actor) struct Directory {
    path: PathBuf,
    file: File,
    parent: File,
}

fn checked(file: File, directory: bool) -> io::Result<File> {
    let metadata = file.metadata()?;
    if metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || if directory {
            !metadata.is_dir()
        } else {
            !metadata.is_file() || metadata.nlink() != 1
        }
    {
        return Err(io::Error::from(io::ErrorKind::PermissionDenied));
    }
    Ok(file)
}
fn open_at(dir: &File, name: &str, flags: i32) -> io::Result<File> {
    let name = CString::new(name)?;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}
fn walk(path: &Path) -> Result<File> {
    ensure!(path.is_absolute(), "local actor directory must be absolute");
    let mut current = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                use std::os::unix::ffi::OsStrExt;
                let name = CString::new(name.as_bytes())?;
                let fd = unsafe {
                    libc::openat(
                        current.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                    )
                };
                ensure!(
                    fd >= 0,
                    "local actor ancestor unavailable or unsafe: {}",
                    io::Error::last_os_error()
                );
                current = unsafe { File::from_raw_fd(fd) };
            }
            _ => anyhow::bail!("local actor path contains non-normal components"),
        }
    }
    Ok(current)
}
impl Directory {
    pub(in crate::attachment::local_actor) fn open(path: &Path) -> Result<Self> {
        let parent = walk(
            path.parent()
                .context("local actor directory needs a parent")?,
        )?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("invalid local actor directory name")?;
        let c_name = CString::new(name)?;
        let made = unsafe { libc::mkdirat(parent.as_raw_fd(), c_name.as_ptr(), 0o700) };
        if made != 0 {
            let error = io::Error::last_os_error();
            ensure!(
                error.kind() == io::ErrorKind::AlreadyExists,
                "cannot create local actor directory: {error}"
            );
        }
        let file = checked(
            open_at(&parent, name, libc::O_RDONLY | libc::O_DIRECTORY)?,
            true,
        )?;
        let directory = Self {
            path: path.into(),
            file,
            parent,
        };
        directory.sync()?;
        directory.verify()?;
        Ok(directory)
    }
    pub(in crate::attachment::local_actor) fn verify(&self) -> Result<()> {
        let current = checked(walk(&self.path)?, true)?;
        let a = current.metadata()?;
        let b = self.file.metadata()?;
        ensure!(
            a.dev() == b.dev() && a.ino() == b.ino(),
            "local actor directory was replaced"
        );
        checked(self.file.try_clone()?, true)?;
        Ok(())
    }
    pub(in crate::attachment::local_actor) fn lock(&self) -> Result<super::Lock> {
        self.lock_after(|_| Ok(()))
    }
    fn lock_after(&self, acquired: impl FnOnce(&File) -> Result<()>) -> Result<super::Lock> {
        self.verify()?;
        let file = checked(
            open_at(&self.file, "actor.lock", libc::O_RDWR | libc::O_CREAT)?,
            false,
        )?;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "local actor storage busy"
        );
        let lock = super::Lock(file);
        acquired(&lock.0)?;
        self.sync()?;
        self.verify()?;
        Ok(lock)
    }
    pub(in crate::attachment::local_actor) fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.verify()?;
        super::read(open_at(&self.file, name, libc::O_RDONLY).and_then(|f| checked(f, false)))
    }
    pub(in crate::attachment::local_actor) fn create(&self, name: &str) -> Result<File> {
        self.verify()?;
        Ok(checked(
            open_at(
                &self.file,
                name,
                libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,
            )?,
            false,
        )?)
    }
    pub(in crate::attachment::local_actor) fn sync_file(&self, name: &str) -> Result<()> {
        checked(open_at(&self.file, name, libc::O_RDONLY)?, false)?.sync_all()?;
        Ok(())
    }
    pub(in crate::attachment::local_actor) fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        self.parent.sync_all()?;
        Ok(())
    }
    pub(in crate::attachment::local_actor) fn publish_new(
        &self,
        name: &str,
        bytes: &[u8],
    ) -> Result<()> {
        let temporary = "publication.json";
        match self.read(temporary)? {
            Some(existing) => ensure!(
                existing == bytes,
                "uncertain local actor publication conflicts"
            ),
            None => {
                let mut file = self.create(temporary)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
        }
        self.sync_file(temporary)?;
        self.sync()?;
        self.verify()?;
        let source = CString::new(temporary)?;
        let target = CString::new(name)?;
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                self.file.as_raw_fd(),
                source.as_ptr(),
                self.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        #[cfg(target_os = "macos")]
        let result = unsafe {
            libc::renameatx_np(
                self.file.as_raw_fd(),
                source.as_ptr(),
                self.file.as_raw_fd(),
                target.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        anyhow::bail!("atomic local actor publication unsupported on this platform");
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        ensure!(
            result == 0,
            "local actor publication failed: {}",
            io::Error::last_os_error()
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_failure_unlocks_while_inherited_description_remains_open() {
        let temp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(temp.path()).unwrap();
        let path = root.join("actor");
        let moved = root.join("moved");
        let directory = Directory::open(&path).unwrap();
        let mut inherited = None;
        let failed = directory.lock_after(|file| {
            // dup and fork both retain the same flock-owning open file description.
            // Keep that description alive deterministically without forking Rust's
            // multithreaded test process or waiting for a child to reach exec.
            inherited = Some(file.try_clone()?);
            fs::rename(&path, &moved)?;
            Ok(())
        });
        assert!(failed.is_err()); // The real post-acquisition path check fails.
        assert!(inherited.is_some());
        let reopened = Directory::open(&moved).unwrap();
        let new_lock = reopened
            .lock()
            .expect("failure must explicitly unlock the inherited description");
        assert!(inherited.as_ref().unwrap().metadata().unwrap().is_file());
        drop(new_lock);
        drop(inherited);
    }
}
