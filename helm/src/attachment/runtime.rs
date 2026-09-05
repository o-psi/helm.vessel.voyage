//! Run-owned integration of the journal and provider-neutral agent loop.
//!
//! This is local-only. Callers must perform authentication/sharing checks before
//! admission and retain the owner until their agent/terminal/subagent cleanup has
//! finished. It does not reconcile the existing JSON SessionStore or enable a port.
use super::journal::{ExecutionGuard, Journal, RunRecord, RunState, TurnAdmission};
use crate::{
    agent::{
        Agent, AgentError, AgentOutcome, CheckpointError, RunCheckpoint, SteeringReceiver,
        StopReason,
    },
    model::{Message, Usage},
};
use async_trait::async_trait;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Trusted runtime time source; never supplied by a remote command or provider.
pub trait RuntimeClock: Send + Sync {
    fn now_ms(&self) -> anyhow::Result<i64>;
}
impl<F> RuntimeClock for F
where
    F: Fn() -> anyhow::Result<i64> + Send + Sync,
{
    fn now_ms(&self) -> anyhow::Result<i64> {
        self()
    }
}
pub struct SystemClock;
impl RuntimeClock for SystemClock {
    fn now_ms(&self) -> anyhow::Result<i64> {
        let elapsed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?;
        Ok(i64::try_from(elapsed.as_millis())?)
    }
}

/// Lifetime owner for one managed session, including idle time and cleanup.
/// No legacy JSON backend is reachable through this owner.
#[derive(Clone)]
pub struct ManagedSessionOwner {
    store: Arc<Mutex<Store>>,
    session_id: Uuid,
}
struct TurnToken {
    run_id: Uuid,
}
struct Store {
    journal: Journal,
    guard: ExecutionGuard,
    session_id: Uuid,
    run_id: Uuid,
    turn: Weak<TurnToken>,
}
impl ManagedSessionOwner {
    pub async fn open(directory: PathBuf, session_id: Uuid) -> anyhow::Result<Self> {
        tokio::task::spawn_blocking(move || {
            let journal = Journal::open(directory)?;
            let guard = journal.acquire_execution(session_id)?;
            Ok(Self {
                store: Arc::new(Mutex::new(Store {
                    journal,
                    guard,
                    session_id,
                    run_id: Uuid::nil(),
                    turn: Weak::new(),
                })),
                session_id,
            })
        })
        .await?
    }
    pub fn session_id(&self) -> Uuid {
        self.session_id
    }
    /// SQLite owns the revision; normalize the returned view, never rewrite disk.
    pub async fn snapshot(&self) -> anyhow::Result<super::journal::VersionedSession> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = store
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            let mut snapshot = store.journal.load_session(store.session_id)?;
            snapshot.session.revision = snapshot.revision;
            Ok(snapshot)
        })
        .await?
    }
    /// New admission cannot overlap an earlier turn's callbacks or cleanup.
    /// Identical retries remain observations and never allocate another executor.
    pub async fn admit(&self, request: TurnAdmission) -> anyhow::Result<Admission> {
        self.admit_with_clock(request, Arc::new(SystemClock)).await
    }
    #[cfg(test)]
    async fn admit_at(&self, request: TurnAdmission, now_ms: i64) -> anyhow::Result<Admission> {
        self.admit_with_clock(request, Arc::new(move || Ok(now_ms)))
            .await
    }
    async fn admit_with_clock(
        &self,
        request: TurnAdmission,
        clock: Arc<dyn RuntimeClock>,
    ) -> anyhow::Result<Admission> {
        self.admit_after(request, clock, || Ok(())).await
    }
    async fn admit_after(
        &self,
        request: TurnAdmission,
        clock: Arc<dyn RuntimeClock>,
        before_admission: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
    ) -> anyhow::Result<Admission> {
        anyhow::ensure!(
            request.session_id == self.session_id,
            "managed owner belongs to another session"
        );
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(
                request.session_id == store.session_id,
                "managed owner session mismatch"
            );
            if let Some(run) = store.journal.lookup_command(&request)? {
                return Ok(Admission::Existing(run));
            }
            anyhow::ensure!(
                store.turn.upgrade().is_none(),
                "managed turn busy; callbacks or cleanup retain ownership"
            );
            before_admission()?;
            let session = store.journal.load_session(store.session_id)?.session;
            let workspace = session.workspace.canonicalize()?;
            let Store { journal, guard, .. } = &mut *store;
            let admission = journal.admit_turn_with_clock(guard, &request, || clock.now_ms())?;
            if admission.duplicate {
                return Ok(Admission::Existing(admission.run));
            }
            let run_id = admission.run.id;
            let token = Arc::new(TurnToken { run_id });
            store.run_id = run_id;
            store.turn = Arc::downgrade(&token);
            drop(store);
            Ok(Admission::New(RunOwner {
                store: shared,
                token,
                run_id,
                workspace,
                model: session.model,
                input: Some((session.messages, request.prompt)),
            }))
        })
        .await?
    }
    /// Explicit recovery only after all prior turn/callback owners are gone.
    pub async fn recover_interrupted(&self) -> anyhow::Result<Option<RunRecord>> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(
                store.turn.upgrade().is_none(),
                "managed turn still owned; finish cleanup before recovery"
            );
            let Store { journal, guard, .. } = &mut *store;
            journal.recover_interrupted(guard)
        })
        .await?
    }
}

/// A retry retrieves existing evidence; it cannot create a runnable owner.
pub enum Admission {
    New(RunOwner),
    Existing(RunRecord),
}

pub struct RunOwner {
    store: Arc<Mutex<Store>>,
    token: Arc<TurnToken>,
    run_id: Uuid,
    workspace: PathBuf,
    model: String,
    input: Option<(Vec<Message>, String)>,
}
/// Clones retain both the session fence and the turn's cleanup exclusion.
#[derive(Clone)]
pub struct ManagedRunCheckpoint {
    store: Arc<Mutex<Store>>,
    token: Arc<TurnToken>,
}
impl ManagedRunCheckpoint {
    async fn storage<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, CheckpointError> {
        let shared = self.store.clone();
        let token = self.token.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared.lock().map_err(|_| CheckpointError)?;
            if store.run_id != token.run_id {
                return Err(CheckpointError);
            }
            let record = store
                .journal
                .run(token.run_id)
                .map_err(|_| CheckpointError)?;
            if record.session_id != store.session_id {
                return Err(CheckpointError);
            }
            operation(&mut store).map_err(|_| CheckpointError)
        })
        .await
        .map_err(|_| CheckpointError)?
    }
}
impl RunOwner {
    /// Convenience entrypoint; duplicates remain readable without execution ownership.
    pub async fn admit(directory: PathBuf, request: TurnAdmission) -> anyhow::Result<Admission> {
        Self::admit_with_clock(directory, request, Arc::new(SystemClock)).await
    }
    #[cfg(test)]
    async fn admit_at(
        directory: PathBuf,
        request: TurnAdmission,
        now_ms: i64,
    ) -> anyhow::Result<Admission> {
        Self::admit_with_clock(directory, request, Arc::new(move || Ok(now_ms))).await
    }
    async fn admit_with_clock(
        directory: PathBuf,
        request: TurnAdmission,
        clock: Arc<dyn RuntimeClock>,
    ) -> anyhow::Result<Admission> {
        let lookup_directory = directory.clone();
        let request = tokio::task::spawn_blocking(move || {
            let journal = Journal::open(lookup_directory)?;
            let existing = journal.lookup_command(&request)?;
            Ok::<_, anyhow::Error>((request, existing))
        })
        .await??;
        let (request, existing) = request;
        if let Some(run) = existing {
            return Ok(Admission::Existing(run));
        }
        ManagedSessionOwner::open(directory, request.session_id)
            .await?
            .admit_with_clock(request, clock)
            .await
    }
    pub fn checkpoint(&self) -> ManagedRunCheckpoint {
        ManagedRunCheckpoint {
            store: self.store.clone(),
            token: self.token.clone(),
        }
    }

    async fn storage<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut Store) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, CheckpointError> {
        self.checkpoint().storage(operation).await
    }

    pub async fn record(&self) -> Result<RunRecord, CheckpointError> {
        self.storage(|store| store.journal.run(store.run_id)).await
    }

    /// Execute at most once. Even after error this owner cannot dispatch again.
    /// The owner remains borrowed/held after return, so its guard is not released
    /// before the caller completes owned-resource cleanup.
    pub async fn execute(
        &mut self,
        agent: &Agent,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
    ) -> Result<AgentOutcome, AgentError> {
        let (history, prompt) = self.input.take().ok_or(CheckpointError)?;
        let result = if cancel.is_cancelled() {
            Err(AgentError::Cancelled)
        } else if agent.workspace() != self.workspace || agent.model() != self.model {
            Err(CheckpointError.into())
        } else {
            async {
                self.storage(|store| store.journal.mark_running(&store.guard, store.run_id))
                    .await?;
                let session = self
                    .storage(|store| {
                        let run = store.journal.run(store.run_id)?;
                        Ok(store.journal.load_session(run.session_id)?.session)
                    })
                    .await?;
                let scope = agent.prepare_run_with_id(&session, self.run_id).await?;
                if let Some(scope) = &scope {
                    let reference = scope.reference();
                    self.storage(move |store| {
                        store
                            .journal
                            .register_run_scope(&store.guard, store.run_id, reference)
                    })
                    .await?;
                }
                agent
                    .run_checkpointed_scoped(
                        history,
                        prompt,
                        cancel.clone(),
                        input,
                        self,
                        self.model.clone(),
                        scope,
                    )
                    .await
            }
            .await
        };
        // Cancellation wins over a late provider completion. A failed terminal
        // commit returns an error, never the otherwise-successful model outcome.
        let result = if cancel.is_cancelled() {
            Err(AgentError::Cancelled)
        } else {
            result
        };
        let (state, reason) = match &result {
            Ok(outcome) => match &outcome.stop_reason {
                StopReason::Completed => (RunState::Completed, None),
                StopReason::Incomplete { .. } => (
                    RunState::Incomplete,
                    Some("completion reconciliation incomplete"),
                ),
            },
            Err(AgentError::Finalization(failure))
                if matches!(&*failure.source, AgentError::Cancelled) =>
            {
                (
                    RunState::Cancelled,
                    Some("run cancelled during completion reconciliation"),
                )
            }
            Err(AgentError::Cancelled) => (RunState::Cancelled, Some("run cancelled")),
            Err(AgentError::Checkpoint(_)) => (RunState::Failed, Some("durable checkpoint failed")),
            Err(_) => (RunState::Failed, Some("provider or runtime failed")),
        };
        let classification = result
            .as_ref()
            .ok()
            .map(|outcome| outcome.stop_reason.clone());
        self.storage(move |store| {
            store.journal.finish_classified(
                &store.guard,
                store.run_id,
                state,
                reason,
                None,
                classification.as_ref(),
            )
        })
        .await?;
        result
    }
}

#[async_trait]
impl RunCheckpoint for ManagedRunCheckpoint {
    fn run_id(&self) -> Uuid {
        self.token.run_id
    }
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        let messages = messages.to_vec();
        let usage = usage.clone();
        self.storage(move |store| {
            store
                .journal
                .checkpoint_canonical(&store.guard, store.run_id, &messages, &usage)
        })
        .await
    }
    async fn accepted(
        &self,
        messages: &[Message],
        usage: &Usage,
        reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        self.storage(|store| {
            anyhow::ensure!(
                store.journal.run(store.run_id)?.state == RunState::Running,
                "acceptance requires the current running turn"
            );
            Ok(())
        })
        .await?;
        if matches!(reason, StopReason::Completed) {
            let messages = messages.to_vec();
            let usage = usage.clone();
            self.storage(move |store| {
                store
                    .journal
                    .accept_checkpoint(&store.guard, store.run_id, &messages, &usage)
            })
            .await?;
        }
        Ok(())
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        let text = text.to_owned();
        self.storage(move |store| {
            store
                .journal
                .append_text(&store.guard, store.run_id, &text)
                .map(|_| ())
        })
        .await
    }
}

#[async_trait]
impl RunCheckpoint for RunOwner {
    fn run_id(&self) -> Uuid {
        self.run_id
    }
    async fn canonical(&self, messages: &[Message], usage: &Usage) -> Result<(), CheckpointError> {
        self.checkpoint().canonical(messages, usage).await
    }
    async fn accepted(
        &self,
        messages: &[Message],
        usage: &Usage,
        reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        self.checkpoint().accepted(messages, usage, reason).await
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        self.checkpoint().partial(text).await
    }
}

#[cfg(test)]
mod tests;
