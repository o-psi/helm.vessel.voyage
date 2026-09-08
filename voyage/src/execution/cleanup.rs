//! Retained cleanup belongs to the original runtime, not to an observer or retrying client.
use super::*;
use crate::attachment::runtime::ManagedRunCheckpoint;
use serde_json::{Value, json};
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::{sync::Mutex, task::JoinHandle, time::Instant};

type Work = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;
const ATTEMPTS: u8 = 3;
const OBSERVATION_BUDGET: Duration = Duration::from_secs(15);

struct Component {
    name: &'static str,
    work: Work,
    task: Option<JoinHandle<bool>>,
    attempts: u8,
    next: Instant,
    observed: bool,
}
impl Component {
    fn new<F, Fut>(name: &'static str, work: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = bool> + Send + 'static,
    {
        Self {
            name,
            work: Arc::new(move || Box::pin(work())),
            task: None,
            attempts: 0,
            next: Instant::now(),
            observed: false,
        }
    }
    async fn advance(&mut self) {
        if self.task.as_ref().is_some_and(JoinHandle::is_finished) {
            self.observed = matches!(self.task.take().unwrap().await, Ok(true));
            self.next = Instant::now() + Duration::from_secs(u64::from(self.attempts) * 2);
        }
        if !self.observed
            && self.task.is_none()
            && self.attempts < ATTEMPTS
            && Instant::now() >= self.next
        {
            self.attempts += 1;
            self.next = Instant::now() + OBSERVATION_BUDGET;
            self.task = Some(tokio::spawn((self.work)()));
        }
    }
    fn blocked(&self) -> bool {
        !self.observed
            && if self.task.is_some() {
                Instant::now() >= self.next
            } else {
                self.attempts >= ATTEMPTS
            }
    }
}

pub struct PendingCleanup {
    checkpoint: ManagedRunCheckpoint,
    // Keep the executor's resource registry and reservation even after a timeout.
    _resources: crate::build::ManagedAgent,
    reservation: crate::host_resources::Reservation,
    watcher: Option<JoinHandle<Result<()>>>,
    monitor_failed: Arc<AtomicBool>,
    components: Vec<Component>,
    progress: Option<Value>,
    started: Instant,
    commits: u8,
    next_commit: Instant,
    completion_pending: &'static str,
}
impl PendingCleanup {
    pub(super) fn new(
        checkpoint: ManagedRunCheckpoint,
        resources: crate::build::ManagedAgent,
        reservation: crate::host_resources::Reservation,
        watcher: JoinHandle<Result<()>>,
        monitor_failed: Arc<AtomicBool>,
        controls: Option<Arc<crate::server::controls::LiveControls>>,
        owner: ManagedSessionOwner,
    ) -> Self {
        let mut components = Vec::new();
        let children = resources.subagents.clone();
        components.push(Component::new("Subordinate tasks", move || {
            let children = children.clone();
            async move {
                children.shutdown().await;
                true
            }
        }));
        let managed = resources.resources.clone();
        components.push(Component::new("Tools and terminals", move || {
            let managed = managed.clone();
            async move {
                match managed {
                    Some(resources) => resources.shutdown_observed(true).await.is_ok(),
                    None => false,
                }
            }
        }));
        if let Some(controls) = controls {
            components.push(Component::new(
                "Runtime controls and retained terminals",
                move || {
                    let controls = controls.clone();
                    let owner = owner.clone();
                    async move {
                        let closed = controls.close().await;
                        let terminals = controls.shutdown_retained(&owner).await.is_ok();
                        closed && terminals
                    }
                },
            ));
        }
        components.push(Component::new("Compatibility processes", || async {
            crate::provider::shutdown_compatibility().await.is_ok()
        }));
        Self {
            checkpoint,
            _resources: resources,
            reservation,
            watcher: Some(watcher),
            monitor_failed,
            components,
            progress: None,
            started: Instant::now(),
            commits: 0,
            next_commit: Instant::now(),
            completion_pending: "Cleanup journal or participant obligations",
        }
    }
    async fn advance(&mut self, owner_references: usize) -> Result<bool> {
        for component in &mut self.components {
            component.advance().await;
        }
        if self.watcher.as_ref().is_some_and(JoinHandle::is_finished) {
            // An error explains the interruption; joining proves the watcher exited.
            // Any detached blocking checkpoint still holds its own turn token below.
            let _ = self.watcher.take().unwrap().await;
        }
        let mut pending = self
            .components
            .iter()
            .filter(|c| !c.observed)
            .map(|c| c.name)
            .collect::<Vec<_>>();
        if self.watcher.is_some() {
            pending.push("Cancellation observation");
        }
        if !self.checkpoint.cleanup_exclusive(owner_references) {
            pending.push("Outstanding run callbacks");
        }
        if pending.is_empty() && self.commits < ATTEMPTS && Instant::now() >= self.next_commit {
            self.commits += 1;
            self.next_commit = Instant::now() + Duration::from_secs(2);
            // Local release is idempotent. A later journal/participant failure retains
            // this same handle and its original reservation identity for another attempt.
            if self.reservation.release_observed().is_err() {
                self.completion_pending = "Host resource accounting";
            } else if self
                .checkpoint
                .finish_cleanup(owner_references)
                .await
                .is_ok()
            {
                // The cleanup journal is authoritative. A diagnostic write failure
                // cannot strand a run whose cleanup was already committed.
                let _ = self.record("observed", &[]).await;
                return Ok(true);
            } else {
                self.completion_pending = "Cleanup journal or participant obligations";
            }
        }
        if pending.is_empty() {
            pending.push(self.completion_pending);
        }
        let blocked = self.components.iter().any(Component::blocked)
            || self.commits >= ATTEMPTS
            || self.started.elapsed() >= Duration::from_secs(60);
        self.record(if blocked { "blocked" } else { "running" }, &pending)
            .await?;
        Ok(false)
    }
    async fn record(&mut self, phase: &str, pending: &[&str]) -> Result<()> {
        let value = json!({"phase":phase,"pending":pending,"retryable":phase != "observed","reason":
            if self.monitor_failed.load(Ordering::Acquire) { Some("Cancellation monitoring failed; the run was interrupted.") } else { None }});
        if self.progress.as_ref() != Some(&value) {
            self.checkpoint.cleanup_progress(value.clone()).await?;
            self.progress = Some(value);
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct CleanupSlot(Mutex<Option<PendingCleanup>>);
impl CleanupSlot {
    pub(super) async fn install(&self, cleanup: PendingCleanup) -> Result<()> {
        let mut current = self.0.lock().await;
        anyhow::ensure!(current.is_none(), "prior run cleanup is still owned");
        *current = Some(cleanup);
        Ok(())
    }
    pub(crate) async fn advance(&self, owner_references: usize) -> Result<bool> {
        let Ok(mut current) = self.0.try_lock() else {
            return Ok(false);
        };
        let Some(cleanup) = current.as_mut() else {
            return Ok(true);
        };
        if cleanup.advance(owner_references).await? {
            *current = None;
            return Ok(true);
        }
        Ok(false)
    }
    /// A new explicit send may request another bounded observation batch. It never
    /// restarts in-flight cleanup tasks or replays the rejected send itself.
    pub(crate) async fn retry(&self) {
        let mut current = self.0.lock().await;
        if let Some(cleanup) = current.as_mut() {
            if cleanup.commits >= ATTEMPTS {
                cleanup.commits = 0;
                cleanup.next_commit = Instant::now();
            }
            cleanup.started = Instant::now();
            for component in &mut cleanup.components {
                if !component.observed && component.task.is_none() && component.attempts >= ATTEMPTS
                {
                    component.attempts = 0;
                    component.next = Instant::now();
                }
            }
        }
    }
    pub(crate) async fn wait(&self, budget: Duration, owner_references: usize) -> bool {
        tokio::time::timeout(budget, async {
            loop {
                if self.advance(owner_references).await.unwrap_or(false) {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .is_ok()
    }
}
