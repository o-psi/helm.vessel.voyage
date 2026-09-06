//! Explicit cleanup observation. Callers retain their execution fence or a durable
//! admission blocker until observation_complete; dropping a manager is not proof.
use super::*;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
pub(crate) mod linux;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalShutdownFailure {
    Busy,
    Poisoned,
    TimedOut,
    WorkerFailed,
    IdentityUnavailable,
    ObservationUnavailable,
    KillFailed,
    WaitFailed,
    ReaderRunning,
    WriterRunning,
}
#[derive(Clone, Debug, Serialize)]
pub struct TerminalShutdown {
    pub observation_complete: bool,
    pub remaining: Vec<TerminalId>,
    pub omitted_remaining: usize,
    pub failures: Vec<TerminalShutdownFailure>,
}
impl TerminalShutdown {
    pub(crate) fn empty() -> Self {
        Self {
            observation_complete: true,
            remaining: Vec::new(),
            omitted_remaining: 0,
            failures: Vec::new(),
        }
    }
    fn failure(failure: TerminalShutdownFailure) -> Self {
        Self {
            observation_complete: false,
            failures: vec![failure],
            ..Self::empty()
        }
    }
}

pub(super) struct OwnedChild {
    host_reservation: crate::host_resources::Reservation,
    inner: Box<dyn Child + Send + Sync>,
    reader_done: Arc<AtomicBool>,
    input_done: Arc<AtomicBool>,
    input_stop: Arc<AtomicBool>,
    reaped: bool,
    #[cfg(target_os = "linux")]
    session_observed: bool,
    #[cfg(target_os = "linux")]
    identity: Option<super::SessionIdentity>,
}
impl OwnedChild {
    fn new(
        inner: Box<dyn Child + Send + Sync>,
        host_reservation: crate::host_resources::Reservation,
    ) -> Self {
        #[cfg(target_os = "linux")]
        let identity = inner
            .process_id()
            .and_then(|pid| super::SessionIdentity::capture(pid).ok());
        Self {
            host_reservation,
            inner,
            reader_done: Arc::new(AtomicBool::new(true)),
            input_done: Arc::new(AtomicBool::new(true)),
            input_stop: Arc::new(AtomicBool::new(false)),
            reaped: false,
            #[cfg(target_os = "linux")]
            session_observed: false,
            #[cfg(target_os = "linux")]
            identity,
        }
    }
    pub(super) fn process_id(&self) -> Option<u32> {
        self.inner.process_id()
    }
    /// Linux keeps the waitable leader until explicit close/shutdown. Its PID
    /// therefore cannot be reused while we identify the owned PTY session.
    pub(super) fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        #[cfg(target_os = "linux")]
        if !self.reaped
            && let Some(identity) = &self.identity
        {
            return identity.exit_status();
        }
        let result = self.inner.try_wait()?;
        self.reaped |= result.is_some();
        Ok(result)
    }
    pub(super) fn observe(&mut self, deadline: Instant) -> Result<bool, TerminalShutdownFailure> {
        self.input_stop.store(true, Ordering::Release);
        #[cfg(target_os = "linux")]
        {
            if !self.session_observed {
                let Some(identity) = &self.identity else {
                    let _ = self.inner.kill();
                    return Err(TerminalShutdownFailure::IdentityUnavailable);
                };
                self.session_observed =
                    identity.observe_empty(deadline)? || identity.kill_and_observe(deadline)?;
                if !self.session_observed {
                    return Ok(false);
                }
            }
            if !self.reaped {
                self.reaped = self
                    .inner
                    .try_wait()
                    .map_err(|_| TerminalShutdownFailure::WaitFailed)?
                    .is_some();
            }
            if self.reaped && !self.reader_done.load(Ordering::Acquire) {
                return Err(TerminalShutdownFailure::ReaderRunning);
            }
            if self.reaped && !self.input_done.load(Ordering::Acquire) {
                return Err(TerminalShutdownFailure::WriterRunning);
            }
            if self.reaped {
                self.host_reservation
                    .release_observed()
                    .map_err(|_| TerminalShutdownFailure::ObservationUnavailable)?;
            }
            Ok(self.reaped)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = deadline;
            if !self.reaped {
                self.inner
                    .kill()
                    .map_err(|_| TerminalShutdownFailure::KillFailed)?;
                self.reaped = self
                    .inner
                    .try_wait()
                    .map_err(|_| TerminalShutdownFailure::WaitFailed)?
                    .is_some();
            }
            Err(TerminalShutdownFailure::ObservationUnavailable)
        }
    }
}
impl OwnedChild {
    pub(super) fn stop_best_effort(&mut self) {
        self.input_stop.store(true, Ordering::Release);
        if !self.reaped {
            // Best effort only. Successful explicit shutdown has already reaped
            // the child and must never signal its potentially recycled number.
            #[cfg(target_os = "linux")]
            if self
                .identity
                .as_ref()
                .is_some_and(|identity| identity.matches_leader())
            {
                terminate_process_group(self.inner.process_id());
            }
            #[cfg(not(target_os = "linux"))]
            if self.inner.try_wait().ok().flatten().is_none() {
                terminate_process_group(self.inner.process_id());
            }
            #[cfg(not(target_os = "linux"))]
            let _ = self.inner.kill();
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        self.stop_best_effort();
        let _ = self.inner.try_wait();
    }
}
/// Failed setup retains the spawned handle so later shutdown can observe it.
pub(super) struct StartupChild {
    id: Uuid,
    child: Option<OwnedChild>,
    pending: Arc<Mutex<BTreeMap<Uuid, OwnedChild>>>,
    uncertain: Arc<AtomicBool>,
}
impl StartupChild {
    pub(super) fn new(
        tool: &ProcessTool,
        child: Box<dyn Child + Send + Sync>,
        reservation: crate::host_resources::Reservation,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            child: Some(OwnedChild::new(child, reservation)),
            pending: tool.pending.clone(),
            uncertain: tool.uncertain.clone(),
        }
    }
    pub(super) fn id(&self) -> Uuid {
        self.id
    }
    pub(super) fn reader_done(&self) -> Arc<AtomicBool> {
        self.child.as_ref().unwrap().reader_done.clone()
    }
    pub(super) fn input_done(&self) -> Arc<AtomicBool> {
        self.child.as_ref().unwrap().input_done.clone()
    }
    pub(super) fn input_stop(&self) -> Arc<AtomicBool> {
        self.child.as_ref().unwrap().input_stop.clone()
    }
    pub(super) fn take(mut self) -> OwnedChild {
        self.child.take().unwrap()
    }
}
impl Drop for StartupChild {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.stop_best_effort();
            match self.pending.lock() {
                Ok(mut pending) => {
                    pending.insert(self.id, child);
                }
                Err(_) => {
                    self.uncertain.store(true, Ordering::SeqCst);
                }
            }
        }
    }
}
pub(super) struct ReaderDone(pub Arc<AtomicBool>);
impl Drop for ReaderDone {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

impl ProcessTool {
    /// Permanently close this manager to new starts, then boundedly observe owned
    /// PTY cleanup. On Linux this reaps direct children and observes no live
    /// members in their original PTY sessions, plus stopped reader threads.
    /// Detached/new-session escape is outside this scope. Other platforms report
    /// unavailable observation for nonempty managers. No captured text is exposed.
    /// Cancellation drops the waiter, not a running blocking observation worker.
    pub async fn shutdown(&self, timeout: Duration) -> TerminalShutdown {
        self.shutting_down.store(true, Ordering::SeqCst);
        // This fast path performs no OS work and makes a zero-budget empty
        // shutdown independent of blocking-pool scheduling.
        if let Ok(_starting) = self.starting.try_lock()
            && let Ok(processes) = self.processes.try_lock()
            && let Ok(pending) = self.pending.try_lock()
            && processes.is_empty()
            && pending.is_empty()
            && !self.uncertain.load(Ordering::SeqCst)
        {
            return TerminalShutdown::empty();
        }
        let Some(deadline) = Instant::now().checked_add(timeout) else {
            return TerminalShutdown::failure(TerminalShutdownFailure::TimedOut);
        };
        let mut last = TerminalShutdown::failure(TerminalShutdownFailure::TimedOut);
        loop {
            let copy = self.clone();
            let worker = tokio::task::spawn_blocking(move || copy.shutdown_step(deadline));
            // Bound OS observation even if a worker is delayed.
            let wait = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(wait, worker).await {
                Ok(Ok(report)) => last = report,
                Ok(Err(_)) => {
                    return TerminalShutdown::failure(TerminalShutdownFailure::WorkerFailed);
                }
                Err(_) => {
                    last.failures.push(TerminalShutdownFailure::TimedOut);
                    return last;
                }
            }
            if last.observation_complete {
                return last;
            }
            if Instant::now() >= deadline {
                if !last.failures.contains(&TerminalShutdownFailure::TimedOut) {
                    last.failures.push(TerminalShutdownFailure::TimedOut);
                }
                return last;
            }
            tokio::time::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(10)),
            )
            .await;
        }
    }
    /// Closing one terminal does not close admission for the whole manager.
    /// An unobserved handle remains counted against the resource limit.
    pub(super) async fn observe_closed(&self, id: Uuid, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let copy = self.clone();
            let worker = tokio::task::spawn_blocking(move || {
                let Ok(mut pending) = copy.pending.try_lock() else {
                    return false;
                };
                let Some(child) = pending.get_mut(&id) else {
                    return true;
                };
                if child.observe(deadline) == Ok(true) {
                    pending.remove(&id);
                    true
                } else {
                    false
                }
            });
            match tokio::time::timeout(deadline.saturating_duration_since(Instant::now()), worker)
                .await
            {
                Ok(Ok(true)) => return,
                _ if Instant::now() >= deadline => return,
                _ => {}
            }
            tokio::time::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        }
    }
    fn shutdown_step(&self, deadline: Instant) -> TerminalShutdown {
        let _starting = match self.starting.try_lock() {
            Ok(lock) => lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                return TerminalShutdown::failure(TerminalShutdownFailure::Busy);
            }
            Err(_) => return TerminalShutdown::failure(TerminalShutdownFailure::Poisoned),
        };
        let mut processes = match self.processes.try_lock() {
            Ok(lock) => lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                return TerminalShutdown::failure(TerminalShutdownFailure::Busy);
            }
            Err(_) => return TerminalShutdown::failure(TerminalShutdownFailure::Poisoned),
        };
        let mut pending = match self.pending.try_lock() {
            Ok(lock) => lock,
            Err(std::sync::TryLockError::WouldBlock) => {
                return TerminalShutdown::failure(TerminalShutdownFailure::Busy);
            }
            Err(_) => return TerminalShutdown::failure(TerminalShutdownFailure::Poisoned),
        };
        let mut failures = BTreeSet::new();
        if self.uncertain.load(Ordering::SeqCst) {
            failures.insert(TerminalShutdownFailure::ObservationUnavailable);
        }
        processes.retain(|id, process| match process.child.observe(deadline) {
            Ok(true) => {
                let _ = self.events.send(TerminalEvent::Removed(TerminalId(*id)));
                false
            }
            Ok(false) => true,
            Err(error) => {
                failures.insert(error);
                true
            }
        });
        pending.retain(|_, child| match child.observe(deadline) {
            Ok(true) => false,
            Ok(false) => true,
            Err(error) => {
                failures.insert(error);
                true
            }
        });
        let count = processes.len().saturating_add(pending.len());
        let remaining = processes
            .keys()
            .chain(pending.keys())
            .take(64)
            .map(|id| TerminalId(*id))
            .collect::<Vec<_>>();
        TerminalShutdown {
            observation_complete: count == 0 && failures.is_empty(),
            omitted_remaining: count.saturating_sub(remaining.len()),
            remaining,
            failures: failures.into_iter().collect(),
        }
    }
}
