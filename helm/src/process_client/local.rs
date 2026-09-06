//! Discover a private local service and start its independent lifetime if absent.
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use std::{path::Path, time::Duration};
use voyage_protocol::process::VesselCommand;

#[cfg(unix)]
pub fn check_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "Vessel directory must be an owned private directory"
    );
    Ok(())
}

#[cfg(not(unix))]
pub fn check_private_directory(_path: &Path) -> Result<()> {
    anyhow::bail!("private Vessel service is not implemented on this platform")
}

pub async fn connect(directory: std::path::PathBuf, auto_start: bool) -> Result<Client> {
    let client = Client {
        directory,
        ssh: None,
    };
    if client.directory.exists() {
        check_private_directory(&client.directory)?;
        if client.request(VesselCommand::Capabilities).await.is_ok() {
            return Ok(client);
        }
    }
    ensure!(
        auto_start,
        "Vessel is unavailable and automatic startup is disabled"
    );
    start(&client.directory)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if client.request(VesselCommand::Capabilities).await.is_ok() {
            return Ok(client);
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "local Vessel did not become available; inspect its private vessel.log"
        );
    }
}

#[cfg(unix)]
fn start(directory: &Path) -> Result<()> {
    use std::os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt},
        process::CommandExt,
    };
    let parent = directory
        .parent()
        .context("Vessel directory has no parent")?;
    std::fs::create_dir_all(parent)?;
    match std::fs::DirBuilder::new().mode(0o700).create(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    check_private_directory(directory)?;
    let executable = std::env::current_exe()?;
    let binary_directory = executable
        .parent()
        .context("Helm has no executable directory")?;
    let vessel = binary_directory.join("vessel");
    let voyage = binary_directory.join("voyage");
    ensure!(
        vessel.is_file() && voyage.is_file(),
        "install vessel and voyage beside helm"
    );
    let log = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory.join("vessel.log"))?;
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = log.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Vessel log must be an owned private file"
        );
    }
    let mut child = std::process::Command::new(vessel)
        .arg("local-serve")
        .arg("--directory")
        .arg(directory)
        .arg("--voyage-binary")
        .arg(voyage)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log)
        .process_group(0)
        .spawn()
        .context("start local Vessel")?;
    // Reap while Helm is alive, without binding service lifetime to this handle.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(not(unix))]
fn start(_directory: &Path) -> Result<()> {
    anyhow::bail!("automatic Vessel startup is not implemented on this platform")
}
