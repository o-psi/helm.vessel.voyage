#[cfg(target_os = "linux")]
use super::MAX_DOCUMENT;
use super::{CeilingDocument, Error, Result};
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        ffi::CString,
        fs::File,
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
    };
    fn checked(file: &File, directory: bool, owner: u32) -> Result<()> {
        let m = file.metadata().map_err(|_| Error::Ceiling)?;
        if m.uid() != owner
            || m.mode() & 0o022 != 0
            || if directory {
                !m.is_dir()
            } else {
                !m.is_file() || m.nlink() != 1 || m.len() > MAX_DOCUMENT as u64
            }
        {
            return Err(Error::Ceiling);
        }
        Ok(())
    }
    fn open(parent: &File, name: &str, directory: bool) -> std::io::Result<File> {
        let name = CString::new(name)?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_CLOEXEC
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    struct Pin {
        parent: File,
        name: String,
        file: File,
        directory: bool,
    }
    fn verify(root: &File, pins: &[Pin], owner: u32) -> Result<()> {
        checked(root, true, owner)?;
        for pin in pins {
            checked(&pin.file, pin.directory, owner)?;
            let current =
                open(&pin.parent, &pin.name, pin.directory).map_err(|_| Error::Ceiling)?;
            checked(&current, pin.directory, owner)?;
            let a = current.metadata().map_err(|_| Error::Ceiling)?;
            let b = pin.file.metadata().map_err(|_| Error::Ceiling)?;
            if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
                return Err(Error::Ceiling);
            }
        }
        Ok(())
    }
    pub(super) fn read(
        root: File,
        parts: &[&str],
        owner: u32,
        after: impl FnOnce() -> Result<()>,
    ) -> Result<Option<CeilingDocument>> {
        checked(&root, true, owner)?;
        let mut parent = root.try_clone().map_err(|_| Error::Ceiling)?;
        let mut pins = Vec::new();
        for (index, name) in parts.iter().enumerate() {
            let directory = index + 1 < parts.len();
            let file = match open(&parent, name, directory) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    verify(&root, &pins, owner)?;
                    return Ok(None);
                }
                Err(_) => return Err(Error::Ceiling),
            };
            checked(&file, directory, owner)?;
            pins.push(Pin {
                parent: parent.try_clone().map_err(|_| Error::Ceiling)?,
                name: (*name).into(),
                file: file.try_clone().map_err(|_| Error::Ceiling)?,
                directory,
            });
            parent = file;
        }
        let before = parent.metadata().map_err(|_| Error::Ceiling)?;
        let mut bytes = Vec::new();
        parent
            .by_ref()
            .take(MAX_DOCUMENT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::Ceiling)?;
        after()?;
        verify(&root, &pins, owner)?;
        let last = parent.metadata().map_err(|_| Error::Ceiling)?;
        if (
            before.len(),
            before.mtime(),
            before.mtime_nsec(),
            before.ctime(),
            before.ctime_nsec(),
        ) != (
            last.len(),
            last.mtime(),
            last.mtime_nsec(),
            last.ctime(),
            last.ctime_nsec(),
        ) {
            return Err(Error::Ceiling);
        }
        CeilingDocument::decode(&bytes).map(Some)
    }
    pub(super) fn load() -> Result<Option<CeilingDocument>> {
        use std::os::unix::fs::OpenOptionsExt;
        let root = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(|_| Error::Ceiling)?;
        read(root, &["etc", "helm", "policy-ceiling.toml"], 0, || Ok(()))
    }
}
pub(super) fn load() -> Result<Option<CeilingDocument>> {
    #[cfg(target_os = "linux")]
    {
        linux::load()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(Error::UnsupportedPlatform)
    }
}
