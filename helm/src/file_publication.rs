//! Create-only publication of retained bytes, with no discoverable staging pathname.
//! Callers choose and pin the destination directory before preparing publication.
use anyhow::{Result, bail};
use cap_std::fs::Dir;
use std::path::{Component, Path};

pub(crate) struct Publication {
    #[cfg(target_os = "linux")]
    directory: Dir,
    #[cfg(target_os = "linux")]
    file: std::fs::File,
    #[cfg(target_os = "linux")]
    name: std::ffi::CString,
}

impl Publication {
    /// Preparation creates only an anonymous file. Unsupported platforms fail here,
    /// so callers can perform this check before provider requests or other effects.
    pub(crate) fn prepare(directory: Dir, name: &Path) -> Result<Self> {
        let mut components = name.components();
        if !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name.file_name() != Some(name.as_os_str())
        {
            bail!("publication requires one destination filename")
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::{
                fd::{AsRawFd, FromRawFd},
                unix::ffi::OsStrExt,
            };
            let name = std::ffi::CString::new(name.as_os_str().as_bytes())?;
            // No repository pathname exists for another writer to reopen or swap.
            let fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    c".".as_ptr(),
                    libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                bail!("filesystem does not support anonymous create-only publication")
            }
            // openat returned a new owned descriptor, closed on every return path.
            let file = unsafe { std::fs::File::from_raw_fd(fd) };
            Ok(Self {
                directory,
                file,
                name,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = directory;
            bail!("anonymous create-only publication currently requires Linux")
        }
    }

    pub(crate) fn publish(self, bytes: &[u8]) -> Result<()> {
        self.publish_observed(bytes, |_| Ok(()))
    }

    pub(crate) fn publish_observed(
        self,
        bytes: &[u8],
        mut observe: impl FnMut(bool) -> Result<()>,
    ) -> Result<()> {
        #[cfg(target_os = "linux")]
        {
            use std::{io::Write, os::fd::AsRawFd};
            let mut file = self.file;
            file.write_all(bytes)?;
            file.sync_all()?;
            observe(false)?;
            let source = std::ffi::CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
            // Follow only our retained descriptor; never replace an existing name.
            // This avoids AT_EMPTY_PATH's CAP_DAC_READ_SEARCH requirement.
            if unsafe {
                libc::linkat(
                    libc::AT_FDCWD,
                    source.as_ptr(),
                    self.directory.as_raw_fd(),
                    self.name.as_ptr(),
                    libc::AT_SYMLINK_FOLLOW,
                )
            } != 0
            {
                bail!(
                    "publication failed; inspect destination before retrying (existing files are never replaced)"
                )
            }
            self.directory.open(".")?.sync_all().map_err(|_| anyhow::anyhow!(
                "publication may have succeeded but directory synchronization failed; inspect destination before retrying"
            ))?;
            observe(true)?;
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (bytes, &mut observe);
            bail!("anonymous publication is unavailable on this platform")
        }
    }
}

#[cfg(test)]
mod tests;
