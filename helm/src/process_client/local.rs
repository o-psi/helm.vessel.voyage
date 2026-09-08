//! Discover a private local service and start its independent lifetime if absent.
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path, time::Duration};
use voyage_protocol::vessel::{
    LocalAccessCredential, MAX_VESSEL_BODY, VESSEL_API_VERSION, VesselCommand, VesselEventRequest,
    VesselRequest, VesselResponse,
};

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
    let client = Client::local(directory);
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

fn credential(directory: &Path) -> Result<LocalAccessCredential> {
    check_private_directory(directory)?;
    let path = directory.join("process-http.json");
    let file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .context("open private local Vessel credential")?
    };
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.len() < 4096,
            "invalid private local Vessel credential"
        );
    }
    let mut bytes = Vec::new();
    file.take(4096).read_to_end(&mut bytes)?;
    let credential: LocalAccessCredential = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid private local Vessel credential"))?;
    ensure!(
        credential.token.len() == 64
            && credential
                .token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "invalid private local Vessel credential"
    );
    Ok(credential)
}

fn endpoint(credential: &LocalAccessCredential, path: &str) -> Result<reqwest::Url> {
    let mut endpoint = reqwest::Url::parse(&credential.endpoint)
        .map_err(|_| anyhow::anyhow!("invalid local Vessel HTTP endpoint"))?;
    let loopback = endpoint.host_str().is_some_and(|host| {
        host.parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
    });
    ensure!(
        endpoint.scheme() == "http"
            && loopback
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "local Vessel transport requires credential-free literal-loopback HTTP"
    );
    endpoint.set_path(path);
    Ok(endpoint)
}

pub(super) async fn exchange(
    directory: &Path,
    command: VesselCommand,
) -> Result<serde_json::Value> {
    let credential = credential(directory)?;
    let mut response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .build()?
        .post(endpoint(
            &credential,
            voyage_protocol::vessel::COMMAND_PATH,
        )?)
        .bearer_auth(&credential.token)
        .json(&VesselRequest {
            protocol: VESSEL_API_VERSION,
            command,
        })
        .send()
        .await
        .map_err(|_| {
            anyhow::anyhow!("local Vessel connection failed; command delivery may be unknown")
        })?;
    ensure!(
        response.status().is_success(),
        "local Vessel rejected request (HTTP {})",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        anyhow::anyhow!("local Vessel response interrupted; command delivery may be unknown")
    })? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_VESSEL_BODY,
            "local Vessel response exceeds frame limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let reply: VesselResponse = serde_json::from_slice(&bytes).map_err(|_| {
        anyhow::anyhow!("invalid local Vessel response; command delivery may be unknown")
    })?;
    ensure!(
        reply.protocol == VESSEL_API_VERSION,
        "unsupported Vessel protocol"
    );
    if let Some(error) = reply.error {
        if reply.outcome_unknown {
            anyhow::bail!("Vessel command outcome unknown: {}", super::safe(&error));
        }
        return Err(
            super::transport::Refusal(format!("Vessel refused: {}", super::safe(&error))).into(),
        );
    }
    Ok(reply.result)
}

pub(super) async fn events(
    directory: &Path,
    request: VesselEventRequest,
) -> Result<futures_util::stream::BoxStream<'static, Result<voyage_protocol::vessel::VesselEvent>>>
{
    let credential = credential(directory)?;
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .build()?
        .post(endpoint(&credential, voyage_protocol::vessel::EVENTS_PATH)?)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .bearer_auth(&credential.token)
        .json(&request)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("local Vessel event connection failed"))?;
    Ok(super::sse::decode(response))
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
