//! Bounded directory-relative access and create-only publication on supported filesystems.
use anyhow::{Result, bail};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
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
        let (directory, name) = self.parent(path)?;
        let _guard = instruction_guard(&directory, &name, bytes)?;
        crate::file_publication::Publication::prepare(directory, &name)?.publish(bytes)
    }
    #[cfg(test)]
    pub(super) fn publish_observed(
        &self,
        path: &Path,
        bytes: &[u8],
        observe: impl FnMut(bool) -> Result<()>,
    ) -> Result<()> {
        let (directory, name) = self.parent(path)?;
        let _guard = instruction_guard(&directory, &name, bytes)?;
        crate::file_publication::Publication::prepare(directory, &name)?
            .publish_observed(bytes, observe)
    }
}

// The fresh directory open gives every writer its own flock description. A dup
// of Root's descriptor would share a lock and fail to serialize same-process use.
struct InstructionGuard {
    #[cfg(target_os = "linux")]
    file: cap_std::fs::File,
}
#[cfg(target_os = "linux")]
impl Drop for InstructionGuard {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        // Explicit unlock also releases locks inherited by a concurrent fork.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
fn instruction_guard(
    directory: &Dir,
    name: &Path,
    bytes: &[u8],
) -> Result<Option<InstructionGuard>> {
    if name != Path::new("AGENTS.md") && name != Path::new("agents.md") {
        return Ok(None);
    }
    if bytes.len() as u64 > crate::workspace_instructions::MAX_BYTES {
        bail!("active workspace guidance exceeds the runtime 64 KiB limit; use a sidecar draft")
    }
    #[cfg(target_os = "linux")]
    let guard = {
        use std::os::fd::AsRawFd;
        let file = directory.open(".")?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            bail!(
                "another onboarding guidance publication owns this directory; retry after it finishes"
            )
        }
        InstructionGuard { file }
    };
    #[cfg(not(target_os = "linux"))]
    let guard = InstructionGuard {};
    for existing in ["AGENTS.md", "agents.md"] {
        match directory.symlink_metadata(existing) {
            Ok(_) => bail!(
                "existing agent guidance must be preserved; publish a sidecar and merge manually"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(guard))
}
