use std::{collections::VecDeque, sync::{Arc, atomic::{AtomicUsize, Ordering}}, time::Duration};
use anyhow::{Result, ensure, bail};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::{io::{AsyncRead, AsyncWrite, BufReader}, sync::{Semaphore, oneshot}, time::{Instant, timeout, timeout_at}};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use super::{Definitions, Kind, wire};
use crate::tools::schema::CompiledSchema;

const INIT: Duration = Duration::from_secs(5);
const MAX_INVOCATION: Duration = Duration::from_secs(120);
const CANCEL: Duration = Duration::from_millis(100);
const SHUTDOWN: Duration = Duration::from_secs(1);
const CLEANUP: Duration = Duration::from_secs(5);
pub(crate) const MAX_READ: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Identity {
    pub session: Uuid,
    pub incarnation: Uuid,
    pub run: Uuid,
    pub invocation: Uuid,
    pub package: String,
    pub digest: String,
}

pub(crate) struct Invocation {
    pub identity: Identity,
    pub kind: Kind,
    pub name: String,
    pub arguments: Value,
    /// Absolute deadline set at runtime admission; includes queue and approvals.
    pub deadline: Instant,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cleanup { Observed, Pending }

pub(crate) struct Completion {
    /// Authored, bounded diagnostics only; never subprocess stderr/error text.
    pub result: Result<Value>,
    pub cleanup: Cleanup,
}

/// Mandatory authority boundary, implemented by the executing Voyage only.
/// Must atomically revalidate exact grants and persist invocation/resource intent
/// before launch. Use sealed immutable files and required empty-runtime isolation.
/// No inherited env, workspace, network, system mounts, provider state or PTY.
/// A dropped launch future MUST retain its child/reservation for guardian recovery;
/// neither returning Err nor dropping this adapter attests cleanup.
#[async_trait]
pub(crate) trait LaunchAdapter: Send + Sync + 'static {
    async fn launch(&self, identity: &Identity, deadline: Instant) -> Result<Launched>;
}

pub(crate) struct Launched {
    pub reader: Box<dyn AsyncRead + Unpin + Send>,
    pub writer: Box<dyn AsyncWrite + Unpin + Send>,
    pub lease: Box<dyn Lease>,
}

/// Own the child, process-tree tracker and durable host-resource reservation.
/// On timeout/drop/panic retain unresolved evidence and arrange guardian recovery.
/// `Observed` requires positive descendant cleanup, never PID/lock/EOF/ack alone.
/// Persist that outcome even when the invocation receiver was dropped.
#[async_trait]
pub(crate) trait Lease: Send + 'static {
    async fn terminate_and_observe(&mut self) -> Cleanup;
}

/// A narrowed broker, never serialized to the extension. Implementations enforce
/// current roots/access/delegation/approvals and return redacted UTF-8 only.
/// Calls are cancellation-safe: dropping read must cancel pending approval/work.
#[async_trait]
pub(crate) trait Host: Send + Sync + 'static {
    async fn read(&self, identity: &Identity, path: &str, offset: u64, max_bytes: usize,
        deadline: Instant) -> Result<String>;
    /// Redact/sanitize before presentation; bounded backpressure counts in deadline.
    async fn progress(&self, identity: &Identity, text: &str) -> Result<()>;
}

pub(crate) struct InvocationHandle {
    cancel: CancellationToken,
    receiver: Option<oneshot::Receiver<Completion>>,
}
impl InvocationHandle {
    pub(crate) fn cancel(&self) { self.cancel.cancel(); }
    pub(crate) async fn completion(mut self) -> Completion {
        match self.receiver.take().expect("completion receiver").await {
            Ok(completion) => completion,
            Err(_) => Completion { result: Err(anyhow::anyhow!("extension supervisor interrupted")),
                cleanup: Cleanup::Pending },
        }
    }
}
impl Drop for InvocationHandle {
    fn drop(&mut self) { self.cancel.cancel(); }
}

/// Keep one executor per admitted package snapshot. One active call, at most eight
/// queued calls; lazy launch only after a slot is acquired. No ambient hooks.
pub(crate) struct Executor {
    definitions: Definitions,
    pinned: Value,
    capabilities: Vec<String>,
    adapter: Arc<dyn LaunchAdapter>,
    active: Arc<Semaphore>,
    admitted: Arc<Semaphore>,
    closed: CancellationToken,
    unsettled: AtomicUsize,
}
struct SupervisorGuard<'a> { executor: &'a Executor, completed: bool }
impl Drop for SupervisorGuard<'_> {
    fn drop(&mut self) {
        if !self.completed { self.executor.close(); }
    }
}

impl Executor {
    pub(crate) fn new(pinned: Value, capabilities: Vec<String>, adapter: Arc<dyn LaunchAdapter>) -> Result<Arc<Self>> {
        let definitions = Definitions::parse(&pinned, &capabilities)?;
        Ok(Arc::new(Self { definitions, pinned, capabilities, adapter,
            active: Arc::new(Semaphore::new(1)), admitted: Arc::new(Semaphore::new(9)), closed: CancellationToken::new(), unsettled: AtomicUsize::new(0) }))
    }

    /// Revoke catalog grants first, then close admission and cancel all work.
    /// This is a request, not proof of cleanup; retain resource-ledger blockers.
    pub(crate) fn close(&self) { self.closed.cancel(); }

    /// Closes admission, then waits at most seven seconds for all detached work.
    /// A returned Pending includes panics and any unobserved child obligation;
    /// later guardian recovery remains the parent's durable authority.
    pub(crate) async fn shutdown(&self) -> Cleanup {
        self.close();
        match timeout(Duration::from_secs(7), self.admitted.acquire_many(9)).await {
            Ok(Ok(_permits)) if self.unsettled.load(Ordering::SeqCst) == 0 => Cleanup::Observed,
            _ => Cleanup::Pending,
        }
    }

    pub(crate) fn start(self: &Arc<Self>, mut call: Invocation, host: Arc<dyn Host>) -> Result<InvocationHandle> {
        ensure!(!self.closed.is_cancelled(), "extension executor is quarantined or closed");
        let def = self.definitions.find(call.kind, &call.name)?;
        CompiledSchema::compile(&def.input_schema)?.validate(&call.arguments)?;
        let output = CompiledSchema::compile(&def.output_schema)?;
        ensure!(call.identity.package.len() <= 128 && !call.identity.package.is_empty()
            && call.identity.package.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            && call.identity.digest.len() == 64
            && call.identity.digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid extension identity");
        call.deadline = call.deadline.min(Instant::now() + MAX_INVOCATION);
        ensure!(call.deadline > Instant::now() && !call.cancellation.is_cancelled(), "extension admission expired");
        let admission = self.admitted.clone().try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("extension queue is full"))?;
        // Recheck after owning a permit: shutdown cannot observe a drained queue
        // and then race a previously prechecked admission into a new task.
        ensure!(!self.closed.is_cancelled(), "extension executor is closed");
        let cancel = call.cancellation.child_token();
        call.cancellation = cancel.clone();
        let (sender, receiver) = oneshot::channel();
        let executor = self.clone();
        self.unsettled.fetch_add(1, Ordering::SeqCst);
        // Intentionally detached from the caller. Owned admission, transport and
        // lease survive receiver drop. Runtime death is handled by durable lease.
        tokio::spawn(async move {
            let _admission = admission;
            let mut guard = SupervisorGuard { executor: &executor, completed: false };
            let slot = tokio::select! {
                biased;
                _ = executor.closed.cancelled() => None,
                _ = call.cancellation.cancelled() => None,
                value = timeout_at(call.deadline, executor.active.clone().acquire_owned()) =>
                    value.ok().and_then(|result| result.ok()),
            };
            let completion = if slot.is_some() {
                executor.execute(&call, host.as_ref(), &output).await
            } else {
                Completion { result: Err(anyhow::anyhow!("extension queue cancelled or expired")), cleanup: Cleanup::Observed }
            };
            if completion.cleanup == Cleanup::Observed { executor.unsettled.fetch_sub(1, Ordering::SeqCst); }
            if slot.is_some() && (completion.result.is_err() || completion.cleanup == Cleanup::Pending) { executor.close(); }
            guard.completed = true;
            drop(slot);
            let _ = sender.send(completion);
        });
        Ok(InvocationHandle { cancel, receiver: Some(receiver) })
    }

    async fn execute(&self, call: &Invocation, host: &dyn Host, output: &CompiledSchema) -> Completion {
        let launched = tokio::select! {
            biased;
            _ = self.closed.cancelled() => Err(anyhow::anyhow!("extension closed")),
            _ = call.cancellation.cancelled() => Err(anyhow::anyhow!("extension cancelled")),
            value = timeout_at(call.deadline, self.adapter.launch(&call.identity, call.deadline)) =>
                value.unwrap_or_else(|_| Err(anyhow::anyhow!("extension launch expired"))),
        };
        let mut launched = match launched {
            Ok(value) => value,
            // Adapter may have reserved/spawned: lack of a returned lease is NOT
            // evidence of no effects. Adapter owns persistence and recovery.
            Err(_) => return Completion { result: Err(anyhow::anyhow!("extension launch refused or interrupted")), cleanup: Cleanup::Pending },
        };
        let mut reader = BufReader::new(launched.reader);
        let result = tokio::select! {
            biased;
            _ = self.closed.cancelled() => Err(anyhow::anyhow!("extension closed")),
            _ = call.cancellation.cancelled() => Err(anyhow::anyhow!("extension cancelled")),
            value = timeout_at(call.deadline, self.exchange(call, host, output, &mut reader, &mut launched.writer)) =>
                value.unwrap_or_else(|_| Err(anyhow::anyhow!("extension invocation expired"))),
        };
        if result.is_err() {
            let _ = timeout(CANCEL, wire::write(&mut launched.writer,
                &json!({"type":"cancel","invocation":call.identity.invocation}))).await;
        }
        let shutdown = timeout(SHUTDOWN, async {
            wire::write(&mut launched.writer, &json!({"type":"shutdown"})).await?;
            let reply: Incoming = decode(wire::read(&mut reader).await?)?;
            ensure!(matches!(reply, Incoming::ShutdownAck), "unexpected extension shutdown reply");
            // No additional frames, partial frames or late responses may follow.
            use tokio::io::AsyncReadExt;
            let mut byte = [0];
            ensure!(reader.read(&mut byte).await? == 0, "late extension response");
            Ok::<_,anyhow::Error>(())
        }).await;
        drop(reader);
        drop(launched.writer);
        let cleanup = timeout(CLEANUP, launched.lease.terminate_and_observe()).await.unwrap_or(Cleanup::Pending);
        let result = if result.is_ok() && !matches!(shutdown, Ok(Ok(()))) {
            Err(anyhow::anyhow!("extension shutdown protocol failed"))
        } else { result };
        Completion { result, cleanup }
    }

    async fn exchange<R: tokio::io::AsyncBufRead + Unpin, W: AsyncWrite + Unpin>(
        &self, call: &Invocation, host: &dyn Host, output: &CompiledSchema, reader: &mut R, writer: &mut W,
    ) -> Result<Value> {
        timeout(INIT, async {
            wire::write(writer, &json!({"type":"initialize","protocol":1,"identity":call.identity,
                "definitions":self.pinned,"capabilities":self.capabilities})).await?;
            match decode(wire::read(reader).await?)? {
                Incoming::Initialized { protocol, identity, definitions, capabilities } => {
                    ensure!(protocol == 1 && identity == call.identity && definitions == self.pinned
                        && capabilities == self.capabilities, "extension initialization mismatch");
                    Ok(())
                }
                _ => bail!("unexpected extension initialization reply"),
            }
        }).await.map_err(|_| anyhow::anyhow!("extension initialization expired"))??;
        wire::write(writer, &json!({"type":"invoke","invocation":call.identity.invocation,
            "kind":call.kind,"name":call.name,"arguments":call.arguments})).await?;
        let mut progress = VecDeque::new();
        let mut progress_count = 0usize;
        let mut requests = 0u32;
        loop {
            match decode(wire::read(reader).await?)? {
                Incoming::Progress { invocation, text } => {
                    ensure!(invocation == call.identity.invocation && text.len() <= 4096, "invalid extension progress");
                    let now = Instant::now();
                    while progress.front().is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1)) { progress.pop_front(); }
                    progress_count += 1;
                    ensure!(progress_count <= 256 && progress.len() < 10, "extension progress rate exceeded");
                    progress.push_back(now);
                    host.progress(&call.identity, &text).await.map_err(|_| anyhow::anyhow!("extension progress refused"))?;
                }
                Incoming::Result { invocation, value } => {
                    ensure!(invocation == call.identity.invocation, "extension result identity mismatch");
                    output.validate(&value)?;
                    return Ok(value);
                }
                Incoming::Error { invocation, code } => {
                    ensure!(invocation == call.identity.invocation, "extension error identity mismatch");
                    return Err(anyhow::anyhow!(match code {
                        ErrorCode::InvalidInput => "extension rejected input",
                        ErrorCode::Failed => "extension reported failure",
                        ErrorCode::Cancelled => "extension reported cancellation",
                    }));
                }
                Incoming::HostRead { invocation, request, path, offset, max_bytes } => {
                    requests += 1;
                    ensure!(invocation == call.identity.invocation && request == requests && requests <= 32,
                        "invalid extension host request identity");
                    ensure!(self.capabilities.iter().any(|c| c == "host.file.read"), "extension host capability denied");
                    ensure!(!path.is_empty() && path.len() <= 4096 && !path.chars().any(char::is_control)
                        && max_bytes > 0 && max_bytes <= MAX_READ, "invalid extension host read bounds");
                    // Any second message while the host request is outstanding is
                    // a protocol violation. Do not buffer unbounded host requests.
                    let read = tokio::select! {
                        biased;
                        _ = tokio::io::AsyncBufReadExt::fill_buf(reader) => bail!("extension sent bytes during outstanding host request"),
                        result = host.read(&call.identity, &path, offset, max_bytes, call.deadline) => result,
                    };
                    let reply = match read {
                        Ok(text) => {
                            ensure!(text.len() <= max_bytes, "extension broker exceeded read bound");
                            json!({"type":"host_result","invocation":invocation,"request":request,"text":text})
                        }
                        Err(_) => json!({"type":"host_error","invocation":invocation,"request":request,"code":"denied"}),
                    };
                    wire::write(writer, &reply).await?;
                }
                _ => bail!("unexpected extension response"),
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Incoming {
    Initialized { protocol: u32, identity: Identity, definitions: Value, capabilities: Vec<String> },
    Progress { invocation: Uuid, text: String },
    Result { invocation: Uuid, value: Value },
    Error { invocation: Uuid, code: ErrorCode },
    #[serde(rename = "host.file.read")]
    HostRead { invocation: Uuid, request: u32, path: String, offset: u64, max_bytes: usize },
    ShutdownAck,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ErrorCode { InvalidInput, Failed, Cancelled }
fn decode(value: Value) -> Result<Incoming> {
    serde_json::from_value(value).map_err(|_| anyhow::anyhow!("invalid extension response"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use tokio::io::{AsyncWriteExt, duplex};

    struct TestLease(Arc<AtomicBool>);
    #[async_trait]
    impl Lease for TestLease {
        async fn terminate_and_observe(&mut self) -> Cleanup {
            self.0.store(true, Ordering::SeqCst);
            Cleanup::Pending // An in-memory stream is not descendant evidence.
        }
    }
    struct Adapter { cleanup: Arc<AtomicBool>, wrong_identity: bool }
    #[async_trait]
    impl LaunchAdapter for Adapter {
        async fn launch(&self, _: &Identity, _: Instant) -> Result<Launched> {
            let (host, peer) = duplex(4096);
            let wrong = self.wrong_identity;
            tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(peer);
                let mut read = BufReader::new(read);
                let mut init = wire::read(&mut read).await.unwrap();
                init["type"] = json!("initialized");
                if wrong { init["protocol"] = json!(2); }
                wire::write(&mut write, &init).await.unwrap();
                if wrong { return; }
                let call = wire::read(&mut read).await.unwrap();
                wire::write(&mut write, &json!({"type":"result", "invocation":call["invocation"], "value":{"ok":true}})).await.unwrap();
                let shutdown = wire::read(&mut read).await.unwrap();
                assert_eq!(shutdown["type"], "shutdown");
                wire::write(&mut write, &json!({"type":"shutdown_ack"})).await.unwrap();
                write.shutdown().await.unwrap();
            });
            let (reader, writer) = tokio::io::split(host);
            Ok(Launched { reader: Box::new(reader), writer: Box::new(writer), lease: Box::new(TestLease(self.cleanup.clone())) })
        }
    }
    struct NoHost;
    #[async_trait]
    impl Host for NoHost {
        async fn read(&self, _: &Identity, _: &str, _: u64, _: usize, _: Instant) -> Result<String> { bail!("denied") }
        async fn progress(&self, _: &Identity, _: &str) -> Result<()> { Ok(()) }
    }
    fn pinned() -> Value {
        json!({"tools":[{"name":"echo","description":"Echo", "input_schema":true,
            "output_schema":{"type":"object","required":["ok"],"properties":{"ok":{"const":true}},"additionalProperties":false}}],"commands":[],"lifecycle":[]})
    }
    fn call() -> Invocation {
        Invocation { identity: Identity { session: Uuid::new_v4(), incarnation: Uuid::new_v4(), run: Uuid::new_v4(), invocation: Uuid::new_v4(),
            package: "example".into(), digest: "a".repeat(64) }, kind: Kind::Tool, name: "echo".into(), arguments: json!({}),
            deadline: Instant::now() + Duration::from_secs(2), cancellation: CancellationToken::new() }
    }
    #[tokio::test]
    async fn confirms_pin_and_retains_cleanup_disposition() {
        for wrong in [false, true] {
            let cleanup = Arc::new(AtomicBool::new(false));
            let executor = Executor::new(pinned(), vec!["execute".into()],
                Arc::new(Adapter { cleanup: cleanup.clone(), wrong_identity: wrong })).unwrap();
            let completion = executor.start(call(), Arc::new(NoHost)).unwrap().completion().await;
            assert_eq!(completion.result.is_err(), wrong);
            assert_eq!(completion.cleanup, Cleanup::Pending);
            assert!(cleanup.load(Ordering::SeqCst));
        }
    }
    #[test]
    fn strict_response_fields_and_codes() {
        assert!(decode(json!({"type":"shutdown_ack","extra":true})).is_err());
        assert!(decode(json!({"type":"error","invocation":Uuid::new_v4(),"code":"private secret"})).is_err());
    }
    struct BlockingAdapter {
        launches: AtomicUsize,
        entered: tokio::sync::Notify,
    }
    #[async_trait]
    impl LaunchAdapter for BlockingAdapter {
        async fn launch(&self, _: &Identity, _: Instant) -> Result<Launched> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            std::future::pending().await
        }
    }
    #[tokio::test]
    async fn queue_is_bounded_and_close_drains_without_late_launches() {
        let adapter = Arc::new(BlockingAdapter { launches: AtomicUsize::new(0), entered: tokio::sync::Notify::new() });
        let executor = Executor::new(pinned(), vec!["execute".into()], adapter.clone()).unwrap();
        let first = executor.start(call(), Arc::new(NoHost)).unwrap();
        timeout(Duration::from_secs(1), adapter.entered.notified()).await.unwrap();
        let queued: Vec<_> = (0..8).map(|_| executor.start(call(), Arc::new(NoHost)).unwrap()).collect();
        assert!(executor.start(call(), Arc::new(NoHost)).is_err());
        drop(first); // Detached supervision retains launch uncertainty.
        assert_eq!(executor.shutdown().await, Cleanup::Pending);
        for handle in queued { assert!(handle.completion().await.result.is_err()); }
        assert_eq!(adapter.launches.load(Ordering::SeqCst), 1);
        assert!(executor.start(call(), Arc::new(NoHost)).is_err());
    }
    #[tokio::test]
    async fn closing_an_unused_executor_has_no_effects() {
        let adapter = Arc::new(BlockingAdapter { launches: AtomicUsize::new(0), entered: tokio::sync::Notify::new() });
        let executor = Executor::new(pinned(), vec!["execute".into()], adapter.clone()).unwrap();
        assert_eq!(executor.shutdown().await, Cleanup::Observed);
        assert_eq!(adapter.launches.load(Ordering::SeqCst), 0);
    }

}
