//! Private human access credentials; never provider or model configuration.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
use uuid::Uuid;
use voyage_protocol::vessel::{
    AccessCredential, MAX_VESSEL_BODY, VESSEL_API_VERSION, VesselCommand, VesselEvent,
    VesselEventRequest, VesselRequest, VesselResponse,
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
    fn token(&self) -> &str {
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
pub(super) fn http(streaming: bool) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8));
    if !streaming {
        builder = builder.timeout(Duration::from_secs(15));
    }
    Ok(builder.build()?)
}
fn classified(text: &str) -> Option<ConnectionFailure> {
    let text = text.to_ascii_lowercase();
    if text.contains("expir") {
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
    let parsed = serde_json::from_slice::<VesselResponse>(&bytes);
    if let Ok(reply) = &parsed {
        if reply.protocol != VESSEL_API_VERSION {
            return Err(ConnectionFailure::Version.into());
        }
        if let Some(error) = &reply.error {
            if reply.outcome_unknown {
                anyhow::bail!(
                    "Vessel command outcome unknown; original command identity must be retained"
                );
            }
            if let Some(error) = classified(error) {
                return Err(error.into());
            }
            // Do not echo untrusted HTTP error text (it may include the submitted secret).
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                return Err(ConnectionFailure::Revoked.into());
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
        http(false)?.post(endpoint(
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
pub(super) async fn events_credential(
    credential: &Credential,
    pin: Option<Uuid>,
    request: VesselEventRequest,
) -> Result<futures_util::stream::BoxStream<'static, Result<VesselEvent>>> {
    let request = credential
        .authorize(
            http(true)?
                .post(endpoint(
                    credential.endpoint(),
                    voyage_protocol::vessel::EVENTS_PATH,
                )?)
                .header(reqwest::header::ACCEPT, "text/event-stream"),
            pin,
        )?
        .json(&request)
        .send();
    let response = tokio::time::timeout(Duration::from_secs(15), request)
        .await
        .map_err(|_| ConnectionFailure::Offline)?
        .map_err(|_| ConnectionFailure::Offline)?;
    if !response.status().is_success() {
        envelope(response).await?;
        anyhow::bail!("Vessel event stream refused");
    }
    Ok(super::sse::decode(response))
}
