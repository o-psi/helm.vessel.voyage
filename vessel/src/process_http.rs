//! Authenticated scoped process gateway; runtime execution policy stays local.
use super::*;
pub(super) mod browser;
use axum::response::sse::{Event, KeepAlive, Sse};
use voyage_protocol::vessel::{
    VESSEL_API_VERSION, VesselCommand, VesselEvent, VesselEventRequest, VesselRequest,
    VesselResponse, VoyageCommand, VoyageReply, VoyageRequest,
};

static CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);
static EVENT_CAPACITY: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(64);

impl ProcessRoute {
    async fn exchange(
        &self,
        auth: vessel::process::gateway_ipc::GrantAuth,
        command: VesselCommand,
    ) -> anyhow::Result<VesselResponse> {
        match self {
            Self::Local(directory) => {
                vessel::process::exchange(
                    directory,
                    &VesselRequest {
                        protocol: VESSEL_API_VERSION,
                        command: VesselCommand::Granted {
                            expected_authority_fingerprint: auth.expected_authority_fingerprint,
                            expected_vessel_id: auth.expected_vessel_id,
                            grant_id: auth.grant_id,
                            token: auth.token,
                            command: Box::new(command),
                        },
                    },
                )
                .await
            }
            #[cfg(target_os = "linux")]
            Self::System(name) => {
                vessel::process::gateway_ipc::exchange(
                    name,
                    &vessel::process::gateway_ipc::GatewayRequest::Command { auth, command },
                )
                .await
            }
        }
    }

    async fn pair_preflight(&self) -> anyhow::Result<serde_json::Value> {
        match self {
            Self::Local(directory) => vessel::process::pairing::preflight(directory),
            #[cfg(target_os = "linux")]
            Self::System(name) => {
                let response = vessel::process::gateway_ipc::exchange(
                    name,
                    &vessel::process::gateway_ipc::GatewayRequest::PairPreflight,
                )
                .await?;
                anyhow::ensure!(
                    response.error.is_none() && !response.outcome_unknown,
                    "system pairing preflight unavailable"
                );
                Ok(response.result)
            }
        }
    }

    async fn pair_redeem(
        &self,
        origin: &str,
        expected_vessel_id: Option<Uuid>,
        request: vessel::process::pairing::PairRequest,
    ) -> VesselResponse {
        match self {
            Self::Local(directory) => {
                match vessel::process::pairing::redeem(
                    directory,
                    origin,
                    expected_vessel_id,
                    request,
                ) {
                    Ok(credential) => VesselResponse {
                        protocol: VESSEL_API_VERSION,
                        result: serde_json::to_value(credential).unwrap_or(serde_json::Value::Null),
                        error: None,
                        outcome_unknown: false,
                    },
                    Err(error) => {
                        let refusal = error.downcast_ref::<vessel::process::pairing::PairRefusal>();
                        VesselResponse {
                            protocol: VESSEL_API_VERSION,
                            result: serde_json::Value::Null,
                            error: Some(refusal.map_or_else(
                                || {
                                    "pairing unavailable; retry only the identical pairing request"
                                        .into()
                                },
                                ToString::to_string,
                            )),
                            outcome_unknown: refusal.is_none(),
                        }
                    }
                }
            }
            #[cfg(target_os = "linux")]
            Self::System(name) => vessel::process::gateway_ipc::exchange(
                name,
                &vessel::process::gateway_ipc::GatewayRequest::PairRedeem {
                    expected_vessel_id,
                    request,
                },
            )
            .await
            .unwrap_or_else(|_| VesselResponse {
                protocol: VESSEL_API_VERSION,
                result: serde_json::Value::Null,
                error: Some("pairing unavailable; retry only the identical pairing request".into()),
                outcome_unknown: true,
            }),
        }
    }
}

pub(super) async fn boundary(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(origin) = &state.public_origin else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.headers().get_all("origin").iter().count() > 1
        || request
            .headers()
            .get("origin")
            .is_some_and(|supplied| supplied.to_str().ok() != Some(origin.as_str()))
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(_permit) = CAPACITY.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

pub(super) async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VesselEventRequest>,
) -> Response {
    let Some(route) = state.process_route else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if request.protocol != VESSEL_API_VERSION
        || request.subscriptions.is_empty()
        || request.subscriptions.len() > 32
        || request.subscriptions.iter().enumerate().any(|(i, item)| {
            item.session_id.is_nil()
                || request.subscriptions[..i]
                    .iter()
                    .any(|prior| prior.session_id == item.session_id)
        })
        || headers.get_all("authorization").iter().count() != 1
        || headers.get_all("x-voyage-grant").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let expected_vessel_id = match expected_vessel(&headers) {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    let Some(token) = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned)
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(grant_id) = headers
        .get("x-voyage-grant")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| Uuid::parse_str(value).ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Ok(permit) = EVENT_CAPACITY.try_acquire() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut subscriptions = request.subscriptions;
    let stream = async_stream::stream! {
            let _permit = permit;
            loop {
              for subscription in &mut subscriptions {
                let response = route.exchange(
                    vessel::process::gateway_ipc::GrantAuth {
                        expected_authority_fingerprint: None,
    expected_vessel_id,
                        grant_id,
                        token: token.clone(),
                    },
                    VesselCommand::Voyage(VoyageRequest {
                                session_id: subscription.session_id,
                                incarnation: None,
                                command: VoyageCommand::Events {
                                    after: subscription.after,
                                    limit: 128,
                                    // Re-enter the grant gateway between waits so expiry
                                    // and revocation stop publication at a bounded point.
                                    wait_ms: 0,
                                    projection: subscription.projection.clone(),
                                },
                            }),
                )
                .await;
                let (result, error, outcome_unknown) = match response {
                    Ok(response) if response.error.is_none() => {
                        match serde_json::from_value::<VoyageReply>(response.result) {
                            Ok(reply) => {
                                let mut result = reply.result;
                                if reply.incarnation != subscription.incarnation {
                                    result["owner_changed"] = serde_json::Value::Bool(true);
                                    result["replay_gap"] = serde_json::Value::Bool(true);
                                    result["recovery"] = serde_json::Value::String("snapshot".into());
                                    subscription.incarnation = reply.incarnation;
                                }
                                (result, None, false)
                            },
                            Err(_) => (serde_json::Value::Null, Some("invalid Vessel event response".into()), true),
                        }
                    }
                    Ok(response) => (response.result, response.error, response.outcome_unknown),
                    Err(_) => (serde_json::Value::Null, Some("Vessel routing unavailable; event stream ended".into()), true),
                };
                let terminal = error.is_some();
                if let Some(cursor) = result.get("cursor").and_then(serde_json::Value::as_u64) {
                    subscription.after = cursor;
                }
                let changed = terminal
                    || result.get("replay_gap") == Some(&serde_json::Value::Bool(true))
                    || result.get("events").and_then(serde_json::Value::as_array).is_some_and(|events| !events.is_empty());
                if changed {
                    let event = VesselEvent {
                        protocol: VESSEL_API_VERSION,
                        session_id: subscription.session_id,
                        incarnation: subscription.incarnation,
                        result,
                        error,
                        outcome_unknown,
                    };
                    let Ok(data) = serde_json::to_string(&event) else { return; };
                    yield Ok::<Event, std::convert::Infallible>(Event::default().event("update").data(data));
                }
                if terminal { return; }
              }
              tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        };
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(std::time::Duration::from_secs(5))
                .text("keep-alive"),
        )
        .into_response()
}

pub(super) async fn command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<VesselRequest>,
) -> Response {
    let Some(route) = state.process_route else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if private_envelope(&request.command) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if request.protocol != VESSEL_API_VERSION
        || headers.get_all("authorization").iter().count() != 1
        || headers.get_all("x-voyage-grant").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let expected_vessel_id = match expected_vessel(&headers) {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    let Some(token) = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|token| token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(grant_id) = headers
        .get("x-voyage-grant")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let response = match route
        .exchange(
            vessel::process::gateway_ipc::GrantAuth {
                expected_authority_fingerprint: None,
                expected_vessel_id,
                grant_id,
                token: token.to_owned(),
            },
            request.command,
        )
        .await
    {
        Ok(response) => response,
        Err(_) => VesselResponse {
            protocol: VESSEL_API_VERSION,
            result: serde_json::Value::Null,
            error: Some("Vessel routing unavailable; command outcome unknown".into()),
            outcome_unknown: true,
        },
    };
    Json(response).into_response()
}

fn expected_vessel(headers: &HeaderMap) -> Result<Option<Uuid>, StatusCode> {
    let count = headers.get_all("x-voyage-vessel").iter().count();
    if count == 0 {
        return Ok(None);
    }
    if count != 1 {
        return Err(StatusCode::BAD_REQUEST);
    }
    headers
        .get("x-voyage-vessel")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
        .filter(|v| !v.is_nil())
        .map(Some)
        .ok_or(StatusCode::BAD_REQUEST)
}

/// Public discovery exposes identity and protocol, never workspaces or secrets.
pub(super) async fn pair_capabilities(State(state): State<AppState>) -> Response {
    let Some(route) = state.process_route else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match route.pair_preflight().await {
        Ok(value) => Json(value).into_response(),
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(super) async fn pair(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<vessel::process::pairing::PairRequest>,
) -> Response {
    let Some(route) = state.process_route else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Some(origin) = state.public_origin else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let expected_vessel_id = match expected_vessel(&headers) {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    // Publication may have consumed the invitation. A lost reply must retain
    // the exact request identity rather than minting a replacement.
    let response = route
        .pair_redeem(&origin, expected_vessel_id, request)
        .await;
    Json(response).into_response()
}

/// The public socket uses the same scoped command adapter as HTTP. The initial
/// capability document freezes the accepted identity, principal, rights, scope
/// and revision; any change closes the connection rather than broadening it.
#[derive(Clone)]
struct SocketBackend {
    route: ProcessRoute,
    expected_vessel_id: Option<Uuid>,
    grant_id: Uuid,
    token: String,
    authority: serde_json::Value,
    browser_sockets: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<Uuid, std::sync::Arc<BrowserSocketState>>>,
    >,
}
impl SocketBackend {
    async fn exchange(&self, command: VesselCommand) -> VesselResponse {
        self.exchange_socket(command, None).await
    }
    fn expected_authority(&self) -> Result<Option<String>, ()> {
        if self.authority.is_null() {
            return Ok(None);
        }
        let advertised = self
            .authority
            .get("features")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|features| {
                features
                    .iter()
                    .any(|feature| feature == "saved_authority_pin")
            });
        match self.authority.get("authorization_fingerprint") {
            Some(serde_json::Value::String(value))
                if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                Ok(Some(value.clone()))
            }
            None if !advertised => Ok(None),
            _ => Err(()),
        }
    }
    async fn exchange_socket(
        &self,
        command: VesselCommand,
        socket: Option<(Uuid, std::sync::Arc<BrowserSocketState>)>,
    ) -> VesselResponse {
        if private_envelope(&command) {
            return VesselResponse {
                protocol: VESSEL_API_VERSION,
                result: serde_json::Value::Null,
                error: Some("private envelope refused".into()),
                outcome_unknown: false,
            };
        }
        let Ok(expected_authority_fingerprint) = self.expected_authority() else {
            return browser_refusal();
        };
        let auth = vessel::process::gateway_ipc::GrantAuth {
            expected_authority_fingerprint,
            expected_vessel_id: self.expected_vessel_id,
            grant_id: self.grant_id,
            token: self.token.clone(),
        };
        let result = match &self.route {
            ProcessRoute::Local(directory) => {
                let command = VesselCommand::Granted {
                    expected_authority_fingerprint: auth.expected_authority_fingerprint,
                    expected_vessel_id: auth.expected_vessel_id,
                    grant_id: auth.grant_id,
                    token: auth.token,
                    command: Box::new(command),
                };
                let command = match socket {
                    Some((socket_id, _)) => VesselCommand::Socket {
                        socket: voyage_protocol::host_browser::HostBrowserSocket { socket_id },
                        command: Box::new(command),
                    },
                    None => command,
                };
                vessel::process::exchange(
                    directory,
                    &VesselRequest {
                        protocol: VESSEL_API_VERSION,
                        command,
                    },
                )
                .await
            }
            #[cfg(target_os = "linux")]
            ProcessRoute::System(name) => {
                if let Some((_, state)) = socket {
                    let mut channel = state.system.lock().await;
                    if state.closed.load(std::sync::atomic::Ordering::SeqCst) {
                        return browser_refusal();
                    }
                    if channel.is_none() {
                        match vessel::process::gateway_ipc::open_socket(name, auth.clone()).await {
                            Ok(opened) => *channel = Some(opened),
                            Err(error) => return routing_failure(error),
                        }
                    }
                    let (stream, _) = channel.as_mut().expect("opened socket");
                    let request = vessel::process::gateway_ipc::GatewayRequest::SocketCommand {
                        auth,
                        command,
                    };
                    let response = async {
                        vessel::process::gateway_ipc::write_frame(stream, &request).await?;
                        vessel::process::gateway_ipc::read_frame(stream)
                            .await?
                            .ok_or_else(|| anyhow::anyhow!("supervisor socket closed"))
                    }
                    .await;
                    if response.is_err() {
                        *channel = None;
                    }
                    response
                } else {
                    self.route.exchange(auth, command).await
                }
            }
        };
        result.unwrap_or_else(|_| VesselResponse {
            protocol: VESSEL_API_VERSION,
            result: serde_json::Value::Null,
            error: Some("Vessel routing unavailable; command outcome unknown".into()),
            outcome_unknown: true,
        })
    }
}
// Discovery metadata is dynamic, not an authority version. The authenticated
// grant revision/identity/expiry/rights still invalidate the socket on change.
fn socket_authority(mut capabilities: serde_json::Value) -> serde_json::Value {
    if capabilities
        .get("scope")
        .and_then(serde_json::Value::as_str)
        == Some("owner")
        && let Some(object) = capabilities.as_object_mut()
    {
        object.remove("workspaces");
    }
    capabilities
}

impl vessel::duplex::Backend for SocketBackend {
    fn connected(&self, connection: vessel::duplex::Connection) {
        if let Ok(mut states) = self.browser_sockets.lock() {
            states.insert(connection.socket_id, Default::default());
        }
    }
    fn disconnected(&self, socket_id: Uuid) {
        let state = self
            .browser_sockets
            .lock()
            .ok()
            .and_then(|mut states| states.remove(&socket_id));
        let Some(state) = state else {
            return;
        };
        state
            .closed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let route = self.route.clone();
        tokio::spawn(async move {
            #[cfg(target_os = "linux")]
            if matches!(route, ProcessRoute::System(_)) {
                // Dropping the persistent authenticated pipe is the root-side
                // controller disconnect signal, even if the gateway dies.
                let mut channel = state.system.lock().await;
                *channel = None;
                return;
            }
            let ProcessRoute::Local(directory) = route else {
                return;
            };
            // Snapshot targets without waiting for in-flight effects. Runtime tombstones
            // refuse a late command even if this notification reaches it first.
            let sessions = state.sessions.lock().await.clone();
            for &(session_id, incarnation) in sessions.iter() {
                let request = VesselRequest {
                    protocol: VESSEL_API_VERSION,
                    command: VesselCommand::HostBrowserDisconnected {
                        session_id,
                        incarnation,
                        socket: voyage_protocol::host_browser::HostBrowserSocket { socket_id },
                    },
                };
                // Cleanup is idempotent; no user effect is replayed. Failure is
                // not evidence of cleanup; runtime must also fence dead owners.
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(20),
                    vessel::process::exchange(&directory, &request),
                )
                .await;
            }
        });
    }
    fn socket_command(
        &self,
        request: VesselRequest,
        socket_id: Uuid,
    ) -> vessel::duplex::BackendFuture<VesselResponse> {
        let backend = self.clone();
        let state = self
            .browser_sockets
            .lock()
            .ok()
            .and_then(|states| states.get(&socket_id).cloned());
        Box::pin(async move {
            if request.protocol != VESSEL_API_VERSION {
                return browser_refusal();
            }
            if !matches!(
                &request.command,
                VesselCommand::Voyage(VoyageRequest {
                    command: VoyageCommand::HostBrowser { .. },
                    ..
                })
            ) {
                return backend.exchange(request.command).await;
            }
            let Some(state) = state else {
                return browser_refusal();
            };
            let mut sessions = state.sessions.lock().await;
            if state.closed.load(std::sync::atomic::Ordering::SeqCst) {
                return browser_refusal();
            }
            if let VesselCommand::Voyage(VoyageRequest {
                session_id,
                incarnation,
                command: VoyageCommand::HostBrowser { operation },
            }) = &request.command
            {
                let Some(incarnation) = incarnation.filter(|id| !id.is_nil()) else {
                    return browser_refusal();
                };
                if !operation.valid() || session_id.is_nil() {
                    return browser_refusal();
                }
                if sessions.len() >= 32 && !sessions.contains(&(*session_id, incarnation)) {
                    return browser_refusal();
                }
                sessions.insert((*session_id, incarnation));
            }
            drop(sessions);
            backend
                .exchange_socket(request.command, Some((socket_id, state)))
                .await
        })
    }
    fn command(&self, request: VesselRequest) -> vessel::duplex::BackendFuture<VesselResponse> {
        let backend = self.clone();
        Box::pin(async move { backend.exchange(request.command).await })
    }
    fn authorize(&self, session: Option<Uuid>) -> vessel::duplex::BackendFuture<bool> {
        let backend = self.clone();
        Box::pin(async move {
            let current = backend.exchange(VesselCommand::Capabilities).await;
            if current.error.is_some()
                || current.outcome_unknown
                || socket_authority(current.result) != backend.authority
            {
                return false;
            }
            if let Some(session_id) = session {
                let scope = backend
                    .exchange(VesselCommand::Inspect { session_id })
                    .await;
                if scope.error.is_some() || scope.outcome_unknown {
                    return false;
                }
            }
            true
        })
    }
}

pub(super) async fn socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    upgrade: axum::extract::ws::WebSocketUpgrade,
) -> Response {
    use vessel::duplex::Backend;

    let Some(route) = state.process_route else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    if !vessel::duplex::supports(&headers)
        || headers.get_all("authorization").iter().count() != 1
        || headers.get_all("x-voyage-grant").iter().count() != 1
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    // Legacy session credentials predate the identity field. Authenticate them
    // first, then pin the observed identity for the entire socket. Workspace
    // credentials still require their persisted pin in the grant gateway.
    let expected_vessel_id = match expected_vessel(&headers) {
        Ok(id) => id,
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
    let Ok(permit) = socket_capacity().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let mut backend = SocketBackend {
        route,
        expected_vessel_id,
        grant_id,
        token: token.into(),
        authority: serde_json::Value::Null,
        browser_sockets: Default::default(),
    };
    let initial = match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        backend.exchange(VesselCommand::Capabilities),
    )
    .await
    {
        Ok(response) if response.error.is_none() && !response.outcome_unknown => response,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let Some(vessel_id) = initial
        .result
        .get("vessel_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
        .filter(|id| !id.is_nil())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if expected_vessel_id.is_some_and(|expected| expected != vessel_id) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    backend.expected_vessel_id = Some(vessel_id);
    backend.authority = socket_authority(initial.result);
    if !tokio::time::timeout(std::time::Duration::from_secs(3), backend.authorize(None))
        .await
        .unwrap_or(false)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let backend = std::sync::Arc::new(backend);
    upgrade
        .protocols([voyage_protocol::duplex::SUBPROTOCOL])
        .max_message_size(voyage_protocol::duplex::MAX_FRAME_BYTES)
        .max_frame_size(voyage_protocol::duplex::MAX_FRAME_BYTES)
        .on_upgrade(move |socket| vessel::duplex::serve(socket, backend, vessel_id, permit))
}

#[cfg(test)]
mod tests;

fn socket_capacity() -> std::sync::Arc<tokio::sync::Semaphore> {
    static CAPACITY: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
        std::sync::OnceLock::new();
    CAPACITY
        .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(64)))
        .clone()
}

#[derive(Default)]
struct BrowserSocketState {
    closed: std::sync::atomic::AtomicBool,
    sessions: tokio::sync::Mutex<std::collections::HashSet<(Uuid, Uuid)>>,
    #[cfg(target_os = "linux")]
    system: tokio::sync::Mutex<Option<(tokio::net::UnixStream, Uuid)>>,
}

fn routing_failure(_error: anyhow::Error) -> VesselResponse {
    VesselResponse {
        protocol: VESSEL_API_VERSION,
        result: serde_json::Value::Null,
        error: Some("Vessel routing unavailable; command outcome unknown".into()),
        outcome_unknown: true,
    }
}
fn private_envelope(command: &VesselCommand) -> bool {
    matches!(
        command,
        VesselCommand::Socket { .. }
            | VesselCommand::Granted { .. }
            | VesselCommand::HostBrowserDisconnected { .. }
    )
}
fn browser_refusal() -> VesselResponse {
    VesselResponse {
        protocol: VESSEL_API_VERSION,
        result: serde_json::Value::Null,
        error: Some("invalid or closed browser socket".into()),
        outcome_unknown: false,
    }
}

#[cfg(test)]
#[path = "process_http/owned_frontdoor_tests.rs"]
mod owned_frontdoor_tests;
