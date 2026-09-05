//! Outbound connection transport. Frames are observations/requests, never execution grants.
use super::client::{EnrollmentClient, validate_origin};
use futures_util::{SinkExt, StreamExt};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle, time::Instant};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::{
    attachment::{MAX_FRAME_BYTES, VERSION},
    events::Features,
    stream::Frame,
};

const PROTOCOL: &str = "voyage.attachment.v2";
const QUEUE: usize = 8;
const WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TransportError {
    #[error("invalid attachment transport input")]
    Invalid,
    #[error("attachment enrollment unavailable")]
    Enrollment,
    #[error("attachment transport unavailable")]
    Network,
    #[error("attachment transport timed out")]
    Timeout,
    #[error("attachment connection closed")]
    Closed,
    #[error("attachment transport queue full")]
    Busy,
}
type Result<T> = std::result::Result<T, TransportError>;

#[derive(Clone)]
pub struct ConnectionContext {
    pub connection_id: Uuid,
    pub machine_id: Uuid,
    pub owner_id: Uuid,
    pub epoch: u64,
    pub features: Features,
}

/// Owns enrollment's exclusive file lease for the connection lifetime. A received
/// request is not admitted work: callers must recheck this lease, current sharing,
/// local policy and durable command admission at effect commitment.
pub struct Connection {
    client: Arc<EnrollmentClient>,
    context: ConnectionContext,
    closed: CancellationToken,
    deadline: Arc<Mutex<Instant>>,
    incoming: mpsc::Receiver<Frame>,
    outgoing: mpsc::Sender<String>,
    task: Option<JoinHandle<()>>,
}

impl Connection {
    pub fn context(&self) -> &ConnectionContext {
        &self.context
    }
    pub fn is_active(&self) -> bool {
        !self.closed.is_cancelled() && self.deadline.lock().is_ok_and(|d| Instant::now() < *d)
    }
    pub async fn receive(&mut self) -> Option<Frame> {
        if !self.is_active() {
            return None;
        }
        let frame = tokio::select! {
            biased;
            _ = self.closed.cancelled() => None,
            frame = self.incoming.recv() => frame,
        };
        if self.is_active() { frame } else { None }
    }
    /// Bounded delivery only; success does not establish receipt, persistence or
    /// execution. A disconnected connection never replays this queue.
    pub fn send(&self, frame: Frame) -> Result<()> {
        if !self.is_active() {
            return Err(TransportError::Closed);
        }
        validate_outbound(&self.context, &frame)?;
        let encoded = frame.encode().map_err(|_| TransportError::Invalid)?;
        self.outgoing.try_send(encoded).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                self.closed.cancel();
                TransportError::Busy
            }
            mpsc::error::TrySendError::Closed(_) => TransportError::Closed,
        })
    }
    pub async fn detach(&mut self) -> Result<()> {
        self.stop().await;
        Arc::get_mut(&mut self.client)
            .ok_or(TransportError::Busy)?
            .detach()
            .map_err(|_| TransportError::Enrollment)
    }
    pub async fn close(mut self) {
        self.stop().await;
    }
    async fn stop(&mut self) {
        self.closed.cancel();
        if let Some(mut task) = self.task.take()
            && tokio::time::timeout(WRITE_TIMEOUT, &mut task)
                .await
                .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.closed.cancel();
    }
}

fn socket_url(origin: &str) -> std::result::Result<String, &'static str> {
    let origin = validate_origin(origin, true).map_err(|_| "invalid origin")?;
    let mut url = reqwest::Url::parse(&origin).map_err(|_| "invalid origin")?;
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme).map_err(|_| "invalid origin")?;
    url.set_path("/v2/attachment");
    Ok(url.to_string())
}

fn welcome(
    frame: Frame,
    client: &EnrollmentClient,
    offered: &Features,
) -> Result<(ConnectionContext, u32)> {
    let Frame::Welcome {
        version,
        connection_id,
        machine_id,
        owner_id,
        epoch,
        lease_ms,
        features,
    } = frame
    else {
        return Err(TransportError::Invalid);
    };
    if version != VERSION
        || machine_id != client.machine_id()
        || Some(owner_id) != client.owner_id()
        || epoch != client.epoch()
    {
        return Err(TransportError::Invalid);
    }
    offered
        .confirm(&features)
        .map_err(|_| TransportError::Invalid)?;
    Ok((
        ConnectionContext {
            connection_id,
            machine_id,
            owner_id,
            epoch,
            features,
        },
        lease_ms,
    ))
}

fn validate_inbound(context: &ConnectionContext, frame: &Frame) -> Result<()> {
    frame
        .validate_features(&context.features)
        .map_err(|_| TransportError::Invalid)?;
    let valid = match frame {
        Frame::Command { command } => {
            command.connection_id == context.connection_id
                && command.machine_id == context.machine_id
                && command.principal_id == context.owner_id
        }
        Frame::ReplayRequest { connection_id, .. } => *connection_id == context.connection_id,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(TransportError::Invalid)
    }
}
fn validate_outbound(context: &ConnectionContext, frame: &Frame) -> Result<()> {
    frame
        .validate_features(&context.features)
        .map_err(|_| TransportError::Invalid)?;
    let connection_id = match frame {
        Frame::Result { connection_id, .. }
        | Frame::Event { connection_id, .. }
        | Frame::Replay { connection_id, .. }
        | Frame::SnapshotRequired { connection_id, .. } => *connection_id,
        _ => return Err(TransportError::Invalid),
    };
    if connection_id == context.connection_id {
        Ok(())
    } else {
        Err(TransportError::Invalid)
    }
}

/// One fresh-proof connection attempt. Failure preserves enrollment on disk.
/// Reconnection must call this again with a reopened enrollment and new challenge;
/// old commands, observations and authorization leases are never replayed here.
pub async fn connect(
    client: EnrollmentClient,
    offered: Features,
    cancel: CancellationToken,
) -> Result<Connection> {
    offered.validate().map_err(|_| TransportError::Invalid)?;
    let closed = cancel.child_token();
    let proof = tokio::select! {
        biased;
        _ = closed.cancelled() => return Err(TransportError::Closed),
        proof = client.connect_proof() => proof.map_err(|_| TransportError::Enrollment)?,
    };
    let target = socket_url(client.origin()).map_err(|_| TransportError::Invalid)?;
    let mut request = target
        .into_client_request()
        .map_err(|_| TransportError::Invalid)?;
    request.headers_mut().insert(
        "origin",
        client
            .origin()
            .parse()
            .map_err(|_| TransportError::Invalid)?,
    );
    request
        .headers_mut()
        .insert("x-voyage-request", "2".parse().unwrap());
    request
        .headers_mut()
        .insert("sec-websocket-protocol", PROTOCOL.parse().unwrap());
    let config = WebSocketConfig::default()
        .read_buffer_size(4096)
        .write_buffer_size(0)
        .max_write_buffer_size(MAX_FRAME_BYTES * 2)
        .max_message_size(Some(MAX_FRAME_BYTES))
        .max_frame_size(Some(MAX_FRAME_BYTES));
    let handshake = async {
        // connect_async does not follow redirects or use proxy environment variables.
        let (mut socket, response) = connect_async_with_config(request, Some(config), true)
            .await
            .map_err(|_| TransportError::Network)?;
        if response
            .headers()
            .get_all("sec-websocket-protocol")
            .iter()
            .count()
            != 1
            || response
                .headers()
                .get("sec-websocket-protocol")
                .is_none_or(|v| v != PROTOCOL)
        {
            return Err(TransportError::Invalid);
        }
        let started = Instant::now();
        let frame = Frame::Authenticate {
            version: VERSION,
            proof,
            features: offered.clone(),
        }
        .encode()
        .map_err(|_| TransportError::Invalid)?;
        socket
            .send(Message::Text(frame.into()))
            .await
            .map_err(|_| TransportError::Network)?;
        let text = loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => break text,
                Some(Ok(Message::Ping(bytes))) => socket
                    .send(Message::Pong(bytes))
                    .await
                    .map_err(|_| TransportError::Network)?,
                Some(Ok(Message::Pong(_))) => (),
                _ => return Err(TransportError::Invalid),
            }
        };
        let frame = Frame::decode(text.as_bytes()).map_err(|_| TransportError::Invalid)?;
        let (context, lease_ms) = welcome(frame, &client, &offered)?;
        let deadline = started + Duration::from_millis(u64::from(lease_ms));
        if Instant::now() >= deadline {
            return Err(TransportError::Timeout);
        }
        Ok((socket, context, deadline))
    };
    let (socket, context, expires) = tokio::select! {
        biased;
        _ = closed.cancelled() => return Err(TransportError::Closed),
        result = tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake) => result.map_err(|_| TransportError::Timeout)??,
    };
    let deadline = Arc::new(Mutex::new(expires));
    let client = Arc::new(client);
    let (outgoing, outgoing_rx) = mpsc::channel(QUEUE);
    let (incoming_tx, incoming) = mpsc::channel(QUEUE);
    let task = tokio::spawn(run(
        SocketOwner {
            socket,
            _client: client.clone(),
        },
        context.clone(),
        closed.clone(),
        deadline.clone(),
        outgoing_rx,
        incoming_tx,
    ));
    Ok(Connection {
        client,
        context,
        closed,
        deadline,
        incoming,
        outgoing,
        task: Some(task),
    })
}

// Field order deliberately drops the socket before releasing enrollment, including
// when Tokio aborts/drops the task future before it is first polled.
struct SocketOwner {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    _client: Arc<EnrollmentClient>,
}

fn heartbeat_at(deadline: Instant) -> Instant {
    let now = Instant::now();
    now + deadline.saturating_duration_since(now) / 3
}

async fn run(
    mut owner: SocketOwner,
    context: ConnectionContext,
    closed: CancellationToken,
    deadline: Arc<Mutex<Instant>>,
    mut outgoing: mpsc::Receiver<String>,
    incoming: mpsc::Sender<Frame>,
) {
    let _close_on_exit = closed.clone().drop_guard();
    let mut heartbeat = match deadline.lock() {
        Ok(d) => heartbeat_at(*d),
        Err(_) => return,
    };
    let mut pending_heartbeat = None;
    loop {
        let expires = match deadline.lock() {
            Ok(d) => *d,
            Err(_) => break,
        };
        let next = tokio::select! {
            _ = closed.cancelled() => break,
            _ = tokio::time::sleep_until(expires) => break,
            _ = tokio::time::sleep_until(heartbeat), if pending_heartbeat.is_none() => {
                pending_heartbeat = Some(Instant::now());
                match (Frame::Heartbeat { connection_id: context.connection_id }).encode() { Ok(text) => Some(Message::Text(text.into())), Err(_) => break }
            },
            message = owner.socket.next() => {
                // Fair selection cannot revive an already expired grant when
                // a queued Lease and its deadline become ready together.
                if closed.is_cancelled() || Instant::now() >= expires { break; }
                match message {
                    Some(Ok(Message::Text(text))) => {
                        let Ok(frame) = Frame::decode(text.as_bytes()) else { break };
                        if let Frame::Lease { connection_id, lease_ms } = frame {
                            if connection_id != context.connection_id { break; }
                            let Some(sent) = pending_heartbeat.take() else { break };
                            let next = sent + Duration::from_millis(u64::from(lease_ms));
                            if Instant::now() >= next { break; }
                            match deadline.lock() { Ok(mut d) => *d = next, Err(_) => break }
                            heartbeat = heartbeat_at(next);
                        } else if validate_inbound(&context, &frame).is_err() || incoming.try_send(frame).is_err() { break; }
                        None
                    },
                    Some(Ok(Message::Ping(bytes))) => Some(Message::Pong(bytes)),
                    Some(Ok(Message::Pong(_))) => None,
                    _ => break,
                }
            },
            message = outgoing.recv() => match message { Some(text) => Some(Message::Text(text.into())), None => break },
        };
        if let Some(message) = next {
            if closed.is_cancelled() || Instant::now() >= expires {
                break;
            }
            let result = tokio::select! {
                biased;
                _ = closed.cancelled() => break,
                _ = tokio::time::sleep_until(expires) => break,
                result = tokio::time::timeout(WRITE_TIMEOUT, owner.socket.send(message)) => result,
            };
            if !matches!(result, Ok(Ok(()))) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_target_preserves_exact_enrolled_origin_and_has_no_secrets() {
        assert_eq!(
            socket_url("https://vessel.example:8443").unwrap(),
            "wss://vessel.example:8443/v2/attachment"
        );
        assert_eq!(
            socket_url("http://127.0.0.1:8080").unwrap(),
            "ws://127.0.0.1:8080/v2/attachment"
        );
        for origin in [
            "https://u:p@example.com",
            "https://example.com?token=secret",
            "https://example.com/path",
            "http://localhost",
            "http://10.0.0.1",
            "wss://example.com",
        ] {
            assert!(socket_url(origin).is_err());
        }
    }

    fn context() -> ConnectionContext {
        use voyage_protocol::events::Feature;
        ConnectionContext {
            connection_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            owner_id: Uuid::new_v4(),
            epoch: 1,
            features: Features::new(vec![
                Feature::SequencedEvents,
                Feature::Replay,
                Feature::ToolActivity,
                Feature::Usage,
            ])
            .unwrap(),
        }
    }
    #[test]
    fn current_owner_connection_and_direction_are_required() {
        use voyage_protocol::{
            attachment::{Command, Operation},
            stream::Reply,
        };
        let scope = context();
        let command = Command {
            version: VERSION,
            connection_id: scope.connection_id,
            machine_id: scope.machine_id,
            principal_id: scope.owner_id,
            command_id: Uuid::new_v4(),
            expires_at_ms: 1,
            operation: Operation::List {
                after: None,
                limit: 10,
            },
        };
        // An expired ID may retrieve prior durable evidence; fresh-admission
        // deadlines remain a coordinator check, not an excuse to replay effects.
        assert!(
            validate_inbound(
                &scope,
                &Frame::Command {
                    command: command.clone()
                }
            )
            .is_ok()
        );
        let mut wrong = command.clone();
        wrong.connection_id = Uuid::new_v4();
        assert!(validate_inbound(&scope, &Frame::Command { command: wrong }).is_err());
        let mut wrong = command.clone();
        wrong.machine_id = Uuid::new_v4();
        assert!(validate_inbound(&scope, &Frame::Command { command: wrong }).is_err());
        let mut wrong = command.clone();
        wrong.principal_id = Uuid::new_v4();
        assert!(validate_inbound(&scope, &Frame::Command { command: wrong }).is_err());
        let reply = Frame::Result {
            connection_id: scope.connection_id,
            command_id: command.command_id,
            reply: Reply::Accepted {},
        };
        assert!(validate_outbound(&scope, &reply).is_ok());
        assert!(validate_inbound(&scope, &reply).is_err());
        assert!(validate_outbound(&scope, &Frame::Command { command }).is_err());
        assert!(validate_outbound(&context(), &reply).is_err());
        assert!(
            validate_outbound(
                &scope,
                &Frame::Lease {
                    connection_id: scope.connection_id,
                    lease_ms: 10_000
                }
            )
            .is_err()
        );
    }

    #[test]
    fn unsupported_observations_never_enter_the_outgoing_queue() {
        use voyage_protocol::events::{EventCursor, RunEvent, SequencedEvent};
        let mut scope = context();
        scope.features = Features::default();
        let frame = Frame::Event {
            connection_id: scope.connection_id,
            session_id: Uuid::new_v4(),
            event: SequencedEvent {
                cursor: EventCursor::new(1).unwrap(),
                run_id: Uuid::new_v4(),
                event: RunEvent::Running {},
            },
        };
        assert!(validate_outbound(&scope, &frame).is_err());
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn already_cancelled_connect_never_requests_a_challenge_or_removes_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("enrollment");
        let client = EnrollmentClient::open(&path, "http://127.0.0.1:1", true).unwrap();
        let machine = client.machine_id();
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            connect(client, Features::default(), cancel).await,
            Err(TransportError::Closed)
        ));
        let reopened = EnrollmentClient::open(&path, "http://127.0.0.1:1", true).unwrap();
        assert_eq!(reopened.machine_id(), machine);
    }
}
