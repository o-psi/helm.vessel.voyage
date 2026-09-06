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

pub struct Directory {
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
    if flags & libc::O_DIRECTORY == 0 {
        validate_name(name).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    }
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
    pub fn open(path: &Path) -> Result<Self> {
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
    /// Pin an existing private directory without creating or syncing any state.
    pub fn open_existing(path: &Path) -> Result<Self> {
        let parent = walk(
            path.parent()
                .context("local actor directory needs a parent")?,
        )?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("invalid local actor directory name")?;
        let file = checked(
            open_at(&parent, name, libc::O_RDONLY | libc::O_DIRECTORY)?,
            true,
        )?;
        let directory = Self {
            path: path.into(),
            file,
            parent,
        };
        directory.verify()?;
        Ok(directory)
    }
    /// A shared reader never creates a missing lock file or changes durability.
    pub fn read_lock(&self) -> Result<super::Lock> {
        self.read_lock_after(|_| Ok(()))
    }
    fn read_lock_after(&self, acquired: impl FnOnce(&File) -> Result<()>) -> Result<super::Lock> {
        self.verify()?;
        let file = checked(open_at(&self.file, "actor.lock", libc::O_RDONLY)?, false)?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error()).context("local actor storage busy");
        }
        let lock = super::Lock(file);
        acquired(&lock.0)?;
        self.verify()?;
        Ok(lock)
    }
    pub fn verify(&self) -> Result<()> {
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
    pub fn lock(&self) -> Result<super::Lock> {
        self.lock_after(|_| Ok(()))
    }
    fn lock_after(&self, acquired: impl FnOnce(&File) -> Result<()>) -> Result<super::Lock> {
        self.verify()?;
        let file = checked(
            open_at(&self.file, "actor.lock", libc::O_RDWR | libc::O_CREAT)?,
            false,
        )?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error()).context("local actor storage busy");
        }
        let lock = super::Lock(file);
        acquired(&lock.0)?;
        self.sync()?;
        self.verify()?;
        Ok(lock)
    }
    pub fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.read_bounded(name, MAX_BYTES)
    }
    pub fn read_bounded(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        validate_limit(limit)?;
        self.verify()?;
        super::read_bounded(
            open_at(&self.file, name, libc::O_RDONLY).and_then(|f| checked(f, false)),
            limit,
        )
    }
    pub fn create(&self, name: &str) -> Result<File> {
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
    pub fn sync_file(&self, name: &str) -> Result<()> {
        checked(open_at(&self.file, name, libc::O_RDONLY)?, false)?.sync_all()?;
        Ok(())
    }
    pub fn sync(&self) -> Result<()> {
        self.file.sync_all()?;
        self.parent.sync_all()?;
        Ok(())
    }
    /// Replace a private record under the caller's exclusive directory lock.
    pub fn publish(&self, name: &str, bytes: &[u8]) -> Result<()> {
        validate_name(name)?;
        validate_limit(bytes.len())?;
        // Check existing destinations without following links before any write.
        self.read_bounded(name, MAX_PRIVATE_BYTES)?;
        let temporary = format!(".tmp-{}", uuid::Uuid::new_v4());
        let mut file = self.create(&temporary)?;
        let source = CString::new(temporary)?;
        let target = CString::new(name)?;
        let result = (|| {
            file.write_all(bytes)?;
            file.sync_all()?;
            self.verify()?;
            self.read_bounded(name, MAX_PRIVATE_BYTES)?;
            ensure!(
                unsafe {
                    libc::renameat(
                        self.file.as_raw_fd(),
                        source.as_ptr(),
                        self.file.as_raw_fd(),
                        target.as_ptr(),
                    )
                } == 0,
                "private record publication failed: {}",
                io::Error::last_os_error()
            );
            self.sync()?;
            self.verify()?;
            Ok(())
        })();
        // This random name belongs only to this operation, never existing data.
        unsafe {
            libc::unlinkat(self.file.as_raw_fd(), source.as_ptr(), 0);
        }
        result
    }
    pub fn publish_new(&self, name: &str, bytes: &[u8]) -> Result<()> {
        validate_name(name)?;
        validate_limit(bytes.len())?;
        let temporary = publication_name(name)?;
        match self.read_bounded(&temporary, MAX_PRIVATE_BYTES)? {
            Some(existing) => ensure!(
                existing == bytes,
                "uncertain local actor publication conflicts"
            ),
            None => {
                let mut file = self.create(&temporary)?;
                file.write_all(bytes)?;
                file.sync_all()?;
            }
        }
        self.sync_file(&temporary)?;
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
