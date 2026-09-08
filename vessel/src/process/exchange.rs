use anyhow::{Result, ensure};
use futures_util::StreamExt;
use std::{path::Path, time::Duration};
use voyage_protocol::process::*;
use voyage_protocol::vessel::{MAX_VESSEL_BODY, VESSEL_API_VERSION};

/// Trusted local adapter used by the authenticated HTTP gateway.
pub async fn exchange(directory: &Path, request: &VesselRequest) -> Result<VesselResponse> {
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
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?
        .post(endpoint.join(voyage_protocol::vessel::COMMAND_PATH)?)
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
            bytes.len().saturating_add(chunk.len()) <= MAX_VESSEL_BODY,
            "local Vessel response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let response: VesselResponse = serde_json::from_slice(&bytes)?;
    ensure!(
        response.protocol == VESSEL_API_VERSION,
        "unsupported Vessel protocol"
    );
    Ok(response)
}
