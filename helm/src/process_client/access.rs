//! Private human access credentials; never provider or model configuration.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
use uuid::Uuid;
use voyage_protocol::vessel::{
    AccessCredential, MAX_VESSEL_BODY, VESSEL_API_VERSION, VesselCommand, VesselRequest,
    VesselResponse,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorkspaceCredential {
    pub schema_version: u32,
    pub kind: String,
    pub endpoint: String,
    pub grant_id: Uuid,
    pub principal_id: Uuid,
    pub vessel_id: Uuid,
    pub token: String,
}
// Deliberately not Debug: even a diagnostic must not expose a token.
#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum Credential {
    Workspace(WorkspaceCredential),
    Session(AccessCredential),
}
impl Credential {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let value: Self = serde_json::from_slice(bytes)
            .map_err(|_| anyhow::anyhow!("invalid private access credential"))?;
        if let Self::Workspace(c) = &value {
            if c.schema_version != 1 || c.kind != "workspace" {
                return Err(ConnectionFailure::Version.into());
            }
            ensure!(
                !c.vessel_id.is_nil() && !c.principal_id.is_nil(),
                "invalid credential identity"
            );
        }
        ensure!(
            !value.grant_id().is_nil()
                && !value.token().is_empty()
                && value.token().len() <= 4096
                && value.token().bytes().all(|b| b.is_ascii_graphic()),
            "invalid access credential"
        );
        endpoint(value.endpoint(), voyage_protocol::vessel::COMMAND_PATH)?;
        Ok(value)
    }
    pub fn endpoint(&self) -> &str {
        match self {
            Self::Workspace(c) => &c.endpoint,
            Self::Session(c) => &c.endpoint,
        }
    }
    pub fn grant_id(&self) -> Uuid {
        match self {
            Self::Workspace(c) => c.grant_id,
            Self::Session(c) => c.grant_id,
        }
    }
    pub(super) fn token(&self) -> &str {
        match self {
            Self::Workspace(c) => &c.token,
            Self::Session(c) => &c.token,
        }
    }
    pub fn vessel_id(&self) -> Option<Uuid> {
        match self {
            Self::Workspace(c) => Some(c.vessel_id),
            _ => None,
        }
    }
    pub fn principal_id(&self) -> Option<Uuid> {
        match self {
            Self::Workspace(c) => Some(c.principal_id),
            _ => None,
        }
    }
    fn authorize(
        &self,
        builder: reqwest::RequestBuilder,
        pin: Option<Uuid>,
    ) -> Result<reqwest::RequestBuilder> {
        ensure!(
            pin.is_none() || self.vessel_id().is_none() || pin == self.vessel_id(),
            "connection credential identity changed"
        );
        let builder = builder
            .bearer_auth(self.token())
            .header("x-voyage-grant", self.grant_id().to_string());
        Ok(if let Some(id) = pin.or(self.vessel_id()) {
            builder.header("x-voyage-vessel", id.to_string())
        } else {
            builder
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConnectionFailure {
    #[error("Access expired; obtain renewed access. Original recovery credentials are retained.")]
    Expired,
    #[error("Access revoked or refused; original recovery credentials are retained.")]
    Revoked,
    #[error(
        "Access unavailable: the server combined expiry and revocation; original recovery credentials are retained."
    )]
    Unavailable,
    #[error("Vessel or principal identity changed; review as a new connection.")]
    Identity,
    #[error("Unsupported Vessel protocol or credential version.")]
    Version,
    #[error("Vessel offline or secure connection failed; command delivery may be unknown.")]
    Offline,
    #[error("Vessel redirect refused; use the approved HTTPS endpoint.")]
    Redirect,
}
pub(super) fn credential(path: &Path) -> Result<Credential> {
    Credential::parse(&super::connections::private::read_path(
        path,
        super::connections::private::CREDENTIAL_LIMIT,
    )?)
}
pub(super) fn endpoint(base: &str, path: &str) -> Result<reqwest::Url> {
    let mut endpoint =
        reqwest::Url::parse(base).map_err(|_| anyhow::anyhow!("invalid Vessel endpoint"))?;
    ensure!(
        endpoint.username().is_empty()
            && endpoint.password().is_none()
            && endpoint.query().is_none()
            && endpoint.fragment().is_none(),
        "Vessel endpoint must not contain credentials, query or fragment"
    );
    let loopback = endpoint.host_str().is_some_and(|host| {
        host.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    ensure!(
        endpoint.host_str().is_some()
            && (endpoint.scheme() == "https" || (endpoint.scheme() == "http" && loopback)),
        "Vessel transport requires HTTPS except literal loopback development"
    );
    endpoint.set_path(path);
    Ok(endpoint)
}
pub(super) fn http(base: &str, streaming: bool) -> Result<reqwest::Client> {
    let endpoint = endpoint(base, "/")?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8));
    // Loopback credentials and pairing secrets must never reach a system proxy.
    if endpoint.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    }) {
        builder = builder.no_proxy();
    }
    if !streaming {
        builder = builder.timeout(Duration::from_secs(15));
    }
    Ok(builder.build()?)
}
fn classified(text: &str) -> Option<ConnectionFailure> {
    let text = text.to_ascii_lowercase();
    if text.contains("expir") && text.contains("revok") {
        Some(ConnectionFailure::Unavailable)
    } else if text.contains("expir") {
        Some(ConnectionFailure::Expired)
    } else if text.contains("revok") {
        Some(ConnectionFailure::Revoked)
    } else if text.contains("identity") || text.contains("pinned") {
        Some(ConnectionFailure::Identity)
    } else if text.contains("protocol") || text.contains("version") {
        Some(ConnectionFailure::Version)
    } else {
        None
    }
}
pub(super) async fn bounded(
    mut response: reqwest::Response,
) -> Result<(reqwest::StatusCode, Vec<u8>)> {
    let status = response.status();
    if status.is_redirection() {
        return Err(ConnectionFailure::Redirect.into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ConnectionFailure::Offline)?
    {
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= MAX_VESSEL_BODY,
            "Vessel response exceeds frame limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok((status, bytes))
}
pub(super) async fn envelope(response: reqwest::Response) -> Result<serde_json::Value> {
    let (status, bytes) = bounded(response).await?;
    // A reverse proxy failure is not a receipt for the submitted command, even
    // if its body happens to resemble a protocol error envelope.
    ensure!(
        !status.is_server_error(),
        "Vessel or proxy failure (HTTP {}); command delivery may be unknown",
        status.as_u16()
    );
    let parsed = serde_json::from_slice::<VesselResponse>(&bytes);
    if let Ok(reply) = &parsed {
        if reply.protocol != VESSEL_API_VERSION {
            return Err(ConnectionFailure::Version.into());
        }
        if reply.outcome_unknown {
            anyhow::bail!(
                "Vessel command outcome unknown; original command identity must be retained"
            );
        }
        if let Some(error) = &reply.error {
            if let Some(error) = classified(error) {
                // Preserve both downcasts: command handlers use Refusal to clear
                // only definitely rejected deliveries; the panel uses the reason.
                return Err(
                    anyhow::Error::new(error).context(super::transport::Refusal(error.to_string()))
                );
            }
            // Do not echo untrusted HTTP error text (it may include the submitted secret).
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let error = ConnectionFailure::Revoked;
                return Err(
                    anyhow::Error::new(error).context(super::transport::Refusal(error.to_string()))
                );
            }
            return Err(super::transport::Refusal("Vessel refused the request".into()).into());
        }
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ConnectionFailure::Revoked.into());
    }
    ensure!(
        status.is_success(),
        "Vessel request failed (HTTP {}); command delivery may be unknown",
        status.as_u16()
    );
    Ok(parsed
        .map_err(|_| anyhow::anyhow!("invalid Vessel response; command delivery may be unknown"))?
        .result)
}
pub(super) async fn exchange_credential(
    credential: &Credential,
    pin: Option<Uuid>,
    command: VesselCommand,
) -> Result<serde_json::Value> {
    let request = credential.authorize(
        http(credential.endpoint(), false)?.post(endpoint(
            credential.endpoint(),
            voyage_protocol::vessel::COMMAND_PATH,
        )?),
        pin,
    )?;
    envelope(
        request
            .json(&VesselRequest {
                protocol: VESSEL_API_VERSION,
                command,
            })
            .send()
            .await
            .map_err(|_| ConnectionFailure::Offline)?,
    )
    .await
}
/// Interpret public socket replies without exposing submitted secrets in errors.
pub(super) fn public_response(reply: VesselResponse, local: bool) -> Result<serde_json::Value> {
    ensure!(
        reply.protocol == VESSEL_API_VERSION,
        "unsupported Vessel response protocol"
    );
    ensure!(
        !reply.outcome_unknown,
        "Vessel command outcome unknown; retain original command identity"
    );
    if let Some(error) = reply.error {
        if let Some(reason) = classified(&error) {
            return Err(
                anyhow::Error::new(reason).context(super::transport::Refusal(reason.to_string()))
            );
        }
        let message = if local {
            format!("Vessel refused: {}", super::safe(&error))
        } else {
            "Vessel refused the request".into()
        };
        return Err(super::transport::Refusal(message).into());
    }
    Ok(reply.result)
}
