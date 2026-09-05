//! Bounded directory-relative access and create-only publication on supported filesystems.
use anyhow::{Result, bail};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
#[cfg(target_os = "linux")]
use std::io::Write;
use std::{
    io::Read,
    path::{Component, Path, PathBuf},
};
pub(super) struct Entry {
    pub name: String,
    pub directory: bool,
}
pub(super) struct Root {
    path: PathBuf,
    directory: Dir,
}
impl Root {
    pub(super) fn open(path: &Path) -> Result<Self> {
        let path = path.canonicalize()?;
        let directory = Dir::open_ambient_dir(&path, cap_std::ambient_authority())?;
        Ok(Self { path, directory })
    }
    fn relative(&self, path: &Path) -> Result<PathBuf> {
        let path = if path.is_absolute() {
            path.strip_prefix(&self.path)
                .map_err(|_| anyhow::anyhow!("onboarding path must be inside its workspace"))?
        } else {
            path
        };
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            bail!("onboarding path must contain only normal workspace-relative components")
        }
        Ok(path.into())
    }
    fn directory(&self, path: &Path) -> Result<Dir> {
        let mut directory = self.directory.try_clone()?;
        for component in path.components() {
            let Component::Normal(name) = component else {
                bail!("invalid directory component")
            };
            directory = directory.open_dir_nofollow(name)?;
        }
        Ok(directory)
    }
    fn parent(&self, path: &Path) -> Result<(Dir, PathBuf)> {
        let path = self.relative(path)?;
        Ok((
            self.directory(path.parent().unwrap())?,
            path.file_name().unwrap().into(),
        ))
    }
    pub(super) fn read(&self, path: &Path, max: usize) -> Result<Vec<u8>> {
        let (directory, name) = self.parent(path)?;
        let mut options = OpenOptions::new();
        options.read(true).follow(FollowSymlinks::No).nonblock(true);
        let file = directory.open_with(name, &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > max as u64 {
            bail!("onboarding input must be a bounded regular file")
        }
        let mut bytes = Vec::new();
        file.take(max as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > max {
            bail!("onboarding input exceeds limit")
        }
        Ok(bytes)
    }
    pub(super) fn entries(&self, path: &Path) -> Result<Vec<Entry>> {
        let directory = self.directory(path)?;
        let mut entries = Vec::new();
        for entry in directory.entries()?.take(super::MAX_ENTRIES + 1) {
            let entry = entry?;
            entries.push(Entry {
                name: entry.file_name().into_string().unwrap_or_default(),
                directory: entry.file_type()?.is_dir(),
            });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }
    pub(super) fn publish(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.publish_observed(path, bytes, |_| Ok(()))
    }
    #[cfg(target_os = "linux")]
    pub(super) fn publish_observed(
        &self,
        path: &Path,
        bytes: &[u8],
        mut observe: impl FnMut(bool) -> Result<()>,
    ) -> Result<()> {
        use std::{
            ffi::CString,
            os::{
                fd::{AsRawFd, FromRawFd},
                unix::ffi::OsStrExt,
            },
        };
        let (directory, name) = self.parent(path)?;
        // Anonymous staging has no repository pathname that another writer can replace.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".".as_ptr(),
                libc::O_TMPFILE | libc::O_RDWR | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            bail!("filesystem does not support anonymous atomic onboarding publication")
        }
        // openat returned a new owned descriptor; File closes it on every return path.
        let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
        file.write_all(bytes)?;
        file.sync_all()?;
        observe(false)?;
        let source = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        let name = CString::new(name.as_os_str().as_bytes())?;
        // Following this process-owned descriptor avoids CAP_DAC_READ_SEARCH required
        // by AT_EMPTY_PATH. The destination stays relative to the pinned directory.
        if unsafe {
            libc::linkat(
                libc::AT_FDCWD,
                source.as_ptr(),
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::AT_SYMLINK_FOLLOW,
            )
        } != 0
        {
            bail!("destination exists or descriptor-based atomic publication is unavailable")
        }
        directory.open(".")?.sync_all()?;
        observe(true)?;
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    pub(super) fn publish_observed(
        &self,
        _: &Path,
        _: &[u8],
        _: impl FnMut(bool) -> Result<()>,
    ) -> Result<()> {
        bail!(
            "onboarding file publication currently requires Linux anonymous staging; preview to stdout remains available"
        )
    }
}
