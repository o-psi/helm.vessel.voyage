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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, atomic::AtomicUsize};
    #[derive(Default)]
    struct State {
        blocked: AtomicBool,
        bytes: Mutex<Vec<u8>>,
        fail: AtomicBool,
        pause_after: AtomicUsize,
    }
    struct Fake(Arc<State>);
    impl Write for Fake {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.0.fail.load(Ordering::Acquire) {
                return Err(io::Error::other("human-canary\x1b[2J"));
            }
            if self.0.blocked.load(Ordering::Acquire) {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            // Force repeated short writes rather than relying on write_all.
            let count = bytes.len().min(3);
            self.0
                .bytes
                .lock()
                .unwrap()
                .extend_from_slice(&bytes[..count]);
            let pause = self.0.pause_after.load(Ordering::Acquire);
            if pause > 0 && self.0.bytes.lock().unwrap().len() >= pause {
                self.0.blocked.store(true, Ordering::Release);
            }
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn fixture() -> (Input, Arc<State>, Arc<AtomicBool>, Arc<AtomicBool>) {
        let state = Arc::new(State::default());
        let stop = Arc::new(AtomicBool::new(false));
        let done = Arc::new(AtomicBool::new(true));
        (
            Input::start(Box::new(Fake(state.clone())), stop.clone(), done.clone()).unwrap(),
            state,
            stop,
            done,
        )
    }
    async fn wait_for(check: impl Fn() -> bool) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while !check() {
                tokio::time::sleep(POLL).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn short_writes_are_fifo_and_oversize_or_full_admission_is_nonmutating() {
        let (input, state, stop, done) = fixture();
        assert_eq!(
            input.write(vec![0; MAX_INPUT_BYTES + 1]).await,
            Err(InputError::TooLarge)
        );
        state.blocked.store(true, Ordering::Release);
        let mut queued = Vec::new();
        loop {
            match input.enqueue(b"must-not-deliver".to_vec(), false) {
                Ok(request) => {
                    queued.push(request);
                    assert!(queued.len() <= QUEUED_WRITES + 1);
                }
                Err(InputError::Busy) => break,
                _ => panic!("unexpected admission result"),
            }
        }
        assert!(state.bytes.lock().unwrap().is_empty());
        drop(queued); // Every cancelled queued suffix must be discarded.
        state.blocked.store(false, Ordering::Release);
        tokio::time::sleep(Duration::from_millis(30)).await;
        input.write(b"first".to_vec()).await.unwrap();
        input.write(b"second".to_vec()).await.unwrap();
        assert_eq!(&*state.bytes.lock().unwrap(), b"firstsecond");
        stop.store(true, Ordering::Release);
        wait_for(|| done.load(Ordering::Acquire)).await;
        assert_eq!(
            input.write(b"closed".to_vec()).await,
            Err(InputError::Closed)
        );
    }
    #[tokio::test]
    async fn deadline_and_dropped_waiter_stop_the_remaining_suffix_and_reports_never_block() {
        let (input, state, stop, done) = fixture();
        state.blocked.store(true, Ordering::Release);
        assert_eq!(
            input.write(b"timed-out".to_vec()).await,
            Err(InputError::Unconfirmed)
        );
        let writer = input.clone();
        let task = tokio::spawn(async move { writer.write(b"abandoned".to_vec()).await });
        tokio::time::sleep(POLL).await;
        task.abort();
        let _ = task.await;
        let before = Instant::now();
        input.report(b"\x1b[1;1R".to_vec());
        assert!(before.elapsed() < Duration::from_millis(20));
        state.blocked.store(false, Ordering::Release);
        wait_for(|| !state.bytes.lock().unwrap().is_empty()).await;
        assert_eq!(&*state.bytes.lock().unwrap(), b"\x1b[1;1R");
        state.fail.store(true, Ordering::Release);
        let error = input.write(b"secret data".to_vec()).await.unwrap_err();
        assert_eq!(error, InputError::Unconfirmed);
        assert!(!error.to_string().contains("canary"));
        assert!(!error.to_string().contains("secret"));
        stop.store(true, Ordering::Release);
        wait_for(|| done.load(Ordering::Acquire)).await;
    }
    #[tokio::test]
    async fn human_takeover_stops_pending_model_suffix_and_denies_new_model_input() {
        let (input, state, stop, done) = fixture();
        state.pause_after.store(3, Ordering::Release);
        let writer = input.clone();
        let pending =
            tokio::spawn(async move { writer.write_model(b"pre-never-deliver".to_vec()).await });
        wait_for(|| state.bytes.lock().unwrap().len() == 3).await;
        input.make_private();
        state.pause_after.store(0, Ordering::Release);
        state.blocked.store(false, Ordering::Release);
        assert_eq!(pending.await.unwrap(), Err(InputError::Private));
        assert_eq!(
            input.write_model(b"denied".to_vec()).await,
            Err(InputError::Private)
        );
        input.write(b"human".to_vec()).await.unwrap();
        assert_eq!(&*state.bytes.lock().unwrap(), b"prehuman");
        stop.store(true, Ordering::Release);
        wait_for(|| done.load(Ordering::Acquire)).await;
    }
}
