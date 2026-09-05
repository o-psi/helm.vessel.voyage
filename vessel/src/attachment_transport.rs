//! Authenticated bounded observation transport. Not mounted by main. Receipt of
//! a frame never grants execution authority or proves durable command acceptance.
use crate::enrollment_http::EnrollmentApi;
use axum::{
    Router,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::get,
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::{Mutex, Semaphore, mpsc, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::{attachment::MAX_FRAME_BYTES, events::Features, stream::Frame};

const PROTOCOL: &str = "voyage.attachment.v2";
/// Fixed errors deliberately exclude proofs, frames and network/storage details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    Invalid,
    Unavailable,
    Stale,
    Full,
}
impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "attachment transport {:?}", self)
    }
}
impl std::error::Error for TransportError {}
type Result<T> = std::result::Result<T, TransportError>;
/// Observation only. Recheck is_current immediately before publishing queued
/// data; downstream sharing policy must also be checked at disclosure commit.
pub struct AuthenticatedFrame {
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
    pub connection_id: Uuid,
    pub frame: Frame,
}
#[derive(Clone, Copy)]
struct Limits {
    auth: Duration,
    lease: Duration,
    check: Duration,
    write: Duration,
    sockets: usize,
    machines: usize,
    queue: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            auth: Duration::from_secs(5),
            lease: Duration::from_secs(10),
            check: Duration::from_secs(1),
            write: Duration::from_secs(2),
            sockets: 32,
            machines: 16,
            queue: 16,
        }
    }
}
#[derive(Clone)]
struct Connection {
    id: Uuid,
    owner: Uuid,
    epoch: u64,
    features: Features,
    cancel: CancellationToken,
    send: mpsc::Sender<Frame>,
    heartbeat: watch::Sender<tokio::time::Instant>,
}
#[derive(Clone)]
pub struct AttachmentApi {
    enrollment: EnrollmentApi,
    supported: Features,
    registry: Arc<Mutex<HashMap<Uuid, Connection>>>,
    sockets: Arc<Semaphore>,
    observations: mpsc::Sender<AuthenticatedFrame>,
    limits: Limits,
}
impl AttachmentApi {
    pub fn new(
        enrollment: EnrollmentApi,
        supported: Features,
    ) -> Result<(Self, mpsc::Receiver<AuthenticatedFrame>)> {
        Self::with_limits(enrollment, supported, Limits::default())
    }
    fn with_limits(
        enrollment: EnrollmentApi,
        supported: Features,
        limits: Limits,
    ) -> Result<(Self, mpsc::Receiver<AuthenticatedFrame>)> {
        if !(Duration::from_millis(1000)..=Duration::from_millis(30000)).contains(&limits.lease)
            || limits.auth.is_zero()
            || limits.write.is_zero()
            || limits.check.is_zero()
            || limits.sockets == 0
            || limits.machines == 0
            || limits.queue == 0
        {
            return Err(TransportError::Invalid);
        }
        supported.validate().map_err(|_| TransportError::Invalid)?;
        let (observations, rx) = mpsc::channel(limits.queue);
        Ok((
            Self {
                enrollment,
                supported,
                registry: Default::default(),
                sockets: Arc::new(Semaphore::new(limits.sockets)),
                observations,
                limits,
            },
            rx,
        ))
    }
    pub fn router(self) -> Router {
        Router::new()
            .route("/v2/attachment", get(upgrade))
            .with_state(self)
    }
    /// Queued transport delivery only. No local dispatch, admission or policy grant.
    pub async fn send(&self, machine: Uuid, frame: Frame) -> Result<()> {
        let connection = self
            .registry
            .lock()
            .await
            .get(&machine)
            .cloned()
            .ok_or(TransportError::Stale)?;
        outbound(&frame, machine, connection.id, connection.owner)?;
        frame
            .validate_features(&connection.features)
            .map_err(|_| TransportError::Invalid)?;
        frame.encode().map_err(|_| TransportError::Invalid)?;
        if !self.is_current(machine, connection.id).await {
            return Err(TransportError::Stale);
        }
        let registry = self.registry.lock().await;
        if registry
            .get(&machine)
            .is_none_or(|c| c.id != connection.id || c.cancel.is_cancelled())
        {
            return Err(TransportError::Stale);
        }
        connection
            .send
            .try_send(frame)
            .map_err(|_| TransportError::Full)
    }
    pub async fn is_current(&self, machine: Uuid, connection_id: Uuid) -> bool {
        let Some(connection) = self.registry.lock().await.get(&machine).cloned() else {
            return false;
        };
        if connection.id != connection_id || connection.cancel.is_cancelled() {
            return false;
        }
        let valid = matches!(tokio::time::timeout(self.limits.check,self.enrollment.attachment_current(machine,connection.epoch)).await,Ok(Ok(receipt)) if receipt.owner_id==connection.owner);
        if !valid {
            connection.cancel.cancel();
            return false;
        }
        self.registry
            .lock()
            .await
            .get(&machine)
            .is_some_and(|c| c.id == connection_id && !c.cancel.is_cancelled())
    }
    async fn socket(&self, mut socket: WebSocket) {
        let authentication = async {
            let text = loop {
                match socket.recv().await {
                    Some(Ok(Message::Text(text))) => break text,
                    Some(Ok(Message::Ping(bytes))) => {
                        bounded_write(
                            &CancellationToken::new(),
                            self.limits.write,
                            socket.send(Message::Pong(bytes)),
                        )
                        .await?;
                    }
                    Some(Ok(Message::Pong(_))) => (),
                    _ => return Err(TransportError::Invalid),
                }
            };
            let Frame::Authenticate {
                proof, features, ..
            } = Frame::decode(text.as_bytes()).map_err(|_| TransportError::Invalid)?
            else {
                return Err(TransportError::Invalid);
            };
            let selected = features
                .negotiate(&self.supported)
                .map_err(|_| TransportError::Invalid)?;
            let receipt = self
                .enrollment
                .attachment_connect(proof)
                .await
                .map_err(|_| TransportError::Invalid)?;
            Ok((receipt, selected))
        };
        let Ok(Ok((receipt, features))) =
            tokio::time::timeout(self.limits.auth, authentication).await
        else {
            return;
        };
        let (sender, mut receiver) = mpsc::channel(self.limits.queue);
        let connection = Connection {
            id: Uuid::new_v4(),
            owner: receipt.owner_id,
            epoch: receipt.epoch,
            features: features.clone(),
            cancel: CancellationToken::new(),
            send: sender,
            heartbeat: watch::channel(tokio::time::Instant::now()).0,
        };
        {
            let mut registry = self.registry.lock().await;
            if !registry.contains_key(&receipt.machine_id) && registry.len() >= self.limits.machines
            {
                return;
            }
            if let Some(old) = registry.insert(receipt.machine_id, connection.clone()) {
                old.cancel.cancel();
            }
        }
        // Watch authorization and the lease independently of socket writes. A
        // blocked peer must not defer revocation checks until a write completes.
        let monitor_api = self.clone();
        let monitor_connection = connection.clone();
        let monitor_machine = receipt.machine_id;
        let _monitor = tokio_util::task::AbortOnDropHandle::new(tokio::spawn(async move {
            let mut tick = tokio::time::interval(monitor_api.limits.check);
            let mut heartbeat = monitor_connection.heartbeat.subscribe();
            loop {
                let deadline = *heartbeat.borrow() + monitor_api.limits.lease;
                tokio::select! { biased;
                    _ = monitor_connection.cancel.cancelled() => return,
                    _ = tokio::time::sleep_until(deadline) => {monitor_connection.cancel.cancel(); return;},
                    changed = heartbeat.changed() => {if changed.is_err() {return;}},
                    _ = tick.tick() => {
                        tokio::select! { biased;
                            _ = tokio::time::sleep_until(deadline) => {monitor_connection.cancel.cancel(); return;},
                            valid = monitor_api.is_current(monitor_machine, monitor_connection.id) => {if !valid {return;}}
                        }
                    }
                }
            }
        }));
        // Cleanup is generation-specific. A terminating old socket cannot remove
        // its replacement. All exits after registration run this common cleanup.
        self.connected(&mut socket, &mut receiver, receipt.machine_id, &connection)
            .await;
        connection.cancel.cancel();
        let mut registry = self.registry.lock().await;
        if registry
            .get(&receipt.machine_id)
            .is_some_and(|c| c.id == connection.id)
        {
            registry.remove(&receipt.machine_id);
        }
    }
    async fn write(
        &self,
        socket: &mut WebSocket,
        connection: &Connection,
        frame: Frame,
    ) -> Result<()> {
        let text = frame.encode().map_err(|_| TransportError::Invalid)?;
        bounded_write(
            &connection.cancel,
            self.limits.write,
            socket.send(Message::Text(text.into())),
        )
        .await
    }

    async fn connected(
        &self,
        socket: &mut WebSocket,
        receiver: &mut mpsc::Receiver<Frame>,
        machine: Uuid,
        connection: &Connection,
    ) {
        if !self.is_current(machine, connection.id).await {
            return;
        }
        if self
            .write(
                socket,
                connection,
                Frame::Welcome {
                    version: 2,
                    connection_id: connection.id,
                    machine_id: machine,
                    owner_id: connection.owner,
                    epoch: connection.epoch,
                    lease_ms: self.limits.lease.as_millis() as u32,
                    features: connection.features.clone(),
                },
            )
            .await
            .is_err()
        {
            return;
        }
        let mut heartbeat = tokio::time::Instant::now();
        let mut check = tokio::time::interval(self.limits.check);
        loop {
            tokio::select! {
                _=connection.cancel.cancelled()=>return,
                _=tokio::time::sleep_until(heartbeat+self.limits.lease)=>return,
                _=check.tick()=>{if !self.is_current(machine,connection.id).await{return;}},
                incoming=socket.recv()=>{
                    let text=match incoming {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Ping(bytes))) => {
                            if !self.is_current(machine,connection.id).await || bounded_write(&connection.cancel,self.limits.write,socket.send(Message::Pong(bytes))).await.is_err(){return;}
                            continue;
                        }
                        Some(Ok(Message::Pong(_))) => continue,
                        _=>return,
                    };
                    let Ok(frame)=Frame::decode(text.as_bytes()) else{return;};
                    if inbound(&frame,connection.id).is_err()||frame.validate_features(&connection.features).is_err()||!self.is_current(machine,connection.id).await{return;}
                    if matches!(frame,Frame::Heartbeat{..}) {
                        heartbeat=tokio::time::Instant::now();
                        connection.heartbeat.send_replace(heartbeat);
                        if self.write(socket,connection,Frame::Lease{connection_id:connection.id,lease_ms:self.limits.lease.as_millis() as u32}).await.is_err(){return;}
                    } else {
                        let registry=self.registry.lock().await;
                        if registry.get(&machine).is_none_or(|c|c.id!=connection.id||c.cancel.is_cancelled()){return;}
                        if self.observations.try_send(AuthenticatedFrame{machine_id:machine,owner_id:connection.owner,epoch:connection.epoch,connection_id:connection.id,frame}).is_err(){return;}
                    }
                },
                outgoing=receiver.recv()=>{
                    let Some(frame)=outgoing else{return;};
                    if !self.is_current(machine,connection.id).await||outbound(&frame,machine,connection.id,connection.owner).is_err()||frame.validate_features(&connection.features).is_err(){return;}
                    if self.write(socket,connection,frame).await.is_err(){return;}
                }
            }
        }
    }
}
/// Kept generic so write-deadline and cancellation failures are deterministic
/// without relying on a particular OS TCP send-buffer size.
async fn bounded_write<E>(
    cancel: &CancellationToken,
    deadline: Duration,
    write: impl std::future::Future<Output = std::result::Result<(), E>>,
) -> Result<()> {
    tokio::select! { biased;
        _ = cancel.cancelled() => Err(TransportError::Stale),
        result = tokio::time::timeout(deadline, write) => match result {
            Ok(Ok(())) => Ok(()),
            _ => Err(TransportError::Unavailable)
        }
    }
}
fn inbound(frame: &Frame, connection: Uuid) -> Result<()> {
    let id = match frame {
        Frame::Result { connection_id, .. }
        | Frame::Event { connection_id, .. }
        | Frame::Replay { connection_id, .. }
        | Frame::SnapshotRequired { connection_id, .. }
        | Frame::Heartbeat { connection_id } => *connection_id,
        _ => return Err(TransportError::Invalid),
    };
    if id != connection {
        Err(TransportError::Stale)
    } else {
        Ok(())
    }
}
fn outbound(frame: &Frame, machine: Uuid, connection: Uuid, owner: Uuid) -> Result<()> {
    match frame {
        Frame::Command { command }
            if command.connection_id == connection
                && command.machine_id == machine
                && command.principal_id == owner =>
        {
            Ok(())
        }
        Frame::ReplayRequest { connection_id, .. } if *connection_id == connection => Ok(()),
        _ => Err(TransportError::Invalid),
    }
}
async fn upgrade(
    State(api): State<AttachmentApi>,
    uri: Uri,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let exact = |name: &str, value: &str| {
        headers.get_all(name).iter().count() == 1
            && headers.get(name).and_then(|v| v.to_str().ok()) == Some(value)
    };
    if uri.query().is_some()
        || uri.authority().is_some()
        || !exact("origin", api.enrollment.attachment_origin())
        || !exact("x-voyage-request", "2")
        || !exact("sec-websocket-protocol", PROTOCOL)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(permit) = api.sockets.clone().try_acquire_owned() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    ws.protocols([PROTOCOL])
        .max_frame_size(MAX_FRAME_BYTES)
        .max_message_size(MAX_FRAME_BYTES)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_FRAME_BYTES + 1024)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            api.socket(socket).await
        })
}
#[cfg(test)]
mod tests;
