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
    pub async fn put_image(
        &self,
        principal: Uuid,
        id: Uuid,
        name: String,
        bytes: Vec<u8>,
        authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    ) -> anyhow::Result<voyage_protocol::content::ImageAttachment> {
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let mut store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            if let Some(authority) = authority {
                authority.check()?;
            }
            let Store { journal, guard, .. } = &mut *store;
            journal.put_image(guard, principal, id, &name, &bytes)
        })
        .await?
    }

    pub async fn lookup_turn(&self, request: TurnAdmission) -> anyhow::Result<Option<RunRecord>> {
        anyhow::ensure!(request.session_id == self.session_id, "session mismatch");
        let shared = self.store.clone();
        tokio::task::spawn_blocking(move || {
            let store = shared
                .lock()
                .map_err(|_| anyhow::anyhow!("managed owner poisoned"))?;
            store.journal.lookup_command(&request)
        })
        .await?
    }

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
    /// New admission cannot overlap an earlier turn's callbacks or cleanup.
    /// Identical retries remain observations and never allocate another executor.
    pub async fn admit(&self, request: TurnAdmission) -> anyhow::Result<Admission> {
        self.admit_with_clock(request, Arc::new(SystemClock)).await
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
            // Admission may atomically consume a deferred next-turn model.
            // Bind the run owner to that committed model, not the pre-admission copy.
            let model = journal.load_session(request.session_id)?.session.model;
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
                model,
                input: Some((session.messages, request.prompt, request.parts)),
                steering_sender,
                steering_receiver: Some(steering_receiver),
                workflow_bindings: None,
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
    workflow_bindings: Option<crate::workflow::secrets::RunBindings>,
    store: Arc<Mutex<Store>>,
    token: Arc<TurnToken>,
    run_id: Uuid,
    workspace: PathBuf,
    model: String,
    input: Option<(
        Vec<Message>,
        String,
        Vec<voyage_protocol::content::ContentPart>,
    )>,
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
    /// Only terminal persistence may retry. Each attempt owns the same guarded
    /// run; no agent, provider, admission, or tool operation is repeated.
    /// The two-second contention budget begins after acquiring the store mutex;
    /// it bounds SQLite retries, not unrelated work already holding that mutex.
    async fn persist_terminal(
        &self,
        state: RunState,
        reason: Option<&'static str>,
        classification: Option<StopReason>,
        title: Option<crate::titles::TitleResult>,
        cancel: CancellationToken,
    ) -> Result<RunRecord, CheckpointError> {
        let shared = self.store.clone();
        let token = self.token.clone();

        tokio::task::spawn_blocking(move || {
            let mut store = shared.lock().map_err(|_| CheckpointError)?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            loop {
                let attempt = (|| -> anyhow::Result<RunRecord> {
                    anyhow::ensure!(store.run_id == token.run_id, "run identity changed");
                    let record = store.journal.run(token.run_id)?;
                    anyhow::ensure!(
                        record.session_id == store.session_id,
                        "session identity changed"
                    );
                    let (state, reason, classification) = if cancel.is_cancelled() {
                        (RunState::Cancelled, Some("run cancelled"), None)
                    } else {
                        (state.clone(), reason, classification.as_ref())
                    };
                    let Store {
                        journal,
                        guard,
                        run_id,
                        ..
                    } = &mut *store;
                    journal.finish_classified_with_title(
                        guard,
                        *run_id,
                        state,
                        reason,
                        None,
                        classification,
                        true,
                        title.as_ref(),
                    )
                })();
                match attempt {
                    Ok(record) => return Ok(record),
                    Err(error) => {
                        if !store.journal.terminal_retry_safe(&error) {
                            return Err(CheckpointError);
                        }

                        let remaining =
                            deadline.saturating_duration_since(std::time::Instant::now());
                        if remaining.is_zero() {
                            return Err(CheckpointError);
                        }
                        // This worker is blocking; never sleep on the async reactor.
                        std::thread::sleep(remaining.min(std::time::Duration::from_millis(10)));
                        if std::time::Instant::now() >= deadline {
                            return Err(CheckpointError);
                        }
                    }
                }
            }
        })
        .await
        .map_err(|_| CheckpointError)?
    }
    /// Convenience entrypoint; duplicates remain readable without execution ownership.
    pub async fn admit(directory: PathBuf, request: TurnAdmission) -> anyhow::Result<Admission> {
        Self::admit_with_clock(directory, request, Arc::new(SystemClock)).await
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
        self.fail_before_execution_reason("local runtime construction or output failed")
            .await
    }
    pub(crate) async fn fail_before_execution_reason(
        &mut self,
        reason: &str,
    ) -> Result<RunRecord, CheckpointError> {
        let reason = reason.to_owned();
        self.input.take().ok_or(CheckpointError)?;
        self.steering_receiver.take();
        self.storage(move |store| {
            store.journal.finish(
                &store.guard,
                store.run_id,
                RunState::Failed,
                Some(&reason),
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
        let (_history, prompt, parts) = self.input.take().ok_or(CheckpointError)?;
        let workflow_bindings = self.workflow_bindings.take();
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
                        let mut session = store.journal.load_session(run.session_id)?.session;
                        store.journal.hydrate_images(&mut session)?;
                        Ok(session)
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
                // Admission already saved this message. Reuse its exact timestamp
                // and trusted metadata instead of recreating accepted history.
                let mut history = session.messages;
                let accepted = history.pop().ok_or(CheckpointError)?;
                if accepted.role != crate::model::Role::User
                    || accepted.content != prompt
                    || accepted.parts != parts
                {
                    return Err(CheckpointError.into());
                }
                agent
                    .run_checkpointed_scoped_with_workflow_secrets(
                        history,
                        accepted,
                        cancel.clone(),
                        input,
                        self,
                        self.model.clone(),
                        scope,
                        workflow_bindings,
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
            Err(error) if error.is_incomplete() => {
                (RunState::Incomplete, Some(error.public_failure_reason()))
            }
            Err(error) => (RunState::Failed, Some(error.public_failure_reason())),
        };
        let classification = result
            .as_ref()
            .ok()
            .map(|outcome| outcome.stop_reason.clone());
        before_finish()?;
        // Use the durable canonical transcript, including the accepted answer
        // and current run attribution. Never hold the journal lock over I/O.
        // This bounded, best-effort request is outside terminal-commit retries.
        let title = if state == RunState::Completed && !cancel.is_cancelled() {
            match self
                .storage(|store| Ok(store.journal.load_session(store.session_id)?.session))
                .await
            {
                Ok(session) if session.title_due_after_turn() => {
                    agent
                        .generate_title_for_session(&session, cancel.clone())
                        .await
                }
                _ => None,
            }
        } else {
            None
        };
        let durable = self
            .persist_terminal(state, reason, classification, title, cancel)
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
    async fn reconciled(
        &self,
        messages: &[Message],
        usage: &Usage,
    ) -> Result<Vec<Message>, CheckpointError> {
        let messages = messages.to_vec();
        let usage = usage.clone();
        let token = self.token.clone();
        self.storage(move |store| {
            if let Some(authority) = &token.execution_authority {
                authority.check()?;
            }
            steering::authorize(store, &token)?;
            let result = store.journal.checkpoint_reconciled_with_clock(
                &store.guard,
                store.run_id,
                &messages,
                &usage,
                || match token.steering_authority.get() {
                    Some((_, clock)) => clock.now_ms(),
                    None => SystemClock.now_ms(),
                },
                true,
            )?;
            let mut session = store.journal.load_session(store.session_id)?.session;
            session.messages = result;
            store.journal.hydrate_images(&mut session)?;
            Ok(session.messages)
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
    async fn reconciled(
        &self,
        messages: &[Message],
        usage: &Usage,
    ) -> Result<Vec<Message>, CheckpointError> {
        self.checkpoint().reconciled(messages, usage).await
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

mod controls;
mod process;

mod decisions;

mod lifecycle;

mod observations;

mod command_binding;

mod rejections;

mod transfer;

mod assignments;

mod configuration;

mod operator;

mod session_resources;

mod cleanup;
