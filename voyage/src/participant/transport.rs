use anyhow::{Result, ensure};
use std::{path::Path, time::Duration};
use voyage_protocol::process::*;
use voyage_protocol::vessel::{MAX_VESSEL_BODY, VESSEL_API_VERSION};

pub(super) fn credential(path: &Path) -> Result<AccessCredential> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        let meta = file.metadata()?;
        ensure!(
            meta.is_file()
                && meta.nlink() == 1
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o077 == 0
                && meta.len() <= 16384,
            "unsafe participant credential"
        );
        let credential: AccessCredential = serde_json::from_reader(file)?;
        let url = reqwest::Url::parse(&credential.endpoint)?;
        ensure!(
            url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && matches!(url.path(), "" | "/"),
            "participant origin must not contain credentials or paths"
        );
        let loopback = url
            .host_str()
            .and_then(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .ok()
            })
            .is_some_and(|ip| ip.is_loopback());
        ensure!(
            url.scheme() == "https" || (url.scheme() == "http" && loopback),
            "participant requires HTTPS or literal loopback"
        );
        ensure!(
            credential.token.len() == 64 && credential.token.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid participant credential"
        );
        Ok(credential)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        anyhow::bail!("private participant transport unsupported on this platform")
    }
}
pub(super) async fn request(
    credential: &AccessCredential,
    command: VesselCommand,
) -> Result<VesselResponse> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(25))
        .no_proxy()
        .build()?;
    let mut response = client
        .post(format!(
            "{}{}",
            credential.endpoint.trim_end_matches('/'),
            voyage_protocol::vessel::COMMAND_PATH
        ))
        .bearer_auth(&credential.token)
        .header("x-voyage-grant", credential.grant_id.to_string())
        .json(&VesselRequest {
            protocol: VESSEL_API_VERSION,
            command,
        })
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("participant delivery failed; outcome unknown"))?;
    ensure!(
        response.status().is_success(),
        "participant gateway rejected request; outcome unknown"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_VESSEL_BODY,
            "participant frame exceeds limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: VesselResponse = serde_json::from_slice(&bytes)?;
    ensure!(
        response.protocol == VESSEL_API_VERSION,
        "participant protocol mismatch"
    );
    Ok(response)
}
