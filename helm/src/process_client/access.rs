//! Grant credentials are read from private files and never appear in arguments.
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::Path};
use voyage_protocol::process::{
    AccessCredential, MAX_PROCESS_FRAME, PROCESS_PROTOCOL, VesselCommand, VesselRequest,
    VesselResponse,
};

fn credential(path: &Path) -> Result<AccessCredential> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .context("open private access credential")?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.len() <= 16384,
            "access credential must be an owned private file of at most 16 KiB"
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid access credential file"))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        anyhow::bail!("private access credential verification is unsupported on this platform")
    }
}

pub(super) async fn exchange(path: &Path, command: VesselCommand) -> Result<serde_json::Value> {
    let credential = credential(path)?;
    let mut endpoint = reqwest::Url::parse(&credential.endpoint)
        .map_err(|_| anyhow::anyhow!("invalid grant endpoint"))?;
    ensure!(
        endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "grant endpoint must not contain credentials, query or fragment"
    );
    let loopback = endpoint.host_str().is_some_and(|host| {
        host.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback),
        "grant transport requires HTTPS except literal loopback development"
    );
    endpoint.set_path("/v3/process/command");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let mut response = client
        .post(endpoint)
        .bearer_auth(&credential.token)
        .header("x-voyage-grant", credential.grant_id.to_string())
        .json(&VesselRequest {
            protocol: PROCESS_PROTOCOL,
            command,
        })
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("grant connection failed; command delivery may be unknown"))?;
    ensure!(
        response.status().is_success(),
        "grant gateway rejected request (HTTP {})",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        anyhow::anyhow!("grant response interrupted; command delivery may be unknown")
    })? {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_PROCESS_FRAME,
            "grant response exceeds frame limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: VesselResponse = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid grant response; command delivery may be unknown"))?;
    ensure!(
        response.protocol == PROCESS_PROTOCOL,
        "unsupported grant protocol"
    );
    if let Some(error) = response.error {
        if response.outcome_unknown {
            anyhow::bail!("Vessel command outcome unknown: {}", super::safe(&error));
        }
        return Err(
            super::transport::Refusal(format!("Vessel refused: {}", super::safe(&error))).into(),
        );
    }
    Ok(response.result)
}
