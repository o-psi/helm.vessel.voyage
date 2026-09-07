use anyhow::{Result, ensure};
use futures_util::StreamExt;
use std::{path::Path, time::Duration};
use tokio::net::UnixStream;
use voyage_protocol::process::*;

/// Trusted local adapter used by the authenticated HTTP gateway.
pub async fn exchange(directory: &Path, request: &VesselRequest) -> Result<VesselResponse> {
    if !directory.join("process-http.json").try_exists()? {
        return legacy_exchange(directory, request).await;
    }
    let credential = super::registry::load_local_access(directory)?;
    let endpoint = url::Url::parse(&credential.endpoint)?;
    ensure!(
        endpoint.scheme() == "http"
            && endpoint.host_str().is_some_and(|host| {
                host.parse::<std::net::IpAddr>()
                    .is_ok_and(|address| address.is_loopback())
            })
            && endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "invalid local Vessel HTTP endpoint"
    );
    ensure!(
        credential.token.len() == 64
            && credential
                .token
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "invalid local Vessel HTTP credential"
    );
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?
        .post(endpoint.join("/v3/process/command")?)
        .bearer_auth(&credential.token)
        .json(request)
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "local Vessel HTTP request rejected"
    );
    let mut bytes = Vec::new();
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_PROCESS_FRAME,
            "local Vessel response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: VesselResponse = serde_json::from_slice(&bytes)?;
    ensure!(
        response.protocol == PROCESS_PROTOCOL,
        "unsupported Vessel protocol"
    );
    Ok(response)
}

/// Upgrade bridge for a supervisor launched before the HTTP transport existed.
async fn legacy_exchange(directory: &Path, request: &VesselRequest) -> Result<VesselResponse> {
    super::registry::private_directory(directory)?;
    tokio::time::timeout(Duration::from_secs(30), async {
        let mut stream = UnixStream::connect(directory.join("vessel.sock")).await?;
        ensure!(
            stream.peer_cred()?.uid() == unsafe { libc::geteuid() },
            "Vessel peer uid mismatch"
        );
        write_frame(&mut stream, request).await?;
        let response: VesselResponse = read_frame(&mut stream).await?;
        ensure!(
            response.protocol == PROCESS_PROTOCOL,
            "unsupported Vessel protocol"
        );
        Ok(response)
    })
    .await
    .map_err(|_| anyhow::anyhow!("Vessel response deadline exceeded; outcome unknown"))?
}
