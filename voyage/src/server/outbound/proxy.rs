//! Private, transient lease transport. No IPC message is durable execution authority.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::{net::UnixStream, sync::mpsc};
use tokio_util::sync::CancellationToken;
use voyage_protocol::{
    process::{ProcessRegistration, read_frame, write_frame},
    stream::Frame,
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Identity {
    pub origin: String,
    pub machine_id: uuid::Uuid,
    pub owner_id: uuid::Uuid,
    pub epoch: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct ContextData {
    pub identity: Identity,
    pub connection_id: uuid::Uuid,
    pub features: voyage_protocol::events::Features,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Hello {
    pub session_id: uuid::Uuid,
    pub incarnation: uuid::Uuid,
    pub token: String,
}
#[derive(Serialize, Deserialize)]
pub(super) enum Message {
    Lease { until: u64 },
    Frame(Box<Frame>),
    Withdraw,
    Closed { cleanup_observed: bool },
}
pub(super) fn now() -> u64 {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut value) } != 0 {
        return u64::MAX;
    }
    (value.tv_sec as u64)
        .saturating_mul(1000)
        .saturating_add(value.tv_nsec as u64 / 1_000_000)
}
pub(super) fn identity(directory: &Path) -> Result<Identity> {
    let file =
        crate::attachment::journal::open_private_file(&directory.join("outbound-identity.json"))?;
    Ok(serde_json::from_reader(file)?)
}
#[derive(Clone)]
pub(super) struct Lease {
    closed: CancellationToken,
    until: Arc<AtomicU64>,
}
impl std::fmt::Debug for Lease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutboundProxyLease").finish_non_exhaustive()
    }
}
impl Lease {
    pub fn is_active(&self) -> bool {
        !self.closed.is_cancelled() && now() < self.until.load(Ordering::Acquire)
    }
}
pub(super) struct Connection {
    context: crate::attachment::transport::ConnectionContext,
    lease: Lease,
    incoming: mpsc::Receiver<Frame>,
    outgoing: mpsc::Sender<Message>,
    task: tokio::task::JoinHandle<()>,
}
impl Connection {
    pub fn lease(&self) -> Lease {
        self.lease.clone()
    }
    pub fn context(&self) -> &crate::attachment::transport::ConnectionContext {
        &self.context
    }
    pub async fn receive(&mut self) -> Option<Frame> {
        tokio::select! { biased; _ = self.lease.closed.cancelled() => None, frame = self.incoming.recv() => frame }
    }
    pub fn send(&self, frame: Frame) -> Result<()> {
        ensure!(self.lease.is_active(), "outbound transport lease inactive");
        if self
            .outgoing
            .try_send(Message::Frame(Box::new(frame)))
            .is_err()
        {
            self.lease.closed.cancel();
            anyhow::bail!("outbound transport queue unavailable");
        }
        Ok(())
    }
    pub async fn close(self, cleanup_observed: bool) {
        // Drain queued terminal frames before acknowledging clean worker departure.
        let _ = self
            .outgoing
            .send(Message::Closed { cleanup_observed })
            .await;
        let mut task = self.task;
        if tokio::time::timeout(std::time::Duration::from_secs(2), &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
        self.lease.closed.cancel();
    }
}
pub(super) async fn connect(
    directory: &Path,
    registration: &ProcessRegistration,
) -> Result<Connection> {
    let mut socket = UnixStream::connect(directory.join("outbound-relay.sock")).await?;
    ensure!(
        socket.peer_cred()?.uid() == unsafe { libc::geteuid() },
        "relay peer mismatch"
    );
    write_frame(
        &mut socket,
        &Hello {
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            token: registration.token.clone(),
        },
    )
    .await?;
    let context: ContextData = read_frame(&mut socket).await?;
    let Message::Lease { until } = read_frame(&mut socket).await? else {
        anyhow::bail!("relay lease missing")
    };
    ensure!(until > now(), "relay lease expired");
    let lease = Lease {
        closed: CancellationToken::new(),
        until: Arc::new(AtomicU64::new(until)),
    };
    let (incoming_tx, incoming) = mpsc::channel(8);
    let (outgoing, mut outgoing_rx) = mpsc::channel(8);
    let observed = lease.clone();
    let (mut reader, mut writer) = socket.into_split();
    let task = tokio::spawn(async move {
        let _guard = observed.closed.clone().drop_guard();
        let (frames, mut received) = mpsc::channel(8);
        let reading = tokio::spawn(async move {
            while let Ok(message) = read_frame::<Message>(&mut reader).await {
                if frames.send(message).await.is_err() {
                    break;
                }
            }
        });
        let mut receiving = true;
        loop {
            let remaining = observed.until.load(Ordering::Acquire).saturating_sub(now());
            tokio::select! { biased;
                _ = tokio::time::sleep(std::time::Duration::from_millis(remaining)), if !observed.closed.is_cancelled() => observed.closed.cancel(),
                message = received.recv(), if receiving => match message {
                    Some(Message::Lease { until }) if observed.is_active() && until > now() => observed.until.store(until, Ordering::Release),
                    Some(Message::Frame(frame)) if observed.is_active() => if incoming_tx.try_send(*frame).is_err() { break; },
                    _ => { observed.closed.cancel(); receiving = false; },
                },
                message = outgoing_rx.recv() => {
                    let Some(message) = message else { break };
                    let closed = matches!(message, Message::Closed { .. });
                    if !matches!(tokio::time::timeout(std::time::Duration::from_secs(2), write_frame(&mut writer, &message)).await, Ok(Ok(()))) || closed { break; }
                }
            }
        }
        reading.abort();
        let _ = reading.await;
    });
    Ok(Connection {
        context: crate::attachment::transport::ConnectionContext {
            connection_id: context.connection_id,
            machine_id: context.identity.machine_id,
            owner_id: context.identity.owner_id,
            epoch: context.identity.epoch,
            features: context.features,
        },
        lease,
        incoming,
        outgoing,
        task,
    })
}
