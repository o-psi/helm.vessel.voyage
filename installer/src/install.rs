use anyhow::Result;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
mod files;
#[cfg(target_os = "linux")]
mod layout;
#[cfg(target_os = "linux")]
mod links;
#[cfg(target_os = "linux")]
mod release;
#[cfg(target_os = "linux")]
mod status;
#[cfg(target_os = "linux")]
mod transaction;
#[cfg(target_os = "linux")]
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
    #[cfg(target_os = "linux")]
    {
        transaction::install(options)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = options;
        anyhow::bail!("Installation is currently supported on Linux")
    }
}
pub fn rollback(dry_run: bool) -> Result<Report> {
    #[cfg(target_os = "linux")]
    {
        transaction::rollback(dry_run)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dry_run;
        anyhow::bail!("Installation is currently supported on Linux")
    }
}

/// Hold across binary publication, service activation and any compensating rollback.
pub struct OperationLock {
    #[cfg(target_os = "linux")]
    _file: std::fs::File,
}
pub fn operation_lock() -> Result<OperationLock> {
    #[cfg(target_os = "linux")]
    {
        let layout = layout::Layout::get()?;
        files::private_directory(&layout.root)?;
        Ok(OperationLock {
            _file: files::lock(&layout.root.join("operation.lock"))?,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Installation is currently supported on Linux")
    }
}

pub fn status() -> Result<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        status::inspect()
    }
    #[cfg(not(target_os = "linux"))]
    {
        anyhow::bail!("Installation inspection is currently supported on Linux")
    }
}
