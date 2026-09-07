//! Account-local supervisor wake uses the public service contract, not runtime IPC.
use anyhow::{Context, Result, ensure};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{io::Read, path::Path, time::Duration};
use voyage_protocol::vessel::{
    COMMAND_PATH, LocalAccessCredential, MAX_VESSEL_BODY, VESSEL_API_VERSION, VesselCommand,
    VesselRequest, VesselResponse,
};

pub(super) async fn wake(directory: &Path, session_id: uuid::Uuid) -> Result<()> {
    let root = directory
        .parent()
        .and_then(Path::parent)
        .context("outbound session directory missing supervisor")?;
    let metadata = std::fs::symlink_metadata(root)?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe supervisor directory"
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join("process-http.json"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() < 4096,
        "unsafe supervisor discovery credential"
    );
    let mut bytes = Vec::new();
    file.take(4096).read_to_end(&mut bytes)?;
    let credential: LocalAccessCredential = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid supervisor discovery credential"))?;
    let mut endpoint = reqwest::Url::parse(&credential.endpoint)
        .map_err(|_| anyhow::anyhow!("invalid supervisor endpoint"))?;
    ensure!(
        endpoint.scheme() == "http"
            && endpoint.host_str().is_some_and(|host| host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback()))
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "supervisor wake requires literal-loopback HTTP"
    );
    ensure!(
        credential.token.len() == 64
            && credential
                .token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "invalid supervisor credential"
    );
    endpoint.set_path(COMMAND_PATH);
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .build()?
        .post(endpoint)
        .bearer_auth(&credential.token)
        .json(&VesselRequest {
            protocol: VESSEL_API_VERSION,
            command: VesselCommand::Wake { session_id },
        })
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("supervisor wake transport unavailable"))?;
    ensure!(response.status().is_success(), "supervisor wake rejected");
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("supervisor wake response interrupted"))?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_VESSEL_BODY,
            "supervisor wake response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let reply: VesselResponse = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid supervisor wake response"))?;
    ensure!(
        reply.protocol == VESSEL_API_VERSION && reply.error.is_none() && !reply.outcome_unknown,
        "outbound wake unconfirmed"
    );
    Ok(())
}
