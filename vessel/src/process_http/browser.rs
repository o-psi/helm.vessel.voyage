//! Opt-in browser transport: short-lived, origin-bound credentials are never URLs.
use super::*;
use futures_util::StreamExt;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use vessel::duplex::Backend;

pub(crate) const CREDENTIALS_PATH: &str = "/v1/vessel/browser-credentials";
pub(crate) const SOCKET_PATH: &str = "/v1/vessel/browser-socket";
const TTL: Duration = Duration::from_secs(120);
const MAX_CREDENTIALS: usize = 1024;
#[derive(Clone, Default)]
pub(crate) struct Credentials(Arc<Mutex<HashMap<String, Credential>>>);
#[derive(Clone)]
struct Credential {
    origin: String,
    backend: SocketBackend,
    deadline: tokio::time::Instant,
}
impl Credentials {
    fn insert(&self, origin: String, backend: SocketBackend) -> Result<(String, u64), StatusCode> {
        use ring::rand::{SecureRandom, SystemRandom};
        let mut bytes = [0; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut store = self.0.lock().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        let now = tokio::time::Instant::now();
        store.retain(|_, c| c.deadline > now);
        if store.len() >= MAX_CREDENTIALS {
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        store.insert(
            token.clone(),
            Credential {
                origin,
                backend,
                deadline: now + TTL,
            },
        );
        Ok((
            token,
            (chrono::Utc::now().timestamp_millis() as u64) + TTL.as_millis() as u64,
        ))
    }
    // Single-use authentication prevents socket fan-out with a stolen temporary token.
    fn take(&self, token: &str, origin: &str) -> Option<Credential> {
        let mut store = self.0.lock().ok()?;
        let now = tokio::time::Instant::now();
        store.retain(|_, c| c.deadline > now);
        if store.get(token)?.origin != origin {
            return None;
        }
        store.remove(token)
    }
}
fn canonical_origin(origin: &str) -> bool {
    vessel::origin::validate_origin(origin, false).is_ok_and(|canonical| canonical == origin)
}
fn request_origin(headers: &HeaderMap) -> Option<String> {
    if headers.get_all("origin").iter().count() != 1 {
        return None;
    }
    let origin = headers.get("origin")?.to_str().ok()?;
    canonical_origin(origin).then(|| origin.to_owned())
}
pub(crate) async fn boundary(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    // This exception is intentionally installed on exactly one route. Minting and
    // all native endpoints retain the ordinary public-origin boundary.
    if state.public_origin.is_none() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    if request.uri().path() != SOCKET_PATH
        || request.uri().query().is_some()
        || request_origin(request.headers()).is_none()
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(request).await
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MintRequest {
    origin: String,
}
pub(crate) async fn mint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MintRequest>,
) -> Response {
    let Some(directory) = state.process_directory else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if !canonical_origin(&request.origin) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if headers.get_all("authorization").iter().count() != 1
        || headers.get_all("x-voyage-grant").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let expected_vessel_id = match expected_vessel(&headers) {
        Ok(Some(id)) => id,
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    let Some(token) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(grant_id) = headers
        .get("x-voyage-grant")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .filter(|v| !v.is_nil())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let mut backend = SocketBackend {
        directory,
        expected_vessel_id: Some(expected_vessel_id),
        grant_id,
        token: token.into(),
        authority: serde_json::Value::Null,
        browser_sockets: Default::default(),
    };
    let initial = match tokio::time::timeout(
        Duration::from_secs(3),
        backend.exchange(VesselCommand::Capabilities),
    )
    .await
    {
        Ok(response) if response.error.is_none() && !response.outcome_unknown => response,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    if initial
        .result
        .get("vessel_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|v| Uuid::parse_str(v).ok())
        != Some(expected_vessel_id)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    backend.authority = socket_authority(initial.result);
    match state.browser_credentials.insert(request.origin, backend) {
        Ok((token, expires_at_ms)) => Json(serde_json::json!({"token": token, "expires_at_ms": expires_at_ms, "vessel_id": expected_vessel_id})).into_response(),
        Err(status) => status.into_response(),
    }
}
#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Authentication {
    Authenticate { token: String },
}
struct BrowserBackend(Credential);
impl Backend for BrowserBackend {
    fn connected(&self, connection: vessel::duplex::Connection) {
        self.0.backend.connected(connection);
    }
    fn socket_command(
        &self,
        request: VesselRequest,
        socket_id: Uuid,
    ) -> vessel::duplex::BackendFuture<VesselResponse> {
        let credential = self.0.clone();
        Box::pin(async move {
            if credential.deadline <= tokio::time::Instant::now() || !allowed(&request.command) {
                return VesselResponse {
                    protocol: VESSEL_API_VERSION,
                    result: serde_json::Value::Null,
                    error: Some("browser command refused".into()),
                    outcome_unknown: false,
                };
            }
            credential.backend.socket_command(request, socket_id).await
        })
    }
    fn disconnected(&self, socket_id: Uuid) {
        self.0.backend.disconnected(socket_id);
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        Some(self.0.deadline)
    }
    fn accepts(&self, frame: &voyage_protocol::duplex::ClientFrame) -> bool {
        match frame {
            voyage_protocol::duplex::ClientFrame::Command { request, .. } => {
                allowed(&request.command)
            }
            voyage_protocol::duplex::ClientFrame::Subscribe { .. }
            | voyage_protocol::duplex::ClientFrame::Unsubscribe { .. } => true,
            // Reverse browser-work requests belong only to the explicit native
            // shared-browser consent boundary, never the web UI connection.
            _ => false,
        }
    }
    fn authorize(&self, session: Option<Uuid>) -> vessel::duplex::BackendFuture<bool> {
        let credential = self.0.clone();
        Box::pin(async move {
            credential.deadline > tokio::time::Instant::now()
                && credential.backend.authorize(session).await
                && credential.deadline > tokio::time::Instant::now()
        })
    }
    fn command(&self, request: VesselRequest) -> vessel::duplex::BackendFuture<VesselResponse> {
        let credential = self.0.clone();
        Box::pin(async move {
            if credential.deadline <= tokio::time::Instant::now() || !allowed(&request.command) {
                return VesselResponse {
                    protocol: VESSEL_API_VERSION,
                    result: serde_json::Value::Null,
                    error: Some("browser command refused".into()),
                    outcome_unknown: false,
                };
            }
            credential.backend.command(request).await
        })
    }
}
// Deliberately mirror web/gateway/protocol.js, not the full native command surface.
fn allowed(command: &VesselCommand) -> bool {
    match command {
        VesselCommand::Capabilities
        | VesselCommand::Catalogue
        | VesselCommand::Inspect { .. }
        | VesselCommand::Branch { .. }
        | VesselCommand::Restart { .. }
        | VesselCommand::Accounts { .. }
        | VesselCommand::EnrollAccount { .. }
        | VesselCommand::ResolveAccountEnrollment { .. }
        | VesselCommand::CancelAccountEnrollment { .. }
        | VesselCommand::PrivateAccountEnrollment { .. }
        | VesselCommand::AccountDefaults { .. }
        | VesselCommand::Profiles { .. }
        | VesselCommand::SaveProfile { .. }
        | VesselCommand::DeleteProfile { .. }
        | VesselCommand::SetDefaultProfile { .. }
        | VesselCommand::AccountUsage { .. }
        | VesselCommand::AccountModels { .. }
        | VesselCommand::StartAccount {
            config_path: None, ..
        }
        | VesselCommand::ResolveStartAccount {
            config_path: None, ..
        } => true,
        VesselCommand::Voyage(request) => match &request.command {
            VoyageCommand::HostBrowser { operation } => operation.valid(),
            VoyageCommand::Snapshot
            | VoyageCommand::Decisions
            | VoyageCommand::Receipt { .. }
            | VoyageCommand::UploadImage { .. }
            | VoyageCommand::SubmitContent { .. }
            | VoyageCommand::Submit { .. }
            | VoyageCommand::Steer { .. }
            | VoyageCommand::Cancel { .. }
            | VoyageCommand::Respond { .. }
            | VoyageCommand::SetAccess { .. }
            | VoyageCommand::Rename { .. }
            | VoyageCommand::Archive { .. }
            | VoyageCommand::Delete { .. }
            | VoyageCommand::Clear { .. }
            | VoyageCommand::Compact { .. }
            | VoyageCommand::SetAccountInference { .. } => true,
            VoyageCommand::History { limit, .. } => (1..=128).contains(limit),
            VoyageCommand::MessageChunk { limit, .. }
            | VoyageCommand::RunOutput { limit, .. }
            | VoyageCommand::ReadArtifact { limit, .. } => (1..=65536).contains(limit),
            VoyageCommand::Events { limit, wait_ms, .. } => {
                (1..=128).contains(limit) && *wait_ms <= 10000
            }
            _ => false,
        },
        _ => false,
    }
}
pub(crate) async fn socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    if headers.get_all("sec-websocket-protocol").iter().count() != 1
        || headers
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            != Some(voyage_protocol::duplex::SUBPROTOCOL)
        || headers.contains_key("authorization")
        || headers.contains_key("x-voyage-grant")
        || headers.contains_key("x-voyage-vessel")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(origin) = request_origin(&headers) else {
        return StatusCode::FORBIDDEN.into_response();
    };
    let Ok(permit) = socket_capacity().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    upgrade
        .protocols([voyage_protocol::duplex::SUBPROTOCOL])
        .max_message_size(voyage_protocol::duplex::MAX_FRAME_BYTES)
        .max_frame_size(voyage_protocol::duplex::MAX_FRAME_BYTES)
        .on_upgrade(move |mut socket| async move {
            let first = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
            let Ok(Some(Ok(axum::extract::ws::Message::Text(text)))) = first else {
                return;
            };
            if text.len() > 256 {
                return;
            }
            let Ok(Authentication::Authenticate { token }) = serde_json::from_str(&text) else {
                return;
            };
            let Some(credential) = state.browser_credentials.take(&token, &origin) else {
                return;
            };
            let vessel_id = credential.backend.expected_vessel_id.unwrap();
            let backend = Arc::new(BrowserBackend(credential));
            if !tokio::time::timeout(Duration::from_secs(3), backend.authorize(None))
                .await
                .unwrap_or(false)
            {
                return;
            }
            vessel::duplex::serve(socket, backend, vessel_id, permit).await;
        })
}

#[cfg(test)]
mod tests;
