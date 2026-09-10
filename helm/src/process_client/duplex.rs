//! One authenticated socket per route activation. No command replay or HTTP fallback.
use anyhow::{Result, ensure};
use futures_util::{SinkExt, StreamExt, stream::BoxStream};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::{
    duplex::*,
    vessel::{
        VESSEL_API_VERSION, VesselCommand, VesselEvent, VesselEventRequest, VesselRequest,
        VesselResponse,
    },
};

/// Monotonic loss counter cannot hide a disconnect behind a coalesced reconnect.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ConnectionState {
    pub socket_id: Option<Uuid>,
    pub loss_generation: u64,
}

pub struct IncomingReverseRequest {
    pub socket_id: Uuid,
    pub request_id: Uuid,
    pub request: ReverseRequest,
    pub reply: ReverseResponder,
}

/// Bound to the exact socket that received the request. Never reconnects.
pub struct ReverseResponder {
    request_id: Uuid,
    sender: Option<oneshot::Sender<ReverseReply>>,
}
impl ReverseResponder {
    pub fn respond(mut self, reply: ReverseReply) -> Result<()> {
        self.sender
            .take()
            .unwrap()
            .send(reply)
            .map_err(|_| anyhow::anyhow!("reverse request expired or connection lost"))
    }
    pub fn request_id(&self) -> Uuid {
        self.request_id
    }
}
impl Drop for ReverseResponder {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(ReverseReply::Unavailable);
        }
    }
}

pub(super) struct Slot {
    connection: tokio::sync::Mutex<Option<Handle>>,
    state: watch::Sender<ConnectionState>,
    observed_vessel: Mutex<Option<Uuid>>,
    reverse: Arc<Mutex<Option<mpsc::Sender<IncomingReverseRequest>>>>,
    stop: CancellationToken,
}
impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DuplexConnection")
            .field("state", &*self.state.borrow())
            .finish_non_exhaustive()
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
type Pool = HashMap<(Uuid, u64), Weak<Slot>>;
pub(super) fn slot(id: Uuid, generation: u64) -> Arc<Slot> {
    static POOL: OnceLock<Mutex<Pool>> = OnceLock::new();
    let mut pool = POOL.get_or_init(Default::default).lock().unwrap();
    pool.retain(|_, slot| slot.strong_count() > 0);
    if let Some(slot) = pool.get(&(id, generation)).and_then(Weak::upgrade) {
        return slot;
    }
    let slot = Arc::new(Slot {
        connection: tokio::sync::Mutex::new(None),
        state: watch::channel(ConnectionState::default()).0,
        observed_vessel: Mutex::new(None),
        reverse: Arc::new(Mutex::new(None)),
        stop: CancellationToken::new(),
    });
    pool.insert((id, generation), Arc::downgrade(&slot));
    slot
}

#[derive(Clone)]
struct Handle {
    sender: mpsc::Sender<Outbound>,
    stop: CancellationToken,
}
enum Outbound {
    Command {
        request: VesselRequest,
        reply: oneshot::Sender<Result<VesselResponse>>,
    },
    Subscribe {
        request: VesselEventRequest,
        events: mpsc::Sender<Result<VesselEvent>>,
        reply: oneshot::Sender<Result<Uuid>>,
    },
    Unsubscribe(Uuid),
}
struct Subscription {
    events: mpsc::Sender<Result<VesselEvent>>,
    identities: HashMap<Uuid, Uuid>,
}
enum PendingReply {
    Command(oneshot::Sender<Result<VesselResponse>>),
    Subscribe {
        reply: oneshot::Sender<Result<Uuid>>,
        subscription: Subscription,
    },
}
struct Pending {
    reply: PendingReply,
    deadline: tokio::time::Instant,
}
impl PendingReply {
    fn closed(&self) -> bool {
        match self {
            Self::Command(tx) => tx.is_closed(),
            Self::Subscribe { reply, .. } => reply.is_closed(),
        }
    }
    fn fail(self) {
        let error = || {
            anyhow::anyhow!(
                "Vessel socket interrupted or timed out; outcome unknown; retain original command identity"
            )
        };
        match self {
            Self::Command(tx) => {
                let _ = tx.send(Err(error()));
            }
            Self::Subscribe { reply, .. } => {
                let _ = reply.send(Err(error()));
            }
        }
    }
}
impl Slot {
    pub(super) fn state(&self) -> watch::Receiver<ConnectionState> {
        self.state.subscribe()
    }
    pub(super) fn disconnect(&self) {
        self.stop.cancel();
        self.state.send_modify(|s| {
            if s.socket_id.take().is_some() {
                s.loss_generation = s.loss_generation.saturating_add(1);
            }
        });
    }
    pub(super) fn reverse_requests(&self) -> Result<mpsc::Receiver<IncomingReverseRequest>> {
        let mut reverse = self.reverse.lock().unwrap();
        ensure!(
            reverse.as_ref().is_none_or(mpsc::Sender::is_closed),
            "reverse request handler already installed"
        );
        let (tx, rx) = mpsc::channel(MAX_IN_FLIGHT);
        *reverse = Some(tx);
        Ok(rx)
    }
    async fn connection(&self, client: &super::transport::Client) -> Result<Handle> {
        let mut connection = self.connection.lock().await;
        ensure!(
            !self.stop.is_cancelled(),
            "Vessel connection activation is disconnected"
        );
        if let Some(handle) = connection
            .as_ref()
            .filter(|h| !h.sender.is_closed() && !h.stop.is_cancelled())
        {
            return Ok(handle.clone());
        }
        // A stopped actor may not yet have published its terminal state. Fence
        // that socket BEFORE publishing a reconnect; watch coalescing preserves
        // loss_generation even if consumers never observe the intermediate None.
        if connection.take().is_some() {
            self.state.send_modify(|state| {
                if state.socket_id.take().is_some() {
                    state.loss_generation = state.loss_generation.saturating_add(1);
                }
            });
        }
        let (request, pin) = client.socket_request()?;
        let config = WebSocketConfig::default()
            .max_message_size(Some(MAX_FRAME_BYTES))
            .max_frame_size(Some(MAX_FRAME_BYTES))
            .write_buffer_size(16 * 1024)
            .max_write_buffer_size(MAX_FRAME_BYTES + 64 * 1024);
        let (mut socket, response) = tokio::time::timeout(
            Duration::from_secs(8),
            connect_async_with_config(request, Some(config), true),
        )
        .await
        .map_err(|_| anyhow::anyhow!("Vessel socket connection deadline elapsed"))?
        .map_err(|_| anyhow::anyhow!("Vessel socket upgrade unavailable; no HTTP fallback"))?;
        ensure!(
            response
                .headers()
                .get("sec-websocket-protocol")
                .and_then(|h| h.to_str().ok())
                == Some(SUBPROTOCOL),
            "unsupported Vessel socket protocol"
        );
        let message = tokio::time::timeout(Duration::from_secs(8), socket.next())
            .await
            .map_err(|_| anyhow::anyhow!("Vessel socket greeting timed out"))?
            .ok_or_else(|| anyhow::anyhow!("Vessel socket closed before greeting"))?
            .map_err(|_| anyhow::anyhow!("invalid Vessel socket greeting"))?;
        let Message::Text(text) = message else {
            anyhow::bail!("expected Vessel socket greeting");
        };
        let ServerFrame::Hello {
            protocol,
            socket_id,
            vessel_id,
        } = serde_json::from_str(&text)
            .map_err(|_| anyhow::anyhow!("invalid Vessel socket greeting"))?
        else {
            anyhow::bail!("expected Vessel socket greeting");
        };
        ensure!(
            protocol == VESSEL_API_VERSION
                && !socket_id.is_nil()
                && !vessel_id.is_nil()
                && pin.is_none_or(|id| id == vessel_id),
            "Vessel socket protocol or identity changed"
        );
        ensure!(
            !self.stop.is_cancelled(),
            "Vessel connection activation is disconnected"
        );
        {
            let mut observed = self.observed_vessel.lock().unwrap();
            ensure!(
                observed.is_none_or(|prior| prior == vessel_id),
                "Vessel identity changed on reconnect"
            );
            *observed = Some(vessel_id);
        }
        let (sender, receiver) = mpsc::channel(MAX_IN_FLIGHT);
        let stop = self.stop.child_token();
        let handle = Handle {
            sender,
            stop: stop.clone(),
        };
        self.state
            .send_modify(|state| state.socket_id = Some(socket_id));
        let state = self.state.clone();
        let reverse = self.reverse.clone();
        tokio::spawn(async move {
            let _ = run(
                socket,
                receiver,
                stop.clone(),
                socket_id,
                reverse,
                state.clone(),
            )
            .await;
            stop.cancel();
            state.send_modify(|state| {
                if state.socket_id == Some(socket_id) {
                    state.socket_id = None;
                    state.loss_generation = state.loss_generation.saturating_add(1);
                }
            });
        });
        *connection = Some(handle.clone());
        Ok(handle)
    }
    pub(super) async fn exchange(
        &self,
        client: &super::transport::Client,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        let handle = self.connection(client).await?;
        let (tx, rx) = oneshot::channel();
        handle
            .sender
            .try_send(Outbound::Command {
                request: VesselRequest {
                    protocol: VESSEL_API_VERSION,
                    command,
                },
                reply: tx,
            })
            .map_err(|_| {
                super::transport::Refusal(
                    "Vessel socket queue unavailable; request not sent".into(),
                )
            })?;
        let response = rx
            .await
            .map_err(|_| anyhow::anyhow!("Vessel socket lost; command outcome unknown"))??;
        super::access::public_response(response, client.is_local())
    }
    pub(super) async fn events(
        &self,
        client: &super::transport::Client,
        request: VesselEventRequest,
    ) -> Result<BoxStream<'static, Result<VesselEvent>>> {
        ensure!(
            !request.subscriptions.is_empty() && request.subscriptions.len() <= MAX_SUBSCRIPTIONS,
            "invalid Vessel subscription count"
        );
        let mut ids = std::collections::HashSet::new();
        ensure!(
            request.subscriptions.iter().all(|s| !s.session_id.is_nil()
                && !s.incarnation.is_nil()
                && ids.insert(s.session_id)),
            "invalid Vessel subscription identity"
        );
        let handle = self.connection(client).await?;
        let (tx, rx) = oneshot::channel();
        let (events, mut receiver) = mpsc::channel(32);
        handle
            .sender
            .try_send(Outbound::Subscribe {
                request,
                events,
                reply: tx,
            })
            .map_err(|_| anyhow::anyhow!("Vessel socket subscription queue unavailable"))?;
        let id = rx
            .await
            .map_err(|_| anyhow::anyhow!("Vessel socket lost during subscription"))??;
        struct Guard(Handle, Uuid);
        impl Drop for Guard {
            fn drop(&mut self) {
                if self
                    .0
                    .sender
                    .try_send(Outbound::Unsubscribe(self.1))
                    .is_err()
                {
                    self.0.stop.cancel();
                }
            }
        }
        let guard = Guard(handle, id);
        Ok(Box::pin(async_stream::try_stream! {
            let _guard = guard;
            while let Some(event) = receiver.recv().await { yield event?; }
            Err(anyhow::anyhow!("Vessel socket event subscription ended; reconnect from durable cursor"))?;
        }))
    }
}

pub(super) fn request(
    base: reqwest::Url,
    token: &str,
    grant: Option<Uuid>,
    pin: Option<Uuid>,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut url = base;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| anyhow::anyhow!("invalid Vessel socket URL"))?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| anyhow::anyhow!("invalid Vessel socket URL"))?;
    let headers = request.headers_mut();
    headers.insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid Vessel credential"))?,
    );
    headers.insert("sec-websocket-protocol", SUBPROTOCOL.parse()?);
    if let Some(grant) = grant {
        headers.insert("x-voyage-grant", grant.to_string().parse()?);
    }
    if let Some(pin) = pin {
        headers.insert("x-voyage-vessel", pin.to_string().parse()?);
    }
    Ok(request)
}

async fn run(
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    mut receiver: mpsc::Receiver<Outbound>,
    stop: CancellationToken,
    socket_id: Uuid,
    reverse: Arc<Mutex<Option<mpsc::Sender<IncomingReverseRequest>>>>,
    state: watch::Sender<ConnectionState>,
) -> Result<()> {
    let (mut sink, mut stream) = socket.split();
    let mut pending: HashMap<Uuid, Pending> = HashMap::new();
    let mut subscriptions: HashMap<Uuid, Subscription> = HashMap::new();
    let mut reverse_pending = futures_util::stream::FuturesUnordered::new();
    let mut reverse_ids = std::collections::HashSet::new();
    let mut tick = tokio::time::interval(Duration::from_secs(HEARTBEAT_SECONDS));
    let mut last_received = tokio::time::Instant::now();
    let result: Result<()> = async {
      loop {
        let send = tokio::select! {
            _ = stop.cancelled() => break,
            _ = tick.tick() => {
                ensure!(last_received.elapsed() < Duration::from_secs(DEADLINE_SECONDS), "Vessel socket heartbeat expired");
                let now = tokio::time::Instant::now();
                let expired: Vec<_> = pending.iter().filter(|(_,p)| p.deadline <= now || p.reply.closed()).map(|(id,_)|*id).collect();
                for id in expired { if let Some(p) = pending.remove(&id) { p.reply.fail(); } }
                Some(Message::Ping(Vec::new().into()))
            },
            Some((id, reply)) = reverse_pending.next(), if !reverse_pending.is_empty() => {
                reverse_ids.remove(&id);
                Some(encoded(&ClientFrame::ReverseReply{request_id:id,reply})?)
            },
            item = receiver.recv() => {
                let Some(item) = item else { break; };
                match item {
                    Outbound::Unsubscribe(id) => { subscriptions.remove(&id); Some(encoded(&ClientFrame::Unsubscribe{subscription_id:id})?) },
                    Outbound::Command{request,reply} => {
                        if reply.is_closed() { continue; }
                        if pending.len() >= MAX_IN_FLIGHT { let _ = reply.send(Err(super::transport::Refusal("Vessel in-flight limit; request not sent".into()).into())); continue; }
                        let id = Uuid::new_v4();
                        let message = encoded(&ClientFrame::Command {request_id:id,request});
                        match message { Ok(message) => {pending.insert(id, Pending{reply:PendingReply::Command(reply),deadline:tokio::time::Instant::now()+Duration::from_secs(DEADLINE_SECONDS)}); Some(message)}, Err(error) => {let _=reply.send(Err(super::transport::Refusal(error.to_string()).into())); continue;} }
                    },
                    Outbound::Subscribe{request,events,reply} => {
                        if reply.is_closed() { continue; }
                        if pending.len() >= MAX_IN_FLIGHT || subscriptions.len() >= MAX_IN_FLIGHT { let _ = reply.send(Err(anyhow::anyhow!("Vessel subscription limit"))); continue; }
                        let id = Uuid::new_v4();
                        let identities = request.subscriptions.iter().map(|s| (s.session_id,s.incarnation)).collect();
                        let message = encoded(&ClientFrame::Subscribe{request_id:id,request})?;
                        pending.insert(id,Pending{reply:PendingReply::Subscribe{reply,subscription:Subscription{events,identities}},deadline:tokio::time::Instant::now()+Duration::from_secs(DEADLINE_SECONDS)});
                        Some(message)
                    },
                }
            },
            message = stream.next() => {
                let message = message.ok_or_else(|| anyhow::anyhow!("Vessel socket closed"))?.map_err(|_| anyhow::anyhow!("Vessel socket interrupted"))?;
                last_received = tokio::time::Instant::now();
                match message {
                    Message::Ping(bytes) => Some(Message::Pong(bytes)),
                    Message::Pong(_) => None,
                    Message::Close(_) => break,
                    Message::Text(text) => {
                        let frame:ServerFrame=serde_json::from_str(&text).map_err(|_|anyhow::anyhow!("invalid Vessel socket frame"))?;
                        match frame {
                            ServerFrame::Hello{..} => anyhow::bail!("duplicate Vessel socket greeting"),
                            ServerFrame::Reply{request_id,response} => {
                                ensure!(response.protocol == VESSEL_API_VERSION, "unsupported Vessel response version");
                                if let Some(p)=pending.remove(&request_id) {
                                    match p.reply { PendingReply::Command(tx) => {let _=tx.send(Ok(response));}, PendingReply::Subscribe{reply,..} => {let _=reply.send(Err(anyhow::anyhow!("Vessel subscription refused")));} }
                                } // A late reply to a timed-out command is not a new receipt or retry.
                            },
                            ServerFrame::Subscribed{request_id} => {
                                if let Some(p)=pending.remove(&request_id) {
                                    let PendingReply::Subscribe{reply,subscription}=p.reply else {anyhow::bail!("Vessel reply type mismatch");};
                                    subscriptions.insert(request_id,subscription);
                                    if reply.send(Ok(request_id)).is_err() { subscriptions.remove(&request_id); return Err(anyhow::anyhow!("abandoned Vessel subscription")); }
                                }
                            },
                            ServerFrame::Event{subscription_id,event} => {
                                ensure!(event.protocol == VESSEL_API_VERSION, "unsupported Vessel event version");
                                if let Some(subscription)=subscriptions.get_mut(&subscription_id) {
                                    let incarnation=subscription.identities.get_mut(&event.session_id).ok_or_else(||anyhow::anyhow!("Vessel event session mismatch"))?;
                                    if *incarnation != event.incarnation {
                                        ensure!(event.result.get("owner_changed")==Some(&serde_json::Value::Bool(true)) && event.result.get("replay_gap")==Some(&serde_json::Value::Bool(true)), "Vessel event incarnation mismatch");
                                        *incarnation=event.incarnation;
                                    }
                                    subscription.events.try_send(Ok(event)).map_err(|_|anyhow::anyhow!("Vessel event consumer stalled"))?;
                                }
                            },
                            ServerFrame::ReverseRequest{request_id,request} => {
                                ensure!(!request_id.is_nil() && reverse_ids.len()<MAX_IN_FLIGHT && reverse_ids.insert(request_id), "invalid reverse correlation");
                                let (tx,rx)=oneshot::channel();
                                let incoming=IncomingReverseRequest{socket_id,request_id,request,reply:ReverseResponder{request_id,sender:Some(tx)}};
                                if let Some(sender)=reverse.lock().unwrap().as_ref() { let _=sender.try_send(incoming); }
                                reverse_pending.push(async move { (request_id, tokio::time::timeout(Duration::from_secs(DEADLINE_SECONDS),rx).await.ok().and_then(Result::ok).unwrap_or(ReverseReply::Unavailable)) });
                            },
                        }
                        None
                    },
                    _ => anyhow::bail!("unsupported Vessel socket frame"),
                }
            }
        };
        if let Some(message)=send { tokio::time::timeout(Duration::from_secs(5),sink.send(message)).await.map_err(|_|anyhow::anyhow!("Vessel socket write stalled"))?.map_err(|_|anyhow::anyhow!("Vessel socket write failed"))?; }
      }
      Ok(())
    }.await;
    state.send_modify(|state| {
        if state.socket_id == Some(socket_id) {
            state.socket_id = None;
            state.loss_generation = state.loss_generation.saturating_add(1);
        }
    });
    for (_, p) in pending {
        p.reply.fail();
    }
    // Dropping subscription senders wakes readers; never inject terminal or browser
    // private payloads into transcript/history while reporting connection loss.
    drop(subscriptions);
    let _ = tokio::time::timeout(Duration::from_secs(1), sink.close()).await;
    result
}
fn encoded(frame: &ClientFrame) -> Result<Message> {
    let text = serde_json::to_string(frame)?;
    ensure!(
        text.len() <= MAX_FRAME_BYTES,
        "Vessel request exceeds socket frame limit"
    );
    Ok(Message::Text(text.into()))
}
