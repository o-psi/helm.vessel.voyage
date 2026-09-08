//! Descriptor-relative private storage. A lock covers read/merge/atomic commit.
use anyhow::{Context, Result, ensure};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

pub const CREDENTIAL_LIMIT: usize = 16 * 1024;
pub const REGISTRY_LIMIT: usize = 1024 * 1024;

#[cfg(unix)]
fn check(file: &File, directory: bool, limit: usize) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    ensure!(
        m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0
            && if directory {
                m.is_dir()
            } else {
                m.is_file() && m.nlink() == 1 && m.len() <= limit as u64
            },
        "private connection storage ownership, permissions or size is invalid"
    );
    Ok(())
}

pub fn read_path(path: &Path, limit: usize) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)
            .context("cannot open private access file")?;
        check(&file, false, limit)?;
        read(file, limit)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, limit);
        anyhow::bail!("private connections unsupported on this platform")
    }
}
fn read(file: File, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "private connection file exceeds size limit"
    );
    Ok(bytes)
}

pub struct Directory(File);
impl Directory {
    pub fn open(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
            // Only this last component is created; callers select an existing parent.
            match std::fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e.into()),
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY | libc::O_CLOEXEC)
                .open(path)
                .context("cannot open private connection directory")?;
            check(&file, true, 0)?;
            Ok(Self(file))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
    #[cfg(unix)]
    fn open_file(&self, name: &str, flags: i32, limit: usize) -> Result<File> {
        use std::os::fd::{AsRawFd, FromRawFd};
        ensure!(
            !name.contains('/') && name != "." && name != "..",
            "invalid private storage reference"
        );
        let name = std::ffi::CString::new(name)?;
        let fd = unsafe {
            libc::openat(
                self.0.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        check(&file, false, limit)?;
        Ok(file)
    }
    pub fn read(&self, name: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        #[cfg(unix)]
        {
            match self.open_file(name, libc::O_RDONLY, limit) {
                Ok(file) => Ok(Some(read(file, limit)?)),
                Err(e)
                    if e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    Ok(None)
                }
                Err(e) => Err(e),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = (name, limit);
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
    pub fn lock(&self) -> Result<File> {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = self.open_file("registry.lock", libc::O_RDWR | libc::O_CREAT, 0)?;
            ensure!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
                "connection registry busy in another Helm window; retry"
            );
            Ok(file)
        }
        #[cfg(not(unix))]
        {
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
    pub fn write(&self, name: &str, bytes: &[u8], limit: usize) -> Result<()> {
        ensure!(
            bytes.len() <= limit,
            "private connection file exceeds size limit"
        );
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // Refuse unsafe destinations, even though rename would replace a symlink.
            self.read(name, limit)?;
            let temporary = format!("{}.tmp", uuid::Uuid::new_v4());
            let mut file = self.open_file(
                &temporary,
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                limit,
            )?;
            let tmp = std::ffi::CString::new(temporary)?;
            let destination = std::ffi::CString::new(name)?;
            let result = (|| -> Result<()> {
                file.write_all(bytes)?;
                file.sync_all()?;
                ensure!(
                    unsafe {
                        libc::renameat(
                            self.0.as_raw_fd(),
                            tmp.as_ptr(),
                            self.0.as_raw_fd(),
                            destination.as_ptr(),
                        )
                    } == 0,
                    "cannot commit private connection file"
                );
                self.0.sync_all()?;
                Ok(())
            })();
            if result.is_err() {
                unsafe {
                    libc::unlinkat(self.0.as_raw_fd(), tmp.as_ptr(), 0);
                }
            }
            result
        }
        #[cfg(not(unix))]
        {
            let _ = name;
            anyhow::bail!("private connections unsupported on this platform")
        }
    }
}
