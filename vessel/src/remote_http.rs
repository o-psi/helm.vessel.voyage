//! Opt-in authenticated operator relay. No provider credentials, runtime grants, or transcripts.
use super::*;
use axum::extract::{Path, Query};
use std::{collections::HashMap, time::Duration};
use tokio::sync::oneshot;
use vessel::attachment_transport::{AttachmentApi, AuthenticatedFrame, ConnectionPresence};
use voyage_protocol::{
    attachment::{Command, Operation, VERSION},
    events::{EventCursor, Feature, Features},
    stream::Frame,
};

#[derive(Clone)]
pub(super) struct RemoteApi {
    attachment: AttachmentApi,
    origin: String,
    pending: Arc<Mutex<HashMap<(Uuid, Uuid, Uuid), oneshot::Sender<Frame>>>>,
}
struct Pending {
    key: (Uuid, Uuid, Uuid),
    pending: Arc<Mutex<HashMap<(Uuid, Uuid, Uuid), oneshot::Sender<Frame>>>>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&self.key);
        }
    }
}
impl RemoteApi {
    pub fn new(enrollment: vessel::enrollment_http::EnrollmentApi, origin: String) -> Result<Self> {
        let features = Features::new(vec![
            Feature::SequencedEvents,
            Feature::Replay,
            Feature::ToolActivity,
            Feature::Usage,
            Feature::ManagedExecution,
        ])
        .map_err(anyhow::Error::msg)?;
        let (attachment, mut incoming) = AttachmentApi::new(enrollment, features)?;
        let this = Self {
            attachment,
            origin,
            pending: Arc::new(Mutex::new(HashMap::new())),
        };
        let monitor = this.clone();
        tokio::spawn(async move {
            loop {
                let received = tokio::select! {
                    value = incoming.recv() => value,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        if monitor.attachment.is_shutdown() { break; }
                        continue;
                    }
                };
                let Some(AuthenticatedFrame {
                    machine_id,
                    connection_id,
                    frame,
                    ..
                }) = received
                else {
                    break;
                };
                if !monitor
                    .attachment
                    .is_current(machine_id, connection_id)
                    .await
                {
                    continue;
                }
                let request = match &frame {
                    Frame::Result { command_id, .. } => *command_id,
                    Frame::Replay { request_id, .. }
                    | Frame::SnapshotRequired { request_id, .. } => *request_id,
                    // Live text is never cached by Vessel. Clients explicitly request bounded
                    // replay pages from Helm's transactional public cursor.
                    _ => continue,
                };
                let sender = monitor
                    .pending
                    .lock()
                    .ok()
                    .and_then(|mut p| p.remove(&(machine_id, connection_id, request)));
                if let Some(sender) = sender {
                    let _ = sender.send(frame);
                }
            }
        });
        Ok(this)
    }
    pub fn attachment(&self) -> AttachmentApi {
        self.attachment.clone()
    }
    async fn connection(&self, machine: Uuid) -> UiResult<ConnectionPresence> {
        self.attachment
            .connections()
            .await
            .into_iter()
            .find(|c| c.machine_id == machine)
            .ok_or_else(|| {
                ui_error((
                    StatusCode::NOT_FOUND,
                    "Current remote connection unavailable",
                ))
            })
    }
    async fn exchange(
        &self,
        current: &ConnectionPresence,
        id: Uuid,
        frame: Frame,
    ) -> UiResult<Frame> {
        let (sender, receiver) = oneshot::channel();
        let key = (current.machine_id, current.connection_id, id);
        {
            let mut pending = self
                .pending
                .lock()
                .map_err(|_| ui_error(StatusCode::SERVICE_UNAVAILABLE))?;
            if pending.len() >= 64 || pending.contains_key(&key) {
                return Err(ui_error((StatusCode::CONFLICT, "Remote request busy")));
            }
            pending.insert(key, sender);
        }
        let _pending = Pending {
            key,
            pending: self.pending.clone(),
        };
        self.attachment
            .send(current.machine_id, frame)
            .await
            .map_err(|_| ui_error((StatusCode::CONFLICT, "Remote connection unavailable")))?;
        let frame = tokio::time::timeout(Duration::from_secs(5), receiver)
            .await
            .map_err(|_| {
                ui_error((
                    StatusCode::GATEWAY_TIMEOUT,
                    "Remote outcome unconfirmed; retry the same command ID and deadline",
                ))
            })?
            .map_err(|_| ui_error(StatusCode::SERVICE_UNAVAILABLE))?;
        if !self
            .attachment
            .is_current(current.machine_id, current.connection_id)
            .await
        {
            return Err(ui_error((
                StatusCode::CONFLICT,
                "Remote connection changed; observe with a fresh connection",
            )));
        }
        Ok(frame)
    }
}
fn authorize<'a>(state: &'a AppState, headers: &HeaderMap) -> UiResult<&'a RemoteApi> {
    operator_auth(state, headers)?;
    let api = state
        .remote
        .as_ref()
        .ok_or_else(|| ui_error(StatusCode::NOT_FOUND))?;
    if headers.get_all("origin").iter().count() > 1
        || headers
            .get("origin")
            .is_some_and(|v| v.to_str().ok() != Some(api.origin.as_str()))
        || headers
            .get("sec-fetch-site")
            .is_some_and(|v| v != "same-origin" && v != "none")
    {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    Ok(api)
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    command_id: Uuid,
    expires_at_ms: i64,
    operation: Operation,
}
pub(super) async fn command(
    State(state): State<AppState>,
    Path(machine): Path<Uuid>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> UiResult<Response> {
    let api = authorize(&state, &headers)?;
    let request: Request = serde_json::from_slice(&body)
        .map_err(|_| ui_error((StatusCode::BAD_REQUEST, "Invalid remote request")))?;
    if headers.get("x-voyage-request").is_none_or(|v| v != "2") {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    if !matches!(
        request.operation,
        Operation::List { .. }
            | Operation::Inspect { .. }
            | Operation::Submit { .. }
            | Operation::Cancel { .. }
    ) {
        return Err(ui_error(StatusCode::FORBIDDEN));
    }
    let current = api.connection(machine).await?;
    let command = Command {
        version: VERSION,
        connection_id: current.connection_id,
        machine_id: machine,
        principal_id: current.owner_id,
        command_id: request.command_id,
        expires_at_ms: request.expires_at_ms,
        operation: request.operation,
    };
    command
        .validate_structure()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    let frame = api
        .exchange(&current, request.command_id, Frame::Command { command })
        .await?;
    if !matches!(frame, Frame::Result { .. }) {
        return Err(ui_error(StatusCode::BAD_GATEWAY));
    }
    Ok(private_response(frame))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Watch {
    session_id: Uuid,
    after: u64,
    #[serde(default = "default_limit")]
    limit: u16,
}
fn default_limit() -> u16 {
    32
}
pub(super) async fn watch(
    State(state): State<AppState>,
    Path(machine): Path<Uuid>,
    headers: HeaderMap,
    Query(request): Query<Watch>,
) -> UiResult<Response> {
    let api = authorize(&state, &headers)?;
    let current = api.connection(machine).await?;
    let request_id = Uuid::new_v4();
    let after = EventCursor::new(request.after).map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    let frame = Frame::ReplayRequest {
        connection_id: current.connection_id,
        request_id,
        session_id: request.session_id,
        after,
        limit: request.limit,
    };
    frame
        .encode()
        .map_err(|_| ui_error(StatusCode::BAD_REQUEST))?;
    let result = api.exchange(&current, request_id, frame).await?;
    if !matches!(&result,Frame::Replay{session_id,after:received,..}|Frame::SnapshotRequired{session_id,after:received,..} if *session_id==request.session_id && *received==after)
    {
        return Err(ui_error(StatusCode::BAD_GATEWAY));
    }
    Ok(private_response(result))
}

fn private_response(frame: Frame) -> Response {
    let mut response = Json(frame).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}
