//! Authenticated public duplex transport. Disconnect detaches command work; it
//! never cancels a voyage or replays an uncertain command.
use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch};
use uuid::Uuid;
use voyage_protocol::{duplex::*, vessel::*};

pub type BackendFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
/// Adapters must use the ordinary public command authorization path. `authorize`
/// rechecks the original credential/revision and optional observation scope.
pub trait Backend: Send + Sync + 'static {
    /// Called after upgrade; implementations may associate the metadata-only
    /// notification handle with their browser routing state. Must not block.
    fn connected(&self, _connection: Connection) {}
    /// Lifecycle notification only: never use this to cancel admitted work.
    fn disconnected(&self, _socket_id: Uuid) {}
    fn command(&self, request: VesselRequest) -> BackendFuture<VesselResponse>;
    fn authorize(&self, session: Option<Uuid>) -> BackendFuture<bool>;
}
const IO_TIMEOUT: Duration = Duration::from_secs(3);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_CORRELATIONS: usize = 65536;
struct Outgoing {
    frame: ServerFrame,
    session: Option<Uuid>,
}
struct Reverse {
    request: ReverseRequest,
    result: oneshot::Sender<Option<ReverseReply>>,
}
/// A notification handle conveys no authority: publication is checked again by
/// the writer. Only BrowserWork metadata can cross this interface.
#[derive(Clone)]
pub struct Connection {
    pub socket_id: Uuid,
    tx: mpsc::Sender<Reverse>,
}
impl Connection {
    pub async fn browser_work(&self, session_id: Uuid, incarnation: Uuid) -> Option<ReverseReply> {
        if session_id.is_nil() || incarnation.is_nil() {
            return None;
        }
        let (result, rx) = oneshot::channel();
        self.tx
            .try_send(Reverse {
                request: ReverseRequest::BrowserWork {
                    session_id,
                    incarnation,
                },
                result,
            })
            .ok()?;
        tokio::time::timeout(Duration::from_secs(DEADLINE_SECONDS), rx)
            .await
            .ok()?
            .ok()
            .flatten()
    }
}
fn connections() -> &'static Mutex<HashMap<Uuid, Connection>> {
    static CONNECTIONS: OnceLock<Mutex<HashMap<Uuid, Connection>>> = OnceLock::new();
    CONNECTIONS.get_or_init(Default::default)
}
/// Lookup for server-side browser notification producers. No frames or secrets
/// are retained here; handles become unusable when the socket closes.
pub fn connection(socket_id: Uuid) -> Option<Connection> {
    connections().lock().ok()?.get(&socket_id).cloned()
}
struct Registration(Uuid, Arc<dyn Backend>);
impl Drop for Registration {
    fn drop(&mut self) {
        self.1.disconnected(self.0);
        if let Ok(mut map) = connections().lock() {
            map.remove(&self.0);
        }
    }
}

pub fn supports(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get_all("sec-websocket-protocol")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .any(|v| v.trim() == SUBPROTOCOL)
}
async fn authorized(backend: &Arc<dyn Backend>, session: Option<Uuid>) -> bool {
    tokio::time::timeout(IO_TIMEOUT, backend.authorize(session))
        .await
        .unwrap_or(false)
}
async fn enqueue(tx: &mpsc::Sender<Outgoing>, frame: ServerFrame, session: Option<Uuid>) -> bool {
    tokio::time::timeout(IO_TIMEOUT, tx.send(Outgoing { frame, session }))
        .await
        .is_ok_and(|v| v.is_ok())
}
fn unknown() -> VesselResponse {
    VesselResponse {
        protocol: VESSEL_API_VERSION,
        result: serde_json::Value::Null,
        error: Some(
            "Vessel command deadline exceeded; outcome unknown; do not replay automatically".into(),
        ),
        outcome_unknown: true,
    }
}

pub async fn serve(
    socket: WebSocket,
    backend: Arc<dyn Backend>,
    vessel_id: Uuid,
    permit: OwnedSemaphorePermit,
) {
    let permit = Arc::new(permit);
    let socket_id = Uuid::new_v4();
    let (reverse_tx, mut reverse_rx) = mpsc::channel::<Reverse>(MAX_IN_FLIGHT);
    let connection = Connection {
        socket_id,
        tx: reverse_tx,
    };
    if let Ok(mut map) = connections().lock() {
        map.insert(socket_id, connection.clone());
    }
    let _registration = Registration(socket_id, backend.clone());
    backend.connected(connection);
    let (tx, mut rx) = mpsc::channel::<Outgoing>(MAX_IN_FLIGHT);
    let (mut sink, mut source) = socket.split();
    let (control_tx, mut control_rx) = mpsc::channel::<Message>(4);
    let (closed_tx, mut closed_rx) = watch::channel(false);
    let failure = closed_tx.clone();
    let writer_backend = backend.clone();
    let writer = tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                Some(message) = control_rx.recv() => message,
                out = rx.recv() => {
                    let Some(out) = out else { break; };
                    if !authorized(&writer_backend, out.session).await { break; }
                    let Ok(text) = serde_json::to_string(&out.frame) else { break; };
                    if text.len() > MAX_FRAME_BYTES { break; }
                    Message::Text(text.into())
                }
            };
            if !tokio::time::timeout(IO_TIMEOUT, sink.send(message))
                .await
                .is_ok_and(|v| v.is_ok())
            {
                break;
            }
        }
        let _ = closed_tx.send(true);
        let _ = tokio::time::timeout(IO_TIMEOUT, sink.close()).await;
    });
    if !enqueue(
        &tx,
        ServerFrame::Hello {
            protocol: VESSEL_API_VERSION,
            socket_id,
            vessel_id,
        },
        None,
    )
    .await
    {
        writer.abort();
        return;
    }
    let commands = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let mut seen = HashSet::new();
    let mut subscriptions: HashMap<Uuid, Vec<tokio::task::AbortHandle>> = HashMap::new();
    let mut observations = tokio::task::JoinSet::new();
    let mut pending_reverse: HashMap<
        Uuid,
        (tokio::time::Instant, oneshot::Sender<Option<ReverseReply>>),
    > = HashMap::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Protocol ping/pong is handled by axum/tungstenite; application liveness is
    // enforced by incoming control traffic and periodic authority checks.
    let mut last_incoming = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = closed_rx.changed() => break,
            _ = heartbeat.tick() => {
                if last_incoming.elapsed() > Duration::from_secs(DEADLINE_SECONDS) || !authorized(&backend, None).await { break; }
                if control_tx.try_send(Message::Ping(Vec::new().into())).is_err() { break; }
                pending_reverse.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(DEADLINE_SECONDS));
            }
            Some(_) = observations.join_next(), if !observations.is_empty() => {}
            Some(reverse) = reverse_rx.recv() => {
                if pending_reverse.len() >= MAX_IN_FLIGHT { let _ = reverse.result.send(None); continue; }
                let request_id = Uuid::new_v4();
                let ReverseRequest::BrowserWork { session_id, .. } = reverse.request;
                if !authorized(&backend, Some(session_id)).await { let _ = reverse.result.send(None); continue; }
                pending_reverse.insert(request_id, (tokio::time::Instant::now(), reverse.result));
                if !enqueue(&tx, ServerFrame::ReverseRequest { request_id, request: reverse.request }, Some(session_id)).await { break; }
            }
            incoming = source.next() => {
                let Some(Ok(message)) = incoming else { break; };
                last_incoming = tokio::time::Instant::now();
                let text = match message {
                    Message::Text(text) if text.len() <= MAX_FRAME_BYTES => text,
                    Message::Ping(bytes) => { if control_tx.try_send(Message::Pong(bytes)).is_err() { break; } continue; },
                    Message::Pong(_) => continue,
                    _ => break,
                };
                let Ok(frame) = serde_json::from_str::<ClientFrame>(&text) else { break; };
                match frame {
                    ClientFrame::Command { request_id, request } => {
                        if request_id.is_nil() || request.protocol != VESSEL_API_VERSION || seen.len() >= MAX_CORRELATIONS || !seen.insert(request_id) { break; }
                        let Ok(command_permit) = commands.clone().try_acquire_owned() else { break; };
                        let backend = backend.clone(); let tx = tx.clone(); let lifetime = permit.clone(); let failure = failure.clone();
                        // Dropping a JoinHandle detaches it. It is deliberately NOT
                        // owned by the abort-on-disconnect observation JoinSet.
                        tokio::spawn(async move {
                            let (_command_permit, _lifetime) = (command_permit, lifetime);
                            if !authorized(&backend, None).await { let _ = failure.send(true); return; }
                            let response = tokio::time::timeout(COMMAND_TIMEOUT, backend.command(request)).await.unwrap_or_else(|_| unknown());
                            if !enqueue(&tx, ServerFrame::Reply { request_id, response }, None).await { let _ = failure.send(true); }
                        });
                    }
                    ClientFrame::Subscribe { request_id, request } => {
                        let count: usize = subscriptions.values().map(Vec::len).sum();
                        let mut sessions = HashSet::new();
                        if request_id.is_nil() || request.protocol != VESSEL_API_VERSION || request.subscriptions.is_empty()
                            || count + request.subscriptions.len() > MAX_SUBSCRIPTIONS || seen.len() >= MAX_CORRELATIONS || !seen.insert(request_id)
                            || request.subscriptions.iter().any(|s| s.session_id.is_nil() || s.incarnation.is_nil() || !sessions.insert(s.session_id)) { break; }
                        if !enqueue(&tx, ServerFrame::Subscribed { request_id }, None).await { break; }
                        let mut handles = Vec::new();
                        for subscription in request.subscriptions {
                            handles.push(observations.spawn(observe(backend.clone(), tx.clone(), request_id, subscription, failure.clone())));
                        }
                        subscriptions.insert(request_id, handles);
                    }
                    ClientFrame::Unsubscribe { subscription_id } => {
                        let Some(handles) = subscriptions.remove(&subscription_id) else { break; };
                        for handle in handles { handle.abort(); }
                    }
                    ClientFrame::ReverseReply { request_id, reply } => {
                        let Some((_, result)) = pending_reverse.remove(&request_id) else { break; };
                        let _ = result.send(Some(reply));
                    }
                }
            }
        }
    }
    observations.abort_all();
    writer.abort();
    let _ = writer.await;
}

async fn observe(
    backend: Arc<dyn Backend>,
    tx: mpsc::Sender<Outgoing>,
    id: Uuid,
    mut subscription: VesselEventSubscription,
    failure: watch::Sender<bool>,
) {
    loop {
        if !authorized(&backend, None).await {
            let _ = failure.send(true);
            return;
        }
        let response = tokio::time::timeout(
            IO_TIMEOUT,
            backend.command(VesselRequest {
                protocol: VESSEL_API_VERSION,
                command: VesselCommand::Voyage(VoyageRequest {
                    session_id: subscription.session_id,
                    incarnation: None,
                    command: VoyageCommand::Events {
                        after: subscription.after,
                        limit: 128,
                        wait_ms: 0,
                    },
                }),
            }),
        )
        .await
        .unwrap_or_else(|_| unknown());
        let mut event = VesselEvent {
            protocol: VESSEL_API_VERSION,
            session_id: subscription.session_id,
            incarnation: subscription.incarnation,
            result: response.result,
            error: response.error,
            outcome_unknown: response.outcome_unknown,
        };
        if event.error.is_none() {
            match serde_json::from_value::<VoyageReply>(event.result.clone()) {
                Ok(reply)
                    if reply.session_id == subscription.session_id
                        && !reply.incarnation.is_nil() =>
                {
                    event.result = reply.result;
                    event.incarnation = reply.incarnation;
                    if reply.incarnation != subscription.incarnation {
                        event.result["owner_changed"] = true.into();
                        event.result["replay_gap"] = true.into();
                        event.result["recovery"] = "snapshot".into();
                    }
                    subscription.incarnation = reply.incarnation;
                }
                _ => {
                    event.result = serde_json::Value::Null;
                    event.error = Some("invalid Vessel observation".into());
                }
            }
        }
        if let Some(cursor) = event
            .result
            .get("cursor")
            .and_then(serde_json::Value::as_u64)
        {
            subscription.after = cursor;
        }
        let terminal = event.error.is_some();
        // A refused/missing owner terminates only this observation. Its error
        // carries no result payload; current credential authority still gates
        // publication, but an absent registration need not kill other sessions.
        if terminal {
            event.result = serde_json::Value::Null;
        }
        let changed = terminal
            || event.result.get("replay_gap") == Some(&true.into())
            || event
                .result
                .get("events")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|v| !v.is_empty());
        if changed
            && !enqueue(
                &tx,
                ServerFrame::Event {
                    subscription_id: id,
                    event,
                },
                if terminal {
                    None
                } else {
                    Some(subscription.session_id)
                },
            )
            .await
        {
            let _ = failure.send(true);
            return;
        }
        if terminal {
            return;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
