#[cfg(windows)]
#[allow(dead_code)]
pub(crate) mod native_ntfs;
#[allow(dead_code)]
pub(crate) mod native_receipt;
use anyhow::Result;
use std::path::PathBuf;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod files;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod layout;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod links;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod release;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod status;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod transaction;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod version;

pub struct Options {
    pub bin_dir: PathBuf,
    pub replace_existing: bool,
    pub dry_run: bool,
}
pub struct Report {
    pub release: String,
    pub release_dir: PathBuf,
    pub changed: bool,
    pub current_release: Option<String>,
    pub bin_dir: PathBuf,
    pub actions: Vec<String>,
}
pub fn run(options: Options) -> Result<Report> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        transaction::install(options)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = options;
        anyhow::bail!("Installation is currently supported on Linux")
    }
}
pub fn rollback(dry_run: bool) -> Result<Report> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        transaction::rollback(dry_run)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = dry_run;
        anyhow::bail!("Installation is currently supported on Linux")
    }
}

/// Hold across binary publication, service activation and any compensating rollback.
pub struct OperationLock {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    _file: std::fs::File,
}
pub fn operation_lock() -> Result<OperationLock> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let layout = layout::Layout::get()?;
        files::private_directory(&layout.root)?;
        Ok(OperationLock {
            _file: files::lock(&layout.root.join("operation.lock"))?,
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("Installation is currently supported on Linux")
    }
}

pub fn status() -> Result<Vec<String>> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        status::inspect()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        anyhow::bail!("Installation inspection is currently supported on Linux")
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn rollback_restored_legacy(
    guard: &crate::legacy::Guard,
    expected_current: &str,
    expected_previous: &str,
) -> Result<Report> {
    transaction::rollback_restored_legacy(guard, expected_current, expected_previous)
}
