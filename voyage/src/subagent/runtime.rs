use super::{AgentBudget, AgentId, AgentPolicy, AgentRecord, AgentStatus, AgentTreeStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};
use thiserror::Error;
use tokio::sync::{Mutex, OwnedSemaphorePermit, RwLock, Semaphore, broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    pub max_concurrency: usize,

    pub event_history: usize,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_concurrency: super::default_concurrency(),

            event_history: 2048,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SpawnRequest {
    pub parent_id: Option<AgentId>,
    pub name: String,
    pub task: String,
    pub policy: AgentPolicy,
    pub budget: AgentBudget,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentResult {
    pub summary: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum InboxMessage {
    Message(String),
    FollowUp(String),
}

pub struct ExecutionContext {
    pub completion: Option<crate::completion::runtime::RunHandle>,
    pub id: AgentId,
    pub task: String,
    pub policy: AgentPolicy,
    pub budget: AgentBudget,
    pub worktree: Option<PathBuf>,
    pub cancellation: CancellationToken,
    inbox: mpsc::Receiver<InboxMessage>,
    reporter: ProgressReporter,
}
impl ExecutionContext {
    pub async fn recv(&mut self) -> Option<InboxMessage> {
        self.inbox.recv().await
    }
    pub fn try_recv(&mut self) -> Result<InboxMessage, mpsc::error::TryRecvError> {
        self.inbox.try_recv()
    }
    pub fn take_inbox(&mut self) -> mpsc::Receiver<InboxMessage> {
        let (_sender, receiver) = mpsc::channel(1);
        std::mem::replace(&mut self.inbox, receiver)
    }
    /// Forward only local inference warnings through the existing child progress channel.
    pub fn inference_warning_sink(&self) -> Arc<dyn crate::agent::EventSink> {
        Arc::new(InferenceWarnings(self.reporter.clone()))
    }
    pub async fn progress(&self, text: impl Into<String>) {
        self.reporter.report(text.into()).await;
    }
}

#[async_trait]
pub trait SubagentExecutor: Send + Sync + 'static {
    async fn execute(&self, context: ExecutionContext) -> Result<SubagentResult, String>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubagentEvent {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub agent_id: AgentId,
    pub kind: SubagentEventKind,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SubagentEventKind {
    Queued,
    Started,
    Progress { text: String },
    MessageQueued,
    FollowUpQueued,
    Completed { result: SubagentResult },
    Failed { error: String },
    TimedOut,
    Cancelled,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("unknown subagent {0}")]
    Unknown(AgentId),
    #[error("subagent {0} is no longer active")]
    Terminal(AgentId),
    #[error("invalid request: {0}")]
    Invalid(String),
    #[error("persistence failed: {0}")]
    Persistence(String),
    #[error("subagent result channel closed")]
    ResultChannelClosed,
}

#[derive(Clone)]
pub struct SubagentRuntime {
    inner: Arc<Inner>,
}
struct Inner {
    executor: Arc<dyn SubagentExecutor>,
    tasks: tokio_util::task::TaskTracker,
    limits: RuntimeLimits,
    permits: Arc<Semaphore>,
    agents: RwLock<BTreeMap<AgentId, Arc<Control>>>,
    history: Mutex<super::history::EventHistory>,

    events: broadcast::Sender<SubagentEvent>,
    store: Option<AgentTreeStore>,
    mutations: Mutex<()>,
    archive: RwLock<BTreeMap<AgentId, super::ArchivedAgent>>,
}
// Dropping a wait must not let a worker continue without an execution slot.
struct CancelDroppedWait(Option<CancellationToken>);
impl Drop for CancelDroppedWait {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}

struct Control {
    completion: Option<crate::completion::runtime::RunHandle>,
    record: RwLock<AgentRecord>,
    cancel: CancellationToken,
    inbox: mpsc::Sender<InboxMessage>,
    outcome: watch::Sender<Option<Result<SubagentResult, String>>>,
    permit: Mutex<Option<OwnedSemaphorePermit>>,
    waiting_for: Mutex<Vec<AgentId>>,
}
#[derive(Clone)]
struct ProgressReporter {
    runtime: SubagentRuntime,
    id: AgentId,
}
impl ProgressReporter {
    async fn report(&self, text: String) {
        self.runtime.record_progress(self.id, text).await;
    }
}

struct InferenceWarnings(ProgressReporter);
#[async_trait]
impl crate::agent::EventSink for InferenceWarnings {
    async fn emit(&self, event: crate::agent::AgentEvent) {
        if matches!(event, crate::agent::AgentEvent::InferenceWarning(_)) {
            self.0
                .report(
                    "Local inference allowance warning; inspect helm inference for current counts"
                        .into(),
                )
                .await;
        }
    }
}

fn terminal_outcome(record: &AgentRecord) -> Option<Result<SubagentResult, String>> {
    match record.status {
        AgentStatus::Completed => Some(Ok(SubagentResult {
            summary: record.result.clone().unwrap_or_default(),
        })),
        AgentStatus::Failed
        | AgentStatus::Cancelled
        | AgentStatus::Interrupted
        | AgentStatus::TimedOut => Some(Err(record
            .error
            .clone()
            .unwrap_or_else(|| format!("subagent ended with status {:?}", record.status)))),
        AgentStatus::Queued | AgentStatus::Running | AgentStatus::Waiting => None,
    }
}

impl SubagentRuntime {
    pub fn new(
        executor: Arc<dyn SubagentExecutor>,
        limits: RuntimeLimits,
        mut store: Option<AgentTreeStore>,
    ) -> Result<Self, RuntimeError> {
        if limits.max_concurrency == 0
            || limits.max_concurrency > Semaphore::MAX_PERMITS
            || limits.event_history == 0
            || limits.event_history > super::history::MAX_EVENTS
        {
            return Err(RuntimeError::Invalid(
                "runtime concurrency must be positive and event history must be 1–4096".into(),
            ));
        }
        if let Some(store) = &mut store {
            store
                .acquire_runtime_owner()
                .map_err(|e| RuntimeError::Persistence(e.to_string()))?;
        }
        let history = super::history::EventHistory::open(
            store.as_ref().map(|s| s.history_path()).as_deref(),
            limits.event_history,
            store.as_ref().is_some_and(|s| s.has_history_evidence()),
        );
        let (events, _) = broadcast::channel(limits.event_history.clamp(16, 4096));
        Ok(Self {
            inner: Arc::new(Inner {
                executor,
                tasks: tokio_util::task::TaskTracker::new(),
                permits: Arc::new(Semaphore::new(limits.max_concurrency)),
                limits,
                agents: RwLock::new(BTreeMap::new()),
                history: Mutex::new(history),

                events,
                store,
                mutations: Mutex::new(()),
                archive: RwLock::new(BTreeMap::new()),
            }),
        })
    }

    /// Opens a durable runtime and explicitly reconciles executions left live by a prior process.
    pub async fn new_persistent(
        executor: Arc<dyn SubagentExecutor>,
        limits: RuntimeLimits,
        store: AgentTreeStore,
    ) -> Result<Self, RuntimeError> {
        // Acquire exclusive writer lifetime before reading or recovering records.
        // Store clones retain this lease through detached persistence operations.
        let store = tokio::task::spawn_blocking(move || {
            let mut store = store;
            store.acquire_runtime_owner()?;
            Ok::<_, anyhow::Error>(store)
        })
        .await
        .map_err(|e| RuntimeError::Persistence(e.to_string()))?
        .map_err(|e| RuntimeError::Persistence(e.to_string()))?;
        // Validate known ownership before recovery can archive retained records.
        // A missing/corrupt ledger must never silently turn owned work into legacy.
        for record in store
            .list()
            .await
            .map_err(|e| RuntimeError::Persistence(e.to_string()))?
        {
            if let Some(reference) = &record.completion {
                let coordinator = store.coordinator().ok_or_else(|| {
                    RuntimeError::Persistence("owned agent requires completion coordinator".into())
                })?;
                let run = crate::completion::runtime::RunHandle::resume(
                    coordinator.clone(),
                    reference.session_id,
                    reference.run_id,
                )
                .await
                .map_err(|e| RuntimeError::Persistence(e.to_string()))?;
                if !run
                    .owns(crate::completion::Obligation::Agent(record.id))
                    .await
                    .map_err(|e| RuntimeError::Persistence(e.to_string()))?
                {
                    return Err(RuntimeError::Persistence(
                        "agent owner ledger is missing its obligation".into(),
                    ));
                }
            }
        }
        store
            .recover_after_restart()
            .await
            .map_err(|error| RuntimeError::Persistence(error.to_string()))?;
        store
            .prune_terminal_leaves(0, None)
            .await
            .map_err(|error| RuntimeError::Persistence(error.to_string()))?;
        let records = store
            .list()
            .await
            .map_err(|error| RuntimeError::Persistence(error.to_string()))?;
        let runtime = tokio::task::spawn_blocking(move || Self::new(executor, limits, Some(store)))
            .await
            .map_err(|_| {
                RuntimeError::Persistence("history initialization task failed".into())
            })??;
        let mut agents = runtime.inner.agents.write().await;
        for record in records {
            let completion = match (
                &record.completion,
                runtime.inner.store.as_ref().and_then(|s| s.coordinator()),
            ) {
                (Some(reference), Some(coordinator)) => Some(
                    crate::completion::runtime::RunHandle::resume(
                        coordinator.clone(),
                        reference.session_id,
                        reference.run_id,
                    )
                    .await
                    .map_err(|e| RuntimeError::Persistence(e.to_string()))?,
                ),
                (Some(_), None) => {
                    return Err(RuntimeError::Persistence(
                        "owned agent requires completion coordinator".into(),
                    ));
                }
                (None, _) => None,
            };
            let (inbox, _) = mpsc::channel(1);
            let initial = terminal_outcome(&record);
            let (outcome, _) = watch::channel(initial);
            agents.insert(
                record.id,
                Arc::new(Control {
                    completion,
                    record: RwLock::new(record),
                    cancel: CancellationToken::new(),
                    inbox,
                    outcome,
                    permit: Mutex::new(None),
                    waiting_for: Mutex::new(Vec::new()),
                }),
            );
        }
        drop(agents);
        Ok(runtime)
    }

    pub fn store(&self) -> Option<AgentTreeStore> {
        self.inner.store.clone()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<SubagentEvent> {
        self.inner.events.subscribe()
    }
    pub async fn replay_history(
        &self,
        after: Option<super::HistoryCursor>,
    ) -> super::HistoryReplay {
        self.inner.history.lock().await.replay(after)
    }

    pub async fn events_after(&self, sequence: u64) -> Vec<SubagentEvent> {
        self.inner
            .history
            .lock()
            .await
            .events
            .iter()
            .filter(|e| e.sequence > sequence)
            .cloned()
            .collect()
    }
    pub async fn get(&self, id: AgentId) -> Result<AgentRecord, RuntimeError> {
        match self.get_retained(id).await {
            Ok(record) => Ok(record),
            Err(RuntimeError::Unknown(_)) => self
                .archived(id)
                .await?
                .map(|value| value.record)
                .ok_or(RuntimeError::Unknown(id)),
            Err(error) => Err(error),
        }
    }
    pub async fn is_archived(&self, id: AgentId) -> Result<bool, RuntimeError> {
        if self.inner.agents.read().await.contains_key(&id) {
            return Ok(false);
        }
        Ok(self.archived(id).await?.is_some())
    }
    pub async fn get_retained(&self, id: AgentId) -> Result<AgentRecord, RuntimeError> {
        let c = self.control(id).await?;
        let record = c.record.read().await.clone();
        Ok(record)
    }
    async fn archived(&self, id: AgentId) -> Result<Option<super::ArchivedAgent>, RuntimeError> {
        if let Some(store) = &self.inner.store {
            store
                .get_archived(id)
                .await
                .map_err(|e| RuntimeError::Persistence(e.to_string()))
        } else {
            Ok(self.inner.archive.read().await.get(&id).cloned())
        }
    }
    pub async fn list_archived(
        &self,
        after: Option<AgentId>,
        limit: usize,
    ) -> Result<super::ArchivePage, RuntimeError> {
        super::ArchivePage::validate_limit(limit)
            .map_err(|e| RuntimeError::Invalid(e.to_string()))?;
        if let Some(store) = &self.inner.store {
            return store
                .list_archived(after, limit)
                .await
                .map_err(|e| RuntimeError::Persistence(e.to_string()));
        }
        let archive = self.inner.archive.read().await;
        let mut records = archive
            .iter()
            .filter(|(id, _)| after.is_none_or(|cursor| **id > cursor));
        let agents: Vec<_> = records
            .by_ref()
            .take(limit)
            .map(|(_, value)| value.clone().into())
            .collect();
        let next_after = if records.next().is_some() {
            agents
                .last()
                .map(|record: &super::archive::ArchiveSummary| record.id)
        } else {
            None
        };
        Ok(super::ArchivePage { agents, next_after })
    }
    pub async fn list(&self) -> Vec<AgentRecord> {
        let controls: Vec<_> = self.inner.agents.read().await.values().cloned().collect();
        let mut out = Vec::with_capacity(controls.len());
        for c in controls {
            out.push(c.record.read().await.clone());
        }
        out.sort_by_key(|r| r.created_at);
        out
    }
    pub async fn tree(&self, parent: Option<AgentId>) -> Vec<AgentRecord> {
        self.list()
            .await
            .into_iter()
            .filter(|r| r.parent_id == parent)
            .collect()
    }

    /// Observe one run only after terminal workers have finished their durable
    /// transitions. In-memory terminal status precedes persistence and is not
    /// sufficient evidence; failed or missing durable records remain pending.
    pub(crate) async fn pending_owned_shutdown(
        &self,
        reference: &crate::completion::runtime::RunReference,
    ) -> Result<Vec<AgentId>, RuntimeError> {
        let _mutation = self.inner.mutations.lock().await;
        let store = self.inner.store.as_ref().ok_or_else(|| {
            RuntimeError::Invalid("owned shutdown requires persistent agent state".into())
        })?;
        let tree = store
            .load()
            .await
            .map_err(|error| RuntimeError::Persistence(error.to_string()))?;
        let mut pending = BTreeSet::new();
        for record in tree.agents.values() {
            if record.completion.as_ref() == Some(reference) && !record.status.is_terminal() {
                pending.insert(record.id);
            }
        }
        let controls: Vec<_> = self.inner.agents.read().await.values().cloned().collect();
        for control in controls {
            let record = control.record.read().await.clone();
            if record.completion.as_ref() != Some(reference) {
                continue;
            }
            let durable = match tree.agents.get(&record.id) {
                Some(record) => Some(record.clone()),
                None => store
                    .get_archived(record.id)
                    .await
                    .map_err(|error| RuntimeError::Persistence(error.to_string()))?
                    .map(|archive| archive.record),
            };
            if !record.status.is_terminal()
                || control.outcome.borrow().is_none()
                || !durable.is_some_and(|record| {
                    record.completion.as_ref() == Some(reference) && record.status.is_terminal()
                })
            {
                pending.insert(record.id);
            }
        }
        Ok(pending.into_iter().collect())
    }

    /// Stop accepting work and drain cancelled workers before a frontend handoff.
    /// Callers may bound this wait, but must not release or bypass the writer
    /// lease if draining fails. Tracked workers finish persistence before exit.
    pub async fn shutdown(&self) {
        {
            let _mutation = self.inner.mutations.lock().await;
            self.inner.permits.close();
            for control in self.inner.agents.read().await.values() {
                control.cancel.cancel();
            }
            self.inner.tasks.close();
        }
        self.inner.tasks.wait().await;
    }

    pub async fn spawn(&self, request: SpawnRequest) -> Result<AgentId, RuntimeError> {
        self.spawn_for_run(request, None).await
    }
    pub async fn spawn_for_run(
        &self,
        request: SpawnRequest,
        mut completion: Option<crate::completion::runtime::RunHandle>,
    ) -> Result<AgentId, RuntimeError> {
        let _mutation = self.inner.mutations.lock().await;
        if self.inner.permits.is_closed() {
            return Err(RuntimeError::Invalid("runtime is shutting down".into()));
        }
        if request.task.trim().is_empty() || request.name.trim().is_empty() {
            return Err(RuntimeError::Invalid(
                "name and task cannot be empty".into(),
            ));
        }
        if !request.policy.budget.allows(&request.budget) {
            return Err(RuntimeError::Invalid(
                "execution budget exceeds child policy".into(),
            ));
        }
        if let Some(parent) = request.parent_id {
            let parent_record = self.get_retained(parent).await?;
            if !parent_record.budget.allows(&request.budget) {
                return Err(RuntimeError::Invalid(
                    "execution budget exceeds parent budget".into(),
                ));
            }
            parent_record
                .policy
                .validate_child(&request.policy)
                .map_err(|e| RuntimeError::Invalid(e.to_string()))?;
        }
        if let Some(parent) = request.parent_id {
            let inherited = self.control(parent).await?.completion.clone();
            match (&completion, inherited) {
                (Some(requested), Some(inherited))
                    if requested.reference() != inherited.reference() =>
                {
                    if !requested
                        .owns(crate::completion::Obligation::Agent(parent))
                        .await
                        .map_err(|e| RuntimeError::Persistence(e.to_string()))?
                    {
                        return Err(RuntimeError::Invalid(
                            "explicitly adopt parent before creating a child in another run".into(),
                        ));
                    }
                }
                (_, Some(inherited)) => completion = Some(inherited),
                (Some(requested), None) => {
                    if !requested
                        .owns(crate::completion::Obligation::Agent(parent))
                        .await
                        .map_err(|e| RuntimeError::Persistence(e.to_string()))?
                    {
                        return Err(RuntimeError::Invalid(
                            "explicitly adopt legacy parent before creating owned descendants"
                                .into(),
                        ));
                    }
                }
                (None, None) => (),
            }
        }
        self.prune_terminal_history(0, request.parent_id).await?;
        let cancellation = if let Some(parent) = request.parent_id {
            let parent = self.control(parent).await?;
            let same_run = parent.completion.as_ref().map(|run| run.reference())
                == completion.as_ref().map(|run| run.reference());
            if same_run {
                if parent.cancel.is_cancelled() {
                    return Err(RuntimeError::Invalid(
                        "cannot spawn from a cancelled parent".into(),
                    ));
                }
                parent.cancel.child_token()
            } else {
                if !parent.record.read().await.status.is_terminal() {
                    return Err(RuntimeError::Invalid(
                        "cross-run followup requires a terminal adopted parent".into(),
                    ));
                }
                // Explicit adoption was validated above. New-run followups of
                // terminal work must not inherit the old run's cancellation.
                CancellationToken::new()
            }
        } else {
            CancellationToken::new()
        };
        let mut agents = self.inner.agents.write().await;
        let id = AgentId::new();
        if let Some(run) = &completion {
            if !self
                .inner
                .store
                .as_ref()
                .and_then(|s| s.coordinator())
                .is_some_and(|c| c.same(run.coordinator()))
            {
                return Err(RuntimeError::Invalid(
                    "owned subagent requires coordinated persistent store".into(),
                ));
            }
            run.register(crate::completion::Obligation::Agent(id))
                .await
                .map_err(|e| RuntimeError::Persistence(e.to_string()))?;
        }
        let now = Utc::now();
        let record = AgentRecord {
            completion: completion.as_ref().map(|run| run.reference()),
            id,
            parent_id: request.parent_id,
            name: request.name.clone(),
            task: request.task.clone(),
            status: AgentStatus::Queued,
            policy: request.policy.clone(),
            budget: request.budget.clone(),
            worktree: request.worktree.clone(),
            branch: request.branch.clone(),
            created_at: now,
            started_at: None,
            finished_at: None,
            updated_at: now,
            recent_progress: vec![],
            result: None,
            error: None,
        };
        if let Some(store) = &self.inner.store {
            store
                .create(record.clone())
                .await
                .map_err(|e| RuntimeError::Persistence(e.to_string()))?;
        }
        let (inbox_tx, inbox_rx) = mpsc::channel(64);
        let (outcome, _) = watch::channel(None);
        let control = Arc::new(Control {
            completion,
            record: RwLock::new(record),
            cancel: cancellation,
            inbox: inbox_tx,
            outcome,
            permit: Mutex::new(None),
            waiting_for: Mutex::new(Vec::new()),
        });
        agents.insert(id, control.clone());
        drop(agents);
        self.emit(id, SubagentEventKind::Queued).await;
        let runtime = self.clone();
        self.inner.tasks.spawn(async move {
            runtime.run(id, inbox_rx, control).await;
        });
        Ok(id)
    }

    async fn prune_terminal_history(
        &self,
        max_records: usize,
        protected: Option<AgentId>,
    ) -> Result<(), RuntimeError> {
        let records = self.list().await;
        if records.len() <= max_records {
            return Ok(());
        }
        let mut tree = super::AgentTree::default();
        for record in records {
            tree.agents.insert(record.id, record);
        }
        let original = tree.clone();
        let removable = tree.prune_terminal_leaves(max_records, protected);
        if removable.is_empty() {
            return Ok(());
        }
        let removed = if let Some(store) = &self.inner.store {
            store
                .prune_terminal_leaves(max_records, protected)
                .await
                .map_err(|error| RuntimeError::Persistence(error.to_string()))?
        } else {
            let mut archive = self.inner.archive.write().await;
            for id in &removable {
                archive.insert(*id, super::ArchivedAgent::new(original.agents[id].clone()));
            }
            removable
        };
        let mut agents = self.inner.agents.write().await;
        for id in removed {
            agents.remove(&id);
        }
        Ok(())
    }

    async fn run(&self, id: AgentId, inbox: mpsc::Receiver<InboxMessage>, control: Arc<Control>) {
        let permit = tokio::select! { biased; _=control.cancel.cancelled()=>{self.finish_cancelled(id,&control).await;return}, p=self.inner.permits.clone().acquire_owned()=>match p {Ok(p)=>p,Err(_)=>{self.finish_failed(id,&control,"runtime shut down".into()).await;return}} };
        *control.permit.lock().await = Some(permit);
        {
            let mut r = control.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.status = AgentStatus::Running;
            r.started_at = Some(Utc::now());
            r.updated_at = Utc::now();
        }
        self.persist(&control).await;
        self.emit(id, SubagentEventKind::Started).await;
        let request = control.record.read().await.clone();
        let context = ExecutionContext {
            completion: control.completion.clone(),
            id,
            task: request.task,
            policy: request.policy,
            budget: request.budget.clone(),
            worktree: request.worktree,
            cancellation: control.cancel.clone(),
            inbox,
            reporter: ProgressReporter {
                runtime: self.clone(),
                id,
            },
        };
        enum End {
            Cancelled,
            Result(Result<SubagentResult, String>),
        }
        let result = tokio::select! {
            biased;
            _=control.cancel.cancelled()=>End::Cancelled,
            r=self.inner.executor.execute(context)=>End::Result(r)
        };
        // Provider cancellation may make its future return an error in the same
        // poll as the token becomes ready. Preserve the operator's cancellation.
        let result = if control.cancel.is_cancelled() {
            End::Cancelled
        } else {
            result
        };
        match result {
            End::Cancelled => self.finish_cancelled(id, &control).await,
            End::Result(Ok(value)) => self.finish_completed(id, &control, value).await,
            End::Result(Err(error)) => self.finish_failed(id, &control, error).await,
        }
    }

    /// A worker must yield its execution slot while waiting, then reacquire it
    /// before returning to its model loop. External supervisors own no slot.
    pub async fn wait_many_as(
        &self,
        caller: Option<AgentId>,
        ids: &[AgentId],
    ) -> Result<Vec<Result<SubagentResult, String>>, RuntimeError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.yield_slot(caller, ids, self.wait_many(ids)).await
    }

    async fn yield_slot<T>(
        &self,
        caller: Option<AgentId>,
        ids: &[AgentId],
        operation: impl std::future::Future<Output = Result<T, RuntimeError>>,
    ) -> Result<T, RuntimeError> {
        let Some(caller) = caller else {
            return operation.await;
        };
        let control = self.control(caller).await?;
        let mut dropped_wait = CancelDroppedWait(None);
        {
            let _mutation = self.inner.mutations.lock().await;
            // Validate before surrendering the slot. Walk existing wait edges as
            // well as ancestry so mutual sibling waits cannot form a cycle.
            let mut ancestors = std::collections::BTreeSet::from([caller]);
            let mut parent = control.record.read().await.parent_id;
            while let Some(id) = parent {
                if !ancestors.insert(id) {
                    break;
                }
                parent = self.get(id).await?.parent_id;
            }
            for id in ids {
                let target = self.get(*id).await?;
                if !target.status.is_terminal() && ancestors.contains(id) {
                    return Err(RuntimeError::Invalid(
                        "cannot wait for self or an ancestor".into(),
                    ));
                }
                let mut pending = vec![*id];
                let mut visited = std::collections::BTreeSet::new();
                while let Some(next) = pending.pop() {
                    if next == caller {
                        return Err(RuntimeError::Invalid("cyclic subagent wait".into()));
                    }
                    if visited.insert(next)
                        && let Ok(other) = self.control(next).await
                    {
                        pending.extend(other.waiting_for.lock().await.iter().copied());
                    }
                }
            }
            if !control.waiting_for.lock().await.is_empty() {
                return Err(RuntimeError::Invalid("agent is already waiting".into()));
            }
            if control.cancel.is_cancelled() {
                return Err(RuntimeError::Terminal(caller));
            }
            if control.record.read().await.status != AgentStatus::Running
                || control.permit.lock().await.is_none()
            {
                return Err(RuntimeError::Invalid(
                    "only an executing agent can yield its slot".into(),
                ));
            }
            dropped_wait.0 = Some(control.cancel.clone());
            *control.waiting_for.lock().await = ids.to_vec();
            let mut record = control.record.write().await;
            record.status = AgentStatus::Waiting;
            record.updated_at = Utc::now();
            drop(record);
            control.permit.lock().await.take();
        }
        let result = tokio::select! {
            biased;
            _ = control.cancel.cancelled() => Err(RuntimeError::Terminal(caller)),
            result = operation => result,
        };
        control.waiting_for.lock().await.clear();
        let permit = tokio::select! {
            biased;
            _ = control.cancel.cancelled() => return Err(RuntimeError::Terminal(caller)),
            permit = self.inner.permits.clone().acquire_owned() => permit.map_err(|_| RuntimeError::Terminal(caller))?,
        };
        let _mutation = self.inner.mutations.lock().await;
        if control.cancel.is_cancelled() || control.record.read().await.status.is_terminal() {
            return Err(RuntimeError::Terminal(caller));
        }
        *control.permit.lock().await = Some(permit);
        control.waiting_for.lock().await.clear();
        let mut record = control.record.write().await;
        if !record.status.is_terminal() {
            record.status = AgentStatus::Running;
            record.updated_at = Utc::now();
        }
        dropped_wait.0 = None;
        result
    }

    pub async fn wait(&self, id: AgentId) -> Result<Result<SubagentResult, String>, RuntimeError> {
        let c = match self.control(id).await {
            Ok(control) => control,
            Err(RuntimeError::Unknown(_)) => {
                return terminal_outcome(&self.get(id).await?).ok_or(RuntimeError::Unknown(id));
            }
            Err(error) => return Err(error),
        };
        let mut rx = c.outcome.subscribe();
        loop {
            if let Some(value) = rx.borrow().clone() {
                return Ok(value);
            }
            rx.changed()
                .await
                .map_err(|_| RuntimeError::ResultChannelClosed)?;
        }
    }
    pub async fn cancel(&self, id: AgentId) -> Result<(), RuntimeError> {
        let c = match self.control(id).await {
            Ok(c) => c,
            Err(RuntimeError::Unknown(_)) if self.archived(id).await?.is_some() => return Ok(()),
            Err(error) => return Err(error),
        };
        // Child tokens inherit cancellation at spawn, including concurrent
        // descendants. This avoids recursion and repeated full-tree snapshots.
        c.cancel.cancel();
        Ok(())
    }
    pub(crate) async fn worktree_record(&self, id: AgentId) -> Result<AgentRecord, RuntimeError> {
        let record = self.get_retained(id).await?;
        if self.archived(id).await?.is_some() || !record.status.is_terminal() {
            return Err(RuntimeError::Invalid("worktree mutation requires a retained terminal agent with no archive transaction pending".into()));
        }
        Ok(record)
    }
    pub(crate) async fn mutation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.inner.mutations.lock().await
    }
    pub async fn clear_worktree(&self, id: AgentId) -> Result<(), RuntimeError> {
        let _mutation = self.mutation_guard().await;
        self.clear_worktree_locked(id).await
    }
    pub(crate) async fn clear_worktree_locked(&self, id: AgentId) -> Result<(), RuntimeError> {
        let control = self.control(id).await?;
        if self.archived(id).await?.is_some() {
            return Err(RuntimeError::Invalid(
                "agent archive already exists; retained evidence cannot be changed".into(),
            ));
        }
        {
            let mut record = control.record.write().await;
            if !record.status.is_terminal() {
                return Err(RuntimeError::Invalid(
                    "cannot clean up a running subagent worktree".into(),
                ));
            }
            record.worktree = None;
            record.updated_at = Utc::now();
        }
        self.persist(&control).await;
        Ok(())
    }
    pub async fn send_message(
        &self,
        id: AgentId,
        message: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.send_message_as(None, id, message).await
    }
    pub async fn send_message_as(
        &self,
        caller: Option<AgentId>,
        id: AgentId,
        message: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        self.send_as(
            caller,
            id,
            InboxMessage::Message(message.into()),
            SubagentEventKind::MessageQueued,
        )
        .await
    }
    pub async fn follow_up(
        &self,
        id: AgentId,
        message: impl Into<String>,
    ) -> Result<AgentId, RuntimeError> {
        self.follow_up_as(None, id, message).await
    }
    pub async fn follow_up_as(
        &self,
        caller: Option<AgentId>,
        id: AgentId,
        message: impl Into<String>,
    ) -> Result<AgentId, RuntimeError> {
        self.follow_up_in_run(caller, id, message, None).await
    }
    pub async fn follow_up_in_run(
        &self,
        caller: Option<AgentId>,
        id: AgentId,
        message: impl Into<String>,
        completion: Option<crate::completion::runtime::RunHandle>,
    ) -> Result<AgentId, RuntimeError> {
        if let Some(run) = &completion
            && !run
                .owns(crate::completion::Obligation::Agent(id))
                .await
                .map_err(|e| RuntimeError::Persistence(e.to_string()))?
        {
            return Err(RuntimeError::Invalid(
                "explicitly adopt agent before requesting a followup in this run".into(),
            ));
        }
        let message = message.into();
        let record = self.get_retained(id).await.map_err(|error| match error {
            RuntimeError::Unknown(_) => RuntimeError::Invalid("agent is archived or unknown; use spawn for new work and reference its original ID".into()),
            other => other,
        })?;
        if !record.status.is_terminal() {
            self.send_as(
                caller,
                id,
                InboxMessage::FollowUp(message),
                SubagentEventKind::FollowUpQueued,
            )
            .await?;
            return Ok(id);
        }
        self.spawn_for_run(
            SpawnRequest {
                parent_id: Some(id),
                name: format!("{}-follow-up", record.name),
                task: message,
                policy: record.policy.clone(),
                budget: record.budget.clone(),
                worktree: record.worktree,
                branch: record.branch,
            },
            completion,
        )
        .await
    }

    pub async fn wait_many(
        &self,
        ids: &[AgentId],
    ) -> Result<Vec<Result<SubagentResult, String>>, RuntimeError> {
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            results.push(self.wait(*id).await?);
        }
        Ok(results)
    }
    async fn send_as(
        &self,
        caller: Option<AgentId>,
        id: AgentId,
        message: InboxMessage,
        event: SubagentEventKind,
    ) -> Result<(), RuntimeError> {
        let control = match self.control(id).await {
            Ok(control) => control,
            Err(RuntimeError::Unknown(_)) if self.archived(id).await?.is_some() => {
                return Err(RuntimeError::Terminal(id));
            }
            Err(error) => return Err(error),
        };
        if control.record.read().await.status.is_terminal() {
            return Err(RuntimeError::Terminal(id));
        }
        match control.inbox.try_send(message) {
            Ok(()) => {
                self.emit(id, event).await;
                Ok(())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => Err(RuntimeError::Terminal(id)),
            Err(mpsc::error::TrySendError::Full(message)) => {
                // Backpressure must not occupy the slot the recipient needs to drain.
                self.yield_slot(caller, &[id], self.send(id, message, event))
                    .await
            }
        }
    }

    async fn send(
        &self,
        id: AgentId,
        message: InboxMessage,
        event: SubagentEventKind,
    ) -> Result<(), RuntimeError> {
        let c = match self.control(id).await {
            Ok(c) => c,
            Err(RuntimeError::Unknown(_)) if self.archived(id).await?.is_some() => {
                return Err(RuntimeError::Terminal(id));
            }
            Err(error) => return Err(error),
        };
        if c.record.read().await.status.is_terminal() {
            return Err(RuntimeError::Terminal(id));
        }
        c.inbox
            .send(message)
            .await
            .map_err(|_| RuntimeError::Terminal(id))?;
        self.emit(id, event).await;
        Ok(())
    }
    async fn control(&self, id: AgentId) -> Result<Arc<Control>, RuntimeError> {
        self.inner
            .agents
            .read()
            .await
            .get(&id)
            .cloned()
            .ok_or(RuntimeError::Unknown(id))
    }
    async fn record_progress(&self, id: AgentId, text: String) {
        let text = preview(&text);
        let _mutation = self.inner.mutations.lock().await;
        if let Ok(c) = self.control(id).await {
            let mut r = c.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.recent_progress.push(text.clone());
            if r.recent_progress.len() > 20 {
                r.recent_progress.remove(0);
            }
            r.updated_at = Utc::now();
            drop(r);
            self.persist(&c).await;
            self.emit(id, SubagentEventKind::Progress { text }).await;
        }
    }
    async fn finish_completed(&self, id: AgentId, c: &Control, value: SubagentResult) {
        let _mutation = self.inner.mutations.lock().await;
        c.permit.lock().await.take();
        c.waiting_for.lock().await.clear();
        {
            let mut r = c.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.status = AgentStatus::Completed;
            r.result = Some(value.summary.clone());
            r.finished_at = Some(Utc::now());
            r.updated_at = Utc::now();
        }
        self.persist(c).await;
        self.emit(
            id,
            SubagentEventKind::Completed {
                result: value.clone(),
            },
        )
        .await;
        self.archive_finished().await;
        c.outcome.send_replace(Some(Ok(value)));
    }
    async fn finish_failed(&self, id: AgentId, c: &Control, error: String) {
        let _mutation = self.inner.mutations.lock().await;
        c.permit.lock().await.take();
        c.waiting_for.lock().await.clear();
        {
            let mut r = c.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.status = AgentStatus::Failed;
            r.error = Some(error.clone());
            r.finished_at = Some(Utc::now());
            r.updated_at = Utc::now();
        }
        self.persist(c).await;
        self.emit(
            id,
            SubagentEventKind::Failed {
                error: error.clone(),
            },
        )
        .await;
        self.archive_finished().await;
        c.outcome.send_replace(Some(Err(error)));
    }
    async fn finish_cancelled(&self, id: AgentId, c: &Control) {
        let _mutation = self.inner.mutations.lock().await;
        c.permit.lock().await.take();
        c.waiting_for.lock().await.clear();
        {
            let mut r = c.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.status = AgentStatus::Cancelled;
            r.error = Some("cancelled".into());
            r.finished_at = Some(Utc::now());
            r.updated_at = Utc::now();
        }
        self.persist(c).await;
        let error = "cancelled".to_string();
        self.emit(id, SubagentEventKind::Cancelled).await;
        self.archive_finished().await;
        c.outcome.send_replace(Some(Err(error)));
    }
    async fn archive_finished(&self) {
        if let Err(error) = self.prune_terminal_history(0, None).await {
            tracing::error!(%error, "could not archive finished subagents; records remain retained");
        }
    }
    async fn persist(&self, c: &Control) {
        if let Some(store) = &self.inner.store {
            let record = c.record.read().await.clone();
            if let Err(error) = store.update(record).await {
                tracing::error!(%error,"could not persist subagent state");
            }
        }
    }
    async fn emit(&self, id: AgentId, mut kind: SubagentEventKind) {
        match &mut kind {
            SubagentEventKind::Completed { result } => result.summary = preview(&result.summary),
            SubagentEventKind::Progress { text } => *text = preview(text),
            SubagentEventKind::Failed { error } => *error = preview(error),
            _ => {}
        }
        // The owned task retains the writer lease and finishes publication even if
        // a caller is cancelled while the filesystem write is in flight.
        let runtime = self.clone();
        let _ = tokio::spawn(async move {
            let mut history = runtime.inner.history.lock().await;
            let event = history.push(
                SubagentEvent {
                    sequence: 0,
                    timestamp: Utc::now(),
                    agent_id: id,
                    kind,
                },
                runtime.inner.limits.event_history,
            );
            if let Some(store) = &runtime.inner.store {
                let path = store.history_path();
                let mut snapshot = history.clone();

                match tokio::task::spawn_blocking(move || {
                    snapshot.persist(&path);
                    snapshot
                })
                .await
                {
                    Ok(saved) => *history = saved,
                    Err(_) => history.write_failed(),
                }
            }
            let _ = runtime.inner.events.send(event);
        })
        .await;
    }
}

// Events are notifications. Full completed results are available via get/wait.
fn preview(text: &str) -> String {
    const BYTES: usize = 4096;
    if text.len() <= BYTES {
        return text.to_owned();
    }
    let mut end = BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}… [preview]", &text[..end])
}
