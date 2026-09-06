//! One bounded input worker per PTY. No writer is held behind the process map,
//! and neither model nor human bytes enter diagnostics. Cancellation stops the
//! remaining suffix; already accepted bytes cannot be rolled back.
use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;

pub(super) const MAX_INPUT_BYTES: usize = 64 * 1024;
const QUEUED_WRITES: usize = 8;
const WRITE_DEADLINE: Duration = Duration::from_millis(250);
const POLL: Duration = Duration::from_millis(5);

#[derive(Clone)]
pub(super) struct Input {
    queue: mpsc::SyncSender<Request>,
    stop: Arc<AtomicBool>,
    private: Arc<AtomicBool>,
}
struct Request {
    model: bool,
    bytes: Vec<u8>,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    reply: oneshot::Sender<Result<(), InputError>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum InputError {
    TooLarge,
    Busy,
    Closed,
    Unconfirmed,
    Private,
}
impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "terminal input exceeds 64 KiB; nothing queued",
            Self::Busy => "terminal input queue full; nothing queued",
            Self::Closed => "terminal input is closed; nothing queued",
            Self::Private => "model input unavailable: terminal is private after human attach; prior input may be partial",
            Self::Unconfirmed => {
                "terminal input delivery incomplete or unconfirmed; do not retry automatically"
            }
        })
    }
}
struct CancelOnDrop(Option<Arc<AtomicBool>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancelled) = &self.0 {
            cancelled.store(true, Ordering::Release);
        }
    }
}
struct Finished(Arc<AtomicBool>);
impl Drop for Finished {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl Input {
    pub(super) fn start(
        mut writer: Box<dyn Write + Send>,
        stop: Arc<AtomicBool>,
        done: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let (queue, requests) = mpsc::sync_channel::<Request>(QUEUED_WRITES);
        let worker_stop = stop.clone();
        let private = Arc::new(AtomicBool::new(false));
        let worker_private = private.clone();
        done.store(false, Ordering::Release);
        let finished = Finished(done);
        std::thread::Builder::new()
            .name("helm-pty-input".into())
            .spawn(move || {
                let _finished = finished;
                while !worker_stop.load(Ordering::Acquire) {
                    let request = match requests.recv_timeout(POLL) {
                        Ok(request) => request,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    };
                    let result = deliver(&mut *writer, &request, &worker_stop, &worker_private);
                    let _ = request.reply.send(result);
                }
            })?;
        Ok(Self {
            queue,
            stop,
            private,
        })
    }
    fn enqueue(
        &self,
        bytes: Vec<u8>,
        model: bool,
    ) -> Result<(oneshot::Receiver<Result<(), InputError>>, CancelOnDrop), InputError> {
        if model && self.private.load(Ordering::Acquire) {
            return Err(InputError::Private);
        }
        if bytes.len() > MAX_INPUT_BYTES {
            return Err(InputError::TooLarge);
        }
        if self.stop.load(Ordering::Acquire) {
            return Err(InputError::Closed);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let guard = CancelOnDrop(Some(cancelled.clone()));
        let (reply, result) = oneshot::channel();
        let request = Request {
            model,
            bytes,
            cancelled,
            deadline: Instant::now() + WRITE_DEADLINE,
            reply,
        };
        self.queue.try_send(request).map_err(|error| match error {
            mpsc::TrySendError::Full(_) => InputError::Busy,
            mpsc::TrySendError::Disconnected(_) => InputError::Closed,
        })?;
        Ok((result, guard))
    }
    pub(super) async fn write(&self, bytes: Vec<u8>) -> Result<(), InputError> {
        self.write_input(bytes, false).await
    }
    pub(super) async fn write_model(&self, bytes: Vec<u8>) -> Result<(), InputError> {
        self.write_input(bytes, true).await
    }
    pub(super) fn make_private(&self) {
        self.private.store(true, Ordering::Release);
    }
    async fn write_input(&self, bytes: Vec<u8>, model: bool) -> Result<(), InputError> {
        let (result, _cancel) = self.enqueue(bytes, model)?;
        tokio::time::timeout(WRITE_DEADLINE, result)
            .await
            .map_err(|_| InputError::Unconfirmed)?
            .map_err(|_| InputError::Unconfirmed)?
    }
    /// Reader-generated terminal protocol replies must never block output capture.
    pub(super) fn report(&self, bytes: Vec<u8>) {
        if let Ok((result, mut guard)) = self.enqueue(bytes, false) {
            // This bounded protocol reply has no caller awaiting it. Its deadline
            // and manager stop flag still apply; dropping a receiver is harmless.
            drop(result);
            guard.0.take();
        }
    }
}
fn deliver(
    writer: &mut dyn Write,
    request: &Request,
    stop: &AtomicBool,
    private: &AtomicBool,
) -> Result<(), InputError> {
    let mut offset = 0;
    loop {
        if request.model && private.load(Ordering::Acquire) {
            return Err(InputError::Private);
        }
        if stop.load(Ordering::Acquire)
            || request.cancelled.load(Ordering::Acquire)
            || Instant::now() >= request.deadline
        {
            return Err(InputError::Unconfirmed);
        }
        if offset == request.bytes.len() {
            return writer.flush().map_err(|_| InputError::Unconfirmed);
        }
        // Small writes bound the suffix that may be accepted by a platform with
        // synchronous native handles before cancellation becomes observable.
        let end = (offset + 1024).min(request.bytes.len());
        match writer.write(&request.bytes[offset..end]) {
            Ok(0) => return Err(InputError::Unconfirmed),
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
            Err(_) => return Err(InputError::Unconfirmed),
        }
    }
}

/// portable-pty duplicates the master descriptor for its reader and writer, so
/// O_NONBLOCK applies to both. The reader handles WouldBlock without ending its
/// capture stream; native writes can then observe deadlines even on a full PTY.
#[cfg(unix)]
pub(super) fn make_nonblocking(master: &dyn portable_pty::MasterPty) -> io::Result<()> {
    let fd = master
        .as_raw_fd()
        .ok_or_else(|| io::Error::other("native terminal descriptor unavailable"))?;
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
