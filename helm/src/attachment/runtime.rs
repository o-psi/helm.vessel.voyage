//! Run-owned integration of the journal and provider-neutral agent loop.
//!
//! This is local-only. Callers must perform authentication/sharing checks before
//! admission and retain the owner until their agent/terminal/subagent cleanup has
//! finished. It does not reconcile the existing JSON SessionStore or enable a port.
use super::journal::{ExecutionGuard, Journal, RunRecord, RunState, TurnAdmission};
use crate::{
    agent::{
        Agent, AgentError, AgentOutcome, CheckpointError, RunCheckpoint, SteeringReceiver,
        SteeringSender, StopReason,
    },
    model::{Message, Usage},
};
use async_trait::async_trait;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
mod steering;
pub use steering::{ManagedSteeringHandle, SteeringAuthorization};

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
    execution_authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    steering_authority: OnceLock<(Arc<dyn SteeringAuthorization>, Arc<dyn RuntimeClock>)>,
    poisoned: AtomicBool,
}
impl TurnToken {
    fn new(run_id: Uuid) -> Self {
        Self {
            run_id,
            execution_authority: None,
            steering_authority: OnceLock::new(),
            poisoned: AtomicBool::new(false),
        }
    }
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
    /// Build a current-grant observer before starting remote execution. Its reads
    /// cooperate with this owner's commits without weakening cross-process fences.
    pub async fn remote_grant_observer(
        &self,
        binding: super::journal::RemoteBinding,
    ) -> anyhow::Result<super::journal::RemoteGrantObserver> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(
                store.turn.upgrade().is_none(),
                "grant observer requires idle owner"
            );
            let session = store.session_id;
            store.journal.remote_grant_observer(binding, session)
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
    /// Poll only this session's exact run on the already-owned connection.
    pub async fn local_cancel_requested(&self, run_id: Uuid) -> anyhow::Result<bool> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            store
                .journal
                .local_cancel_requested(store.session_id, run_id)
        })
        .await?
    }
    /// Explicit operator attestation, distinct from observed resource cleanup.
    pub async fn attest_local_cleanup(
        &self,
        run_id: Uuid,
        actor: super::local_actor::LocalActor,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(store.turn.upgrade().is_none(), "managed turn still owned");
            let Store { journal, guard, .. } = &mut *store;
            journal.attest_local_cleanup(guard, run_id, actor.installation_id, actor.principal_id)
        })
        .await?
    }
    /// Append explicit unknown outcomes under the existing owner, never dispatch tools.
    pub async fn reconcile_local_tools(
        &self,
        request: super::journal::LocalReconcileRequest,
    ) -> anyhow::Result<super::journal::LocalReconcileOutcome> {
        anyhow::ensure!(
            request.session_id == self.session_id,
            "managed owner belongs to another session"
        );
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(store.turn.upgrade().is_none(), "managed turn still owned");
            let Store { journal, guard, .. } = &mut *store;
            journal.reconcile_local_tools(guard, &request)
        })
        .await?
    }
    /// Observe a dedicated remote projection under this session's existing connection.
    pub async fn remote_snapshot(
        &self,
        binding: super::journal::RemoteBinding,
        authority: Arc<dyn crate::policy::ExecutionAuthority>,
    ) -> anyhow::Result<voyage_protocol::stream::Reply> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            authority.check()?;
            let result = store.journal.remote_snapshot(&binding, store.session_id)?;
            authority.check()?;
            Ok(result)
        })
        .await?
    }
    pub async fn remote_replay(
        &self,
        binding: super::journal::RemoteBinding,
        after: u64,
        limit: usize,
        authority: Arc<dyn crate::policy::ExecutionAuthority>,
    ) -> anyhow::Result<super::journal::RemoteReplay> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            authority.check()?;
            let result = store
                .journal
                .remote_replay(&binding, store.session_id, after, limit)?;
            authority.check()?;
            Ok(result)
        })
        .await?
    }
    /// The deadline and authority are sampled after waiting for this owner's connection,
    /// inside the new-receipt transaction; existing receipts still recheck authority.
    pub async fn remote_cancel(
        &self,
        binding: super::journal::RemoteBinding,
        command: voyage_protocol::attachment::Command,
        authority: Arc<dyn crate::policy::ExecutionAuthority>,
    ) -> anyhow::Result<super::journal::RemoteCancelReceipt> {
        self.remote_cancel_clock(binding, command, authority, Arc::new(SystemClock))
            .await
    }
    async fn remote_cancel_clock(
        &self,
        binding: super::journal::RemoteBinding,
        command: voyage_protocol::attachment::Command,
        authority: Arc<dyn crate::policy::ExecutionAuthority>,
        clock: Arc<dyn RuntimeClock>,
    ) -> anyhow::Result<super::journal::RemoteCancelReceipt> {
        anyhow::ensure!(
            matches!(&command.operation, voyage_protocol::attachment::Operation::Cancel {session_id,..} if *session_id == self.session_id),
            "managed owner session mismatch"
        );
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            authority.check()?;
            store
                .journal
                .remote_cancel_with_clock(&binding, &command, || {
                    authority.check()?;
                    clock.now_ms()
                })
        })
        .await?
    }
    pub async fn attest_remote_cleanup(
        &self,
        binding: super::journal::RemoteBinding,
        run_id: Uuid,
        actor: super::local_actor::LocalActor,
    ) -> anyhow::Result<()> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(store.turn.upgrade().is_none(), "managed turn still owned");
            let Store { journal, guard, .. } = &mut *store;
            journal.attest_remote_cleanup(guard, &binding, run_id, &actor)
        })
        .await?
    }
    pub async fn reconcile_remote_tools(
        &self,
        binding: super::journal::RemoteBinding,
        request: super::journal::LocalReconcileRequest,
    ) -> anyhow::Result<super::journal::LocalReconcileOutcome> {
        anyhow::ensure!(
            request.session_id == self.session_id,
            "managed owner session mismatch"
        );
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            anyhow::ensure!(store.turn.upgrade().is_none(), "managed turn still owned");
            let Store { journal, guard, .. } = &mut *store;
            journal.reconcile_remote_tools(guard, &request, &binding)
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
        self.admit_authorized_after(request, clock, before_admission, None)
            .await
    }
    /// Foreground authority is fresh even for observation of an identical receipt.
    pub async fn admit_authorized(
        &self,
        request: TurnAdmission,
        authority: Arc<dyn crate::policy::ExecutionAuthority>,
    ) -> anyhow::Result<Admission> {
        self.admit_authorized_after(request, Arc::new(SystemClock), || Ok(()), Some(authority))
            .await
    }
    async fn admit_authorized_after(
        &self,
        request: TurnAdmission,
        clock: Arc<dyn RuntimeClock>,
        before_admission: impl FnOnce() -> anyhow::Result<()> + Send + 'static,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
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
            if let Some(authority) = &authority {
                authority.check()?;
            }
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
            let admission = journal.admit_turn_with_clock(guard, &request, || {
                if let Some(authority) = &authority {
                    authority.check()?;
                }
                clock.now_ms()
            })?;
            if admission.duplicate {
                return Ok(Admission::Existing(admission.run));
            }
            let run_id = admission.run.id;
            let mut token = TurnToken::new(run_id);
            token.execution_authority = authority;
            let token = Arc::new(token);
            let (steering_sender, steering_receiver) =
                crate::agent::steering_channel(super::journal::MAX_PENDING_STEERING);
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
                steering_sender,
                steering_receiver: Some(steering_receiver),
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
    steering_sender: SteeringSender,
    steering_receiver: Option<SteeringReceiver>,
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
    pub async fn configure_remote_redaction(
        &self,
        redactor: Arc<crate::tools::Redactor>,
    ) -> Result<(), CheckpointError> {
        self.storage(move |store| {
            store
                .journal
                .configure_remote_redaction(&store.guard, store.run_id, redactor)
        })
        .await
    }

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

    /// Persist the restart-safe admission blocker before constructing effectful resources.
    pub async fn register_local_cleanup(&self) -> Result<(), CheckpointError> {
        self.storage(|store| {
            store
                .journal
                .register_local_cleanup(&store.guard, store.run_id)
        })
        .await
    }
    /// Record actual cleanup observation only after the run is terminal.
    pub async fn confirm_local_cleanup_observed(&self) -> Result<(), CheckpointError> {
        self.storage(|store| {
            store
                .journal
                .confirm_local_cleanup_observed(&store.guard, store.run_id)
        })
        .await
    }
    /// Finalize a construction/output failure before execution; never dispatch this owner later.
    pub async fn fail_before_execution(&mut self) -> Result<RunRecord, CheckpointError> {
        self.input.take().ok_or(CheckpointError)?;
        self.steering_receiver.take();
        self.storage(|store| {
            store.journal.finish(
                &store.guard,
                store.run_id,
                RunState::Failed,
                Some("local runtime construction or output failed"),
                None,
            )
        })
        .await
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
        self.execute_before_finish(agent, cancel, input, || Ok(()))
            .await
    }

    async fn execute_before_finish(
        &mut self,
        agent: &Agent,
        cancel: CancellationToken,
        input: Option<SteeringReceiver>,
        before_finish: impl FnOnce() -> Result<(), AgentError>,
    ) -> Result<AgentOutcome, AgentError> {
        let external_input = input.is_some();
        drop(input);
        let input = self.steering_receiver.take();
        let (history, prompt) = self.input.take().ok_or(CheckpointError)?;
        let result = if cancel.is_cancelled() {
            Err(AgentError::Cancelled)
        } else if external_input
            || self.token.poisoned.load(Ordering::SeqCst)
            || agent.workspace() != self.workspace
            || agent.model() != self.model
        {
            Err(CheckpointError.into())
        } else {
            async {
                let authority = self.token.execution_authority.clone();
                self.storage(move |store| {
                    if let Some(authority) = authority {
                        authority.check()?;
                    }
                    store.journal.mark_running(&store.guard, store.run_id)
                })
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
        before_finish()?;
        let durable = self
            .storage(move |store| {
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
        if durable.state == RunState::Cancelled {
            Err(AgentError::Cancelled)
        } else {
            result
        }
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
        let token = self.token.clone();
        self.storage(move |store| {
            if let Some(authority) = &token.execution_authority {
                authority.check()?;
            }
            steering::authorize(store, &token)?;
            store.journal.checkpoint_canonical_with_clock(
                &store.guard,
                store.run_id,
                &messages,
                &usage,
                || match token.steering_authority.get() {
                    Some((_, clock)) => clock.now_ms(),
                    None => SystemClock.now_ms(),
                },
            )
        })
        .await
    }
    async fn accepted(
        &self,
        messages: &[Message],
        usage: &Usage,
        reason: &StopReason,
    ) -> Result<(), CheckpointError> {
        let token = self.token.clone();
        self.storage(move |store| {
            if let Some(authority) = &token.execution_authority {
                authority.check()?;
            }
            steering::authorize(store, &token)?;
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
            let token = self.token.clone();
            self.storage(move |store| {
                if let Some(authority) = &token.execution_authority {
                    authority.check()?;
                }
                steering::authorize(store, &token)?;
                store
                    .journal
                    .accept_checkpoint(&store.guard, store.run_id, &messages, &usage)
            })
            .await?;
        }
        Ok(())
    }
    async fn unstreamed(&self, text: &str) -> Result<(), CheckpointError> {
        self.partial(text).await
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        let text = text.to_owned();
        let token = self.token.clone();
        self.storage(move |store| {
            anyhow::ensure!(
                !token.poisoned.load(Ordering::SeqCst),
                "steering persistence uncertain"
            );
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
    async fn unstreamed(&self, text: &str) -> Result<(), CheckpointError> {
        self.checkpoint().unstreamed(text).await
    }
    async fn partial(&self, text: &str) -> Result<(), CheckpointError> {
        self.checkpoint().partial(text).await
    }
}

#[cfg(test)]
mod tests;
