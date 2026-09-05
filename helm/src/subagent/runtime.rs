use super::{AgentBudget, AgentId, AgentPolicy, AgentRecord, AgentStatus, AgentTreeStore};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
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
    limits: RuntimeLimits,
    permits: Arc<Semaphore>,
    agents: RwLock<BTreeMap<AgentId, Arc<Control>>>,
    history: Mutex<VecDeque<SubagentEvent>>,
    sequence: AtomicU64,
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
        store: Option<AgentTreeStore>,
    ) -> Result<Self, RuntimeError> {
        if limits.max_concurrency == 0
            || limits.max_concurrency > Semaphore::MAX_PERMITS
            || limits.event_history == 0
        {
            return Err(RuntimeError::Invalid(
                "runtime limits must be greater than zero".into(),
            ));
        }
        let (events, _) = broadcast::channel(limits.event_history.clamp(16, 4096));
        Ok(Self {
            inner: Arc::new(Inner {
                executor,
                permits: Arc::new(Semaphore::new(limits.max_concurrency)),
                limits,
                agents: RwLock::new(BTreeMap::new()),
                history: Mutex::new(VecDeque::new()),
                sequence: AtomicU64::new(0),
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
        let runtime = Self::new(executor, limits, Some(store))?;
        let mut agents = runtime.inner.agents.write().await;
        for record in records {
            let (inbox, _) = mpsc::channel(1);
            let initial = terminal_outcome(&record);
            let (outcome, _) = watch::channel(initial);
            agents.insert(
                record.id,
                Arc::new(Control {
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

    pub fn subscribe(&self) -> broadcast::Receiver<SubagentEvent> {
        self.inner.events.subscribe()
    }
    pub async fn events_after(&self, sequence: u64) -> Vec<SubagentEvent> {
        self.inner
            .history
            .lock()
            .await
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

    pub async fn spawn(&self, request: SpawnRequest) -> Result<AgentId, RuntimeError> {
        let _mutation = self.inner.mutations.lock().await;
        if request.task.trim().is_empty() || request.name.trim().is_empty() {
            return Err(RuntimeError::Invalid(
                "name and task cannot be empty".into(),
            ));
        }
        if let Some(parent) = request.parent_id {
            let parent_record = self.get_retained(parent).await?;
            parent_record
                .policy
                .validate_child(&request.policy)
                .map_err(|e| RuntimeError::Invalid(e.to_string()))?;
        }
        self.prune_terminal_history(0, request.parent_id).await?;
        let cancellation = if let Some(parent) = request.parent_id {
            let parent = self.control(parent).await?;
            if parent.cancel.is_cancelled() {
                return Err(RuntimeError::Invalid(
                    "cannot spawn from a cancelled parent".into(),
                ));
            }
            parent.cancel.child_token()
        } else {
            CancellationToken::new()
        };
        let mut agents = self.inner.agents.write().await;
        let id = AgentId::new();
        let now = Utc::now();
        let record = AgentRecord {
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
        tokio::spawn(async move {
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
        let permit = tokio::select! { _=control.cancel.cancelled()=>{self.finish_cancelled(id,&control).await;return}, p=self.inner.permits.clone().acquire_owned()=>match p {Ok(p)=>p,Err(_)=>{self.finish_failed(id,&control,"runtime shut down".into()).await;return}} };
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
            _=control.cancel.cancelled()=>End::Cancelled,
            r=self.inner.executor.execute(context)=>End::Result(r)
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
        self.spawn(SpawnRequest {
            parent_id: Some(id),
            name: format!("{}-follow-up", record.name),
            task: message,
            policy: record.policy.clone(),
            budget: record.budget.clone(),
            worktree: record.worktree,
            branch: record.branch,
        })
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
        let event = SubagentEvent {
            sequence: self.inner.sequence.fetch_add(1, Ordering::Relaxed) + 1,
            timestamp: Utc::now(),
            agent_id: id,
            kind,
        };
        let mut history = self.inner.history.lock().await;
        history.push_back(event.clone());
        while history.len() > self.inner.limits.event_history {
            history.pop_front();
        }
        drop(history);
        let _ = self.inner.events.send(event);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeSet,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    struct GateExecutor {
        entered: AtomicUsize,
        active: AtomicUsize,
        peak: AtomicUsize,
        gate: Notify,
    }
    impl GateExecutor {
        fn new() -> Self {
            Self {
                entered: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                peak: AtomicUsize::new(0),
                gate: Notify::new(),
            }
        }
    }
    #[async_trait]
    impl SubagentExecutor for GateExecutor {
        async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
            self.entered.fetch_add(1, Ordering::SeqCst);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            context.progress(format!("started {}", context.id)).await;
            let mut summary = "done".to_string();
            tokio::select! {_=self.gate.notified()=>{}, message=context.recv()=>if let Some(InboxMessage::Message(text))=message {context.progress(text.clone()).await; summary=text}}
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(SubagentResult { summary })
        }
    }
    fn policy() -> AgentPolicy {
        let budget = budget();
        AgentPolicy {
            readable_roots: vec![],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: super::super::ApprovalPolicy::Deny,
            budget,
        }
    }
    fn budget() -> AgentBudget {
        AgentBudget {
            max_tokens: 1000,

            max_terminals: 1,
        }
    }
    fn request(name: &str) -> SpawnRequest {
        SpawnRequest {
            parent_id: None,
            name: name.into(),
            task: "task".into(),
            policy: policy(),
            budget: budget(),
            worktree: None,
            branch: None,
        }
    }

    #[tokio::test]
    async fn enforces_concurrency_and_cancels_queued_work() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 1,

                event_history: 64,
            },
            None,
        )
        .unwrap();
        let first = runtime.spawn(request("first")).await.unwrap();
        let second = runtime.spawn(request("second")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while executor.entered.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        runtime.cancel(second).await.unwrap();
        assert_eq!(runtime.wait(second).await.unwrap(), Err("cancelled".into()));
        assert_eq!(executor.entered.load(Ordering::SeqCst), 1);
        executor.gate.notify_one();
        assert_eq!(runtime.wait(first).await.unwrap().unwrap().summary, "done");
        assert_eq!(executor.peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn messages_progress_events_and_terminal_rules_are_structured() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            None,
        )
        .unwrap();
        let id = runtime.spawn(request("worker")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while runtime.get(id).await.unwrap().status != AgentStatus::Running {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        runtime.send_message(id, "checkpoint").await.unwrap();
        assert_eq!(
            runtime.wait(id).await.unwrap().unwrap().summary,
            "checkpoint"
        );
        assert!(matches!(
            runtime.send_message(id, "late").await,
            Err(RuntimeError::Terminal(_))
        ));
        let events = runtime.events_after(0).await;
        assert!(events.windows(2).all(|w| w[0].sequence < w[1].sequence));
        assert!(
            events
                .iter()
                .any(|e| matches!(&e.kind,SubagentEventKind::Progress{text} if text=="checkpoint"))
        );
        assert_eq!(
            runtime.get(id).await.unwrap().status,
            AgentStatus::Completed
        );
    }

    #[tokio::test]
    async fn more_than_64_agents_queue_without_capacity_failure() {
        let runtime = SubagentRuntime::new(
            Arc::new(GateExecutor::new()),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 16,
            },
            None,
        )
        .unwrap();
        let parent = runtime.spawn(request("parent")).await.unwrap();
        for n in 0..80 {
            let mut child = request(&format!("child-{n}"));
            child.parent_id = Some(parent);
            runtime.spawn(child).await.unwrap();
        }
        assert_eq!(runtime.list().await.len(), 81);
        let mut denied = request("denied");
        denied.parent_id = Some(parent);
        denied.policy.allowed_tools.insert("new-authority".into());
        assert!(matches!(
            runtime.spawn(denied).await,
            Err(RuntimeError::Invalid(_))
        ));
        runtime.cancel(parent).await.unwrap();
        for record in runtime.list().await {
            assert_eq!(
                runtime.wait(record.id).await.unwrap(),
                Err("cancelled".into())
            );
        }
    }

    #[tokio::test]
    async fn spawning_archives_old_terminal_records_without_a_total_limit() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 1,

                event_history: 16,
            },
            None,
        )
        .unwrap();

        let first = runtime.spawn(request("first")).await.unwrap();
        executor.gate.notify_one();
        runtime.wait(first).await.unwrap().unwrap();
        let second = runtime.spawn(request("second")).await.unwrap();
        executor.gate.notify_one();
        runtime.wait(second).await.unwrap().unwrap();

        let third = runtime.spawn(request("third")).await.unwrap();
        let retained = runtime.list().await;
        assert_eq!(retained.len(), 1);
        assert!(!retained.iter().any(|record| record.id == first));
        assert!(!retained.iter().any(|record| record.id == second));
        assert_eq!(runtime.wait(first).await.unwrap().unwrap().summary, "done");
        assert_eq!(
            runtime.list_archived(None, 20).await.unwrap().agents.len(),
            2
        );
        assert!(retained.iter().any(|record| record.id == third));
        runtime.cancel(third).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn agents_continue_beyond_legacy_runtime_budget() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            None,
        )
        .unwrap();
        let mut item = request("long-lived");
        let mut legacy = serde_json::to_value(&item.budget).unwrap();
        legacy["max_runtime_secs"] = serde_json::json!(1);
        legacy["max_children"] = serde_json::json!(0);
        item.budget = serde_json::from_value(legacy).unwrap();
        let id = runtime.spawn(item).await.unwrap();
        while executor.entered.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        tokio::time::advance(std::time::Duration::from_secs(3600)).await;
        assert_eq!(runtime.get(id).await.unwrap().status, AgentStatus::Running);
        executor.gate.notify_one();
        assert_eq!(runtime.wait(id).await.unwrap().unwrap().summary, "done");
    }

    struct MailboxExecutor {
        runtime: std::sync::OnceLock<std::sync::Weak<Inner>>,
    }
    #[async_trait]
    impl SubagentExecutor for MailboxExecutor {
        async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
            let runtime = SubagentRuntime {
                inner: self.runtime.get().unwrap().upgrade().unwrap(),
            };
            if context.task.starts_with("sender") {
                let mut receiver = request("receiver");
                receiver.parent_id = Some(context.id);
                let receiver = runtime.spawn(receiver).await.unwrap();
                for i in 0..65 {
                    if (i + usize::from(context.task == "sender-follow-up")) % 2 == 0 {
                        runtime
                            .send_message_as(Some(context.id), receiver, i.to_string())
                            .await
                            .unwrap();
                    } else {
                        runtime
                            .follow_up_as(Some(context.id), receiver, i.to_string())
                            .await
                            .unwrap();
                    }
                }
                runtime
                    .wait_many_as(Some(context.id), &[receiver])
                    .await
                    .unwrap()[0]
                    .as_ref()
                    .unwrap();
            } else {
                for i in 0..65 {
                    let message = context.recv().await.unwrap();
                    let text = match message {
                        InboxMessage::Message(text) | InboxMessage::FollowUp(text) => text,
                    };
                    assert_eq!(text, i.to_string());
                }
            }
            Ok(SubagentResult {
                summary: "delivered".into(),
            })
        }
    }

    #[tokio::test]
    async fn full_mailboxes_yield_slots_for_message_and_follow_up_delivery() {
        let executor = Arc::new(MailboxExecutor {
            runtime: std::sync::OnceLock::new(),
        });
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            None,
        )
        .unwrap();
        executor
            .runtime
            .set(Arc::downgrade(&runtime.inner))
            .unwrap();
        for task in ["sender-message", "sender-follow-up"] {
            let mut sender = request(task);
            sender.task = task.into();
            let id = runtime.spawn(sender).await.unwrap();
            assert_eq!(
                tokio::time::timeout(std::time::Duration::from_secs(3), runtime.wait(id))
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .summary,
                "delivered"
            );
            assert_eq!(runtime.inner.permits.available_permits(), 1);
        }
    }

    struct NestedExecutor {
        runtime: std::sync::OnceLock<std::sync::Weak<Inner>>,
    }
    #[async_trait]
    impl SubagentExecutor for NestedExecutor {
        async fn execute(&self, context: ExecutionContext) -> Result<SubagentResult, String> {
            let runtime = SubagentRuntime {
                inner: self.runtime.get().unwrap().upgrade().unwrap(),
            };
            let depth: usize = context.task.parse().unwrap();
            if depth > 0 {
                let mut item = request("nested");
                item.parent_id = Some(context.id);
                item.task = (depth - 1).to_string();
                let child = runtime.spawn(item).await.map_err(|e| e.to_string())?;
                let results = runtime
                    .wait_many_as(Some(context.id), &[child])
                    .await
                    .map_err(|e| e.to_string())?;
                assert_eq!(results[0].as_ref().unwrap().summary, "nested result");
                assert_eq!(
                    runtime.get(context.id).await.unwrap().status,
                    AgentStatus::Running
                );
                assert!(
                    runtime
                        .control(context.id)
                        .await
                        .unwrap()
                        .permit
                        .lock()
                        .await
                        .is_some()
                );
            }
            Ok(SubagentResult {
                summary: "nested result".into(),
            })
        }
    }

    #[tokio::test]
    async fn nesting_beyond_eight_with_one_slot_finishes_and_archives() {
        let executor = Arc::new(NestedExecutor {
            runtime: std::sync::OnceLock::new(),
        });
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            None,
        )
        .unwrap();
        executor
            .runtime
            .set(Arc::downgrade(&runtime.inner))
            .unwrap();
        let mut item = request("root");
        item.task = "12".into();
        let root = runtime.spawn(item).await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(5), runtime.wait(root))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(result.summary, "nested result");
        assert!(runtime.list().await.is_empty());
        assert_eq!(
            runtime.list_archived(None, 100).await.unwrap().agents.len(),
            13
        );
        assert_eq!(runtime.inner.permits.available_permits(), 1);
    }

    async fn await_status(runtime: &SubagentRuntime, id: AgentId, status: AgentStatus) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while runtime.get(id).await.unwrap().status != status {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn wait_reacquires_slot_and_cancellation_does_not_leak_it() {
        let runtime = SubagentRuntime::new(
            Arc::new(GateExecutor::new()),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            None,
        )
        .unwrap();
        let parent = runtime.spawn(request("parent")).await.unwrap();
        await_status(&runtime, parent, AgentStatus::Running).await;
        let mut item = request("child");
        item.parent_id = Some(parent);
        let child = runtime.spawn(item).await.unwrap();
        let worker_runtime = runtime.clone();
        let waiter =
            tokio::spawn(async move { worker_runtime.wait_many_as(Some(parent), &[child]).await });
        await_status(&runtime, child, AgentStatus::Running).await;
        let blocker = runtime.spawn(request("blocker")).await.unwrap();
        runtime.send_message(child, "finished child").await.unwrap();
        await_status(&runtime, blocker, AgentStatus::Running).await;
        assert!(
            !waiter.is_finished(),
            "parent must reacquire before returning"
        );
        runtime.cancel(parent).await.unwrap();
        assert!(waiter.await.unwrap().is_err());
        runtime.wait(parent).await.unwrap().unwrap_err();
        runtime.cancel(blocker).await.unwrap();
        runtime.wait(blocker).await.unwrap().unwrap_err();
        assert_eq!(runtime.inner.permits.available_permits(), 1);
        let next = runtime.spawn(request("next")).await.unwrap();
        await_status(&runtime, next, AgentStatus::Running).await;
        runtime.send_message(next, "done").await.unwrap();
        runtime.wait(next).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn invalid_and_cyclic_waits_do_not_lose_slots() {
        let runtime = SubagentRuntime::new(
            Arc::new(GateExecutor::new()),
            RuntimeLimits {
                max_concurrency: 2,
                event_history: 32,
            },
            None,
        )
        .unwrap();
        let a = runtime.spawn(request("a")).await.unwrap();
        let b = runtime.spawn(request("b")).await.unwrap();
        await_status(&runtime, a, AgentStatus::Running).await;
        await_status(&runtime, b, AgentStatus::Running).await;
        assert!(runtime.wait_many_as(Some(a), &[a]).await.is_err());
        assert!(
            runtime
                .wait_many_as(Some(a), &[AgentId::new()])
                .await
                .is_err()
        );
        assert!(
            runtime
                .control(a)
                .await
                .unwrap()
                .permit
                .lock()
                .await
                .is_some()
        );
        let r = runtime.clone();
        let waiting = tokio::spawn(async move { r.wait_many_as(Some(a), &[b]).await });
        await_status(&runtime, a, AgentStatus::Waiting).await;
        assert!(runtime.wait_many_as(Some(b), &[a]).await.is_err());
        assert!(
            runtime
                .control(b)
                .await
                .unwrap()
                .permit
                .lock()
                .await
                .is_some()
        );
        runtime.cancel(b).await.unwrap();
        let result = waiting.await.unwrap().unwrap();
        assert_eq!(result, vec![Err("cancelled".into())]);
        assert!(
            runtime
                .control(a)
                .await
                .unwrap()
                .permit
                .lock()
                .await
                .is_some()
        );
        runtime.cancel(a).await.unwrap();
        runtime.wait(a).await.unwrap().unwrap_err();
    }

    #[tokio::test]
    async fn dropping_wait_cancels_worker_and_releases_descendants() {
        let runtime = SubagentRuntime::new(
            Arc::new(GateExecutor::new()),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            None,
        )
        .unwrap();
        let parent = runtime.spawn(request("parent")).await.unwrap();
        await_status(&runtime, parent, AgentStatus::Running).await;
        let mut child = request("child");
        child.parent_id = Some(parent);
        let child = runtime.spawn(child).await.unwrap();
        assert!(runtime.wait_many_as(Some(child), &[parent]).await.is_err());
        assert!(
            runtime
                .wait_many_as(Some(child), &[AgentId::new()])
                .await
                .is_err()
        );
        assert!(
            runtime
                .wait_many_as(Some(parent), &[])
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            runtime
                .control(parent)
                .await
                .unwrap()
                .permit
                .lock()
                .await
                .is_some()
        );
        let r = runtime.clone();
        let waiting = tokio::spawn(async move { r.wait_many_as(Some(parent), &[child]).await });
        await_status(&runtime, child, AgentStatus::Running).await;
        waiting.abort();
        assert!(waiting.await.unwrap_err().is_cancelled());
        assert_eq!(runtime.wait(parent).await.unwrap(), Err("cancelled".into()));
        assert_eq!(runtime.wait(child).await.unwrap(), Err("cancelled".into()));
        assert_eq!(runtime.inner.permits.available_permits(), 1);
    }

    #[tokio::test]
    async fn cancelled_parent_cannot_spawn_and_existing_children_inherit_cancellation() {
        let runtime = SubagentRuntime::new(
            Arc::new(GateExecutor::new()),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            None,
        )
        .unwrap();
        let parent = runtime.spawn(request("parent")).await.unwrap();
        let mut child = request("child");
        child.parent_id = Some(parent);
        let id = runtime.spawn(child.clone()).await.unwrap();
        runtime.control(parent).await.unwrap().cancel.cancel();
        assert!(runtime.spawn(child).await.is_err());
        assert_eq!(runtime.wait(id).await.unwrap(), Err("cancelled".into()));
        assert_eq!(runtime.wait(parent).await.unwrap(), Err("cancelled".into()));
    }

    #[tokio::test]
    async fn large_unicode_event_previews_preserve_full_durable_results() {
        let directory = tempfile::tempdir().unwrap();
        let store = AgentTreeStore::new(directory.path().join("agents.json"));
        let runtime = SubagentRuntime::new_persistent(
            Arc::new(ImmediateExecutor),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            store.clone(),
        )
        .await
        .unwrap();
        let text = "🦀".repeat(32_000);
        let mut item = request("large");
        item.task = text.clone();
        let id = runtime.spawn(item).await.unwrap();
        assert_eq!(runtime.wait(id).await.unwrap().unwrap().summary, text);
        let events = runtime.events_after(0).await;
        let event = events
            .iter()
            .find_map(|event| match &event.kind {
                SubagentEventKind::Completed { result } => Some(result),
                _ => None,
            })
            .unwrap();
        assert!(event.summary.len() < 4200);
        assert!(event.summary.ends_with("[preview]"));
        drop(runtime);
        let reopened = SubagentRuntime::new_persistent(
            Arc::new(ImmediateExecutor),
            RuntimeLimits {
                max_concurrency: 1,
                event_history: 32,
            },
            store,
        )
        .await
        .unwrap();
        assert_eq!(reopened.wait(id).await.unwrap().unwrap().summary, text);
    }

    #[tokio::test]
    async fn archived_agents_require_fresh_spawn_for_new_work() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            None,
        )
        .unwrap();
        let parent = runtime.spawn(request("original")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while executor.entered.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await
            }
        })
        .await
        .unwrap();
        executor.gate.notify_one();
        runtime.wait(parent).await.unwrap().unwrap();
        assert!(matches!(
            runtime.follow_up(parent, "continue the work").await,
            Err(RuntimeError::Invalid(_))
        ));
        let mut child = request("invalid-child");
        child.parent_id = Some(parent);
        assert_eq!(
            runtime.spawn(child).await.unwrap_err(),
            RuntimeError::Unknown(parent)
        );
        assert!(runtime.list().await.is_empty());
        assert_eq!(
            runtime.get(parent).await.unwrap().result.as_deref(),
            Some("done")
        );
    }

    #[tokio::test]
    async fn completion_is_retained_before_any_waiter_subscribes() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            None,
        )
        .unwrap();
        let id = runtime.spawn(request("early-finish")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while executor.entered.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        executor.gate.notify_one();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while runtime.get(id).await.unwrap().status != AgentStatus::Completed {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let result = tokio::time::timeout(std::time::Duration::from_millis(100), runtime.wait(id))
            .await
            .expect("late wait must not hang")
            .unwrap()
            .unwrap();
        assert_eq!(result.summary, "done");
    }

    #[tokio::test]
    async fn persistent_runtime_hydrates_completed_records_and_results() {
        let directory = tempfile::tempdir().unwrap();
        let store = AgentTreeStore::new(directory.path().join("agents.json"));
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new_persistent(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            store.clone(),
        )
        .await
        .unwrap();
        let id = runtime.spawn(request("persisted")).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while executor.entered.load(Ordering::SeqCst) < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        executor.gate.notify_one();
        runtime.wait(id).await.unwrap().unwrap();
        drop(runtime);

        let reopened = SubagentRuntime::new_persistent(
            Arc::new(GateExecutor::new()),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            store,
        )
        .await
        .unwrap();
        assert!(reopened.list().await.is_empty());
        assert!(reopened.is_archived(id).await.unwrap());
        assert_eq!(reopened.wait(id).await.unwrap().unwrap().summary, "done");
    }
    struct ImmediateExecutor;
    #[async_trait]
    impl SubagentExecutor for ImmediateExecutor {
        async fn execute(&self, context: ExecutionContext) -> Result<SubagentResult, String> {
            if context.task == "fail" {
                Err("expected failure".into())
            } else {
                Ok(SubagentResult {
                    summary: context.task,
                })
            }
        }
    }

    #[tokio::test]
    async fn concurrent_completions_archive_without_losing_waiters_or_records() {
        let directory = tempfile::tempdir().unwrap();
        let store = AgentTreeStore::new(directory.path().join("agents.json"));
        let runtime = SubagentRuntime::new_persistent(
            Arc::new(ImmediateExecutor),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            store.clone(),
        )
        .await
        .unwrap();
        let mut handles = Vec::new();
        for i in 0..24 {
            let runtime = runtime.clone();
            handles.push(tokio::spawn(async move {
                let mut item = request(&format!("child-{i}"));
                item.task = if i % 2 == 0 {
                    "fail".into()
                } else {
                    format!("result-{i}")
                };
                let id = runtime.spawn(item).await.unwrap();
                let outcome = runtime.wait(id).await.unwrap();
                (id, outcome)
            }));
        }
        let mut expected = Vec::new();
        for handle in handles {
            expected.push(handle.await.unwrap());
        }
        // Synchronize with archival when a late waiter read the finished record directly.
        {
            let _guard = runtime.mutation_guard().await;
        }
        assert!(runtime.list().await.is_empty());
        assert_eq!(
            runtime.list_archived(None, 100).await.unwrap().agents.len(),
            24
        );
        let reopened = SubagentRuntime::new_persistent(
            Arc::new(ImmediateExecutor),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            store,
        )
        .await
        .unwrap();
        for (id, outcome) in expected {
            assert_eq!(reopened.wait(id).await.unwrap(), outcome);
            assert!(reopened.is_archived(id).await.unwrap());
        }
        assert_eq!(
            reopened
                .wait(AgentId::new())
                .await
                .unwrap_err()
                .to_string()
                .split_whitespace()
                .next(),
            Some("unknown")
        );
    }

    #[tokio::test]
    async fn auto_archive_releases_completed_children_under_live_parents() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor,
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 2048,
            },
            None,
        )
        .unwrap();
        let parent = runtime.spawn(request("parent")).await.unwrap();
        let mut child_request = request("child");
        child_request.parent_id = Some(parent);
        let child = runtime.spawn(child_request).await.unwrap();
        runtime.send_message(child, "child evidence").await.unwrap();
        assert_eq!(
            runtime.wait(child).await.unwrap().unwrap().summary,
            "child evidence"
        );
        assert!(runtime.is_archived(child).await.unwrap());
        assert_eq!(runtime.list().await.len(), 1);
        runtime.cancel(parent).await.unwrap();
        assert!(runtime.wait(parent).await.unwrap().is_err());
        assert!(runtime.list().await.is_empty());
        assert_eq!(runtime.get(child).await.unwrap().parent_id, Some(parent));
        assert_eq!(
            runtime.wait(child).await.unwrap().unwrap().summary,
            "child evidence"
        );
    }

    #[tokio::test]
    async fn archive_failure_keeps_finished_evidence_and_capacity_retry_is_safe() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agents.json");
        let runtime = SubagentRuntime::new_persistent(
            Arc::new(ImmediateExecutor),
            RuntimeLimits {
                ..RuntimeLimits {
                    max_concurrency: 4,
                    event_history: 2048,
                }
            },
            AgentTreeStore::new(path.clone()),
        )
        .await
        .unwrap();
        std::fs::write(path.with_extension("archive"), "blocked").unwrap();
        let first = runtime.spawn(request("first")).await.unwrap();
        assert!(runtime.wait(first).await.unwrap().is_ok());
        assert_eq!(runtime.list().await.len(), 1);
        assert!(matches!(
            runtime.spawn(request("second")).await,
            Err(RuntimeError::Persistence(_))
        ));
        assert_eq!(
            runtime.get(first).await.unwrap().result.as_deref(),
            Some("task")
        );
        std::fs::remove_file(path.with_extension("archive")).unwrap();
        let second = runtime.spawn(request("second")).await.unwrap();
        runtime.wait(second).await.unwrap().unwrap();
        assert!(runtime.is_archived(first).await.unwrap());
        assert_eq!(runtime.wait(first).await.unwrap().unwrap().summary, "task");
    }
}
