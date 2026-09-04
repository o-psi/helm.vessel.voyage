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
use tokio::sync::{Mutex, RwLock, Semaphore, broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct RuntimeLimits {
    pub max_concurrency: usize,
    pub max_agents: usize,
    pub event_history: usize,
}
impl Default for RuntimeLimits {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            max_agents: 64,
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
    #[error("subagent capacity of {0} reached")]
    Capacity(usize),
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
    permits: Semaphore,
    agents: RwLock<BTreeMap<AgentId, Arc<Control>>>,
    history: Mutex<VecDeque<SubagentEvent>>,
    sequence: AtomicU64,
    events: broadcast::Sender<SubagentEvent>,
    store: Option<AgentTreeStore>,
}
struct Control {
    record: RwLock<AgentRecord>,
    cancel: CancellationToken,
    inbox: mpsc::Sender<InboxMessage>,
    outcome: watch::Sender<Option<Result<SubagentResult, String>>>,
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
        if limits.max_concurrency == 0 || limits.max_agents == 0 || limits.event_history == 0 {
            return Err(RuntimeError::Invalid(
                "runtime limits must be greater than zero".into(),
            ));
        }
        let (events, _) = broadcast::channel(limits.event_history.clamp(16, 4096));
        Ok(Self {
            inner: Arc::new(Inner {
                executor,
                permits: Semaphore::new(limits.max_concurrency),
                limits,
                agents: RwLock::new(BTreeMap::new()),
                history: Mutex::new(VecDeque::new()),
                sequence: AtomicU64::new(0),
                events,
                store,
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
        let c = self.control(id).await?;
        let record = c.record.read().await.clone();
        Ok(record)
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
        if request.task.trim().is_empty() || request.name.trim().is_empty() {
            return Err(RuntimeError::Invalid(
                "name and task cannot be empty".into(),
            ));
        }
        if let Some(parent) = request.parent_id {
            let parent_record = self.get(parent).await?;
            parent_record
                .policy
                .validate_child(&request.policy)
                .map_err(|e| RuntimeError::Invalid(e.to_string()))?;
        }
        let mut agents = self.inner.agents.write().await;
        let controls: Vec<_> = agents.values().cloned().collect();
        let mut active = 0;
        for control in controls {
            if !control.record.read().await.status.is_terminal() {
                active += 1;
            }
        }
        if active >= self.inner.limits.max_agents {
            return Err(RuntimeError::Capacity(self.inner.limits.max_agents));
        }
        if let Some(parent) = request.parent_id {
            let max_children = agents
                .get(&parent)
                .expect("parent existence checked while retained agents are never removed")
                .record
                .read()
                .await
                .budget
                .max_children as usize;
            let mut child_count = 0;
            for control in agents.values() {
                if control.record.read().await.parent_id == Some(parent) {
                    child_count += 1;
                }
            }
            if child_count >= max_children {
                return Err(RuntimeError::Capacity(max_children));
            }
        }
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
            cancel: CancellationToken::new(),
            inbox: inbox_tx,
            outcome,
        });
        agents.insert(id, control.clone());
        drop(agents);
        self.emit(id, SubagentEventKind::Queued).await;
        let runtime = self.clone();
        tokio::spawn(async move {
            runtime.run(id, request, inbox_rx, control).await;
        });
        Ok(id)
    }

    async fn run(
        &self,
        id: AgentId,
        request: SpawnRequest,
        inbox: mpsc::Receiver<InboxMessage>,
        control: Arc<Control>,
    ) {
        let permit = tokio::select! { _=control.cancel.cancelled()=>{self.finish_cancelled(id,&control).await;return}, p=self.inner.permits.acquire()=>match p {Ok(p)=>p,Err(_)=>{self.finish_failed(id,&control,"runtime shut down".into()).await;return}} };
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
            TimedOut,
            Result(Result<SubagentResult, String>),
        }
        let result = tokio::select! {
            _=control.cancel.cancelled()=>End::Cancelled,
            _=tokio::time::sleep(request.budget.runtime())=>End::TimedOut,
            r=self.inner.executor.execute(context)=>End::Result(r)
        };
        drop(permit);
        match result {
            End::Cancelled => self.finish_cancelled(id, &control).await,
            End::TimedOut => self.finish_timed_out(id, &control).await,
            End::Result(Ok(value)) => self.finish_completed(id, &control, value).await,
            End::Result(Err(error)) => self.finish_failed(id, &control, error).await,
        }
    }

    pub async fn wait(&self, id: AgentId) -> Result<Result<SubagentResult, String>, RuntimeError> {
        let c = self.control(id).await?;
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
        let c = self.control(id).await?;
        if !c.record.read().await.status.is_terminal() {
            c.cancel.cancel();
        }
        let descendants: Vec<_> = self
            .list()
            .await
            .into_iter()
            .filter(|record| record.parent_id == Some(id))
            .map(|record| record.id)
            .collect();
        for child in descendants {
            Box::pin(self.cancel(child)).await?;
        }
        Ok(())
    }
    pub async fn clear_worktree(&self, id: AgentId) -> Result<(), RuntimeError> {
        let control = self.control(id).await?;
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
        self.send(
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
        let message = message.into();
        let record = self.get(id).await?;
        if !record.status.is_terminal() {
            self.send(
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
    async fn send(
        &self,
        id: AgentId,
        message: InboxMessage,
        event: SubagentEventKind,
    ) -> Result<(), RuntimeError> {
        let c = self.control(id).await?;
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
        if let Ok(c) = self.control(id).await {
            let mut r = c.record.write().await;
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
        c.outcome.send_replace(Some(Ok(value.clone())));
        self.emit(id, SubagentEventKind::Completed { result: value })
            .await;
    }
    async fn finish_failed(&self, id: AgentId, c: &Control, error: String) {
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
        c.outcome.send_replace(Some(Err(error.clone())));
        self.emit(id, SubagentEventKind::Failed { error }).await;
    }
    async fn finish_cancelled(&self, id: AgentId, c: &Control) {
        {
            let mut r = c.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.status = AgentStatus::Cancelled;
            r.finished_at = Some(Utc::now());
            r.updated_at = Utc::now();
        }
        self.persist(c).await;
        let error = "cancelled".to_string();
        c.outcome.send_replace(Some(Err(error)));
        self.emit(id, SubagentEventKind::Cancelled).await;
    }
    async fn finish_timed_out(&self, id: AgentId, c: &Control) {
        {
            let mut r = c.record.write().await;
            if r.status.is_terminal() {
                return;
            }
            r.status = AgentStatus::TimedOut;
            r.error = Some("runtime budget exhausted".into());
            r.finished_at = Some(Utc::now());
            r.updated_at = Utc::now();
        }
        self.persist(c).await;
        c.outcome
            .send_replace(Some(Err("runtime budget exhausted".into())));
        self.emit(id, SubagentEventKind::TimedOut).await;
    }
    async fn persist(&self, c: &Control) {
        if let Some(store) = &self.inner.store {
            let record = c.record.read().await.clone();
            if let Err(error) = store.update(record).await {
                tracing::error!(%error,"could not persist subagent state");
            }
        }
    }
    async fn emit(&self, id: AgentId, kind: SubagentEventKind) {
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
            max_turns: 10,
            max_tokens: 1000,
            max_runtime_secs: 60,
            max_children: 8,
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
                max_agents: 3,
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
        let runtime =
            SubagentRuntime::new(executor.clone(), RuntimeLimits::default(), None).unwrap();
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
    async fn child_policy_and_capacity_are_enforced() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(
            executor,
            RuntimeLimits {
                max_concurrency: 1,
                max_agents: 1,
                event_history: 16,
            },
            None,
        )
        .unwrap();
        runtime.spawn(request("one")).await.unwrap();
        assert_eq!(
            runtime.spawn(request("two")).await.unwrap_err(),
            RuntimeError::Capacity(1)
        );
    }

    #[tokio::test]
    async fn runtime_budget_produces_a_typed_timeout() {
        let executor = Arc::new(GateExecutor::new());
        let runtime = SubagentRuntime::new(executor, RuntimeLimits::default(), None).unwrap();
        let mut request = request("short");
        request.budget.max_runtime_secs = 0;
        let id = runtime.spawn(request).await.unwrap();
        assert_eq!(
            runtime.wait(id).await.unwrap(),
            Err("runtime budget exhausted".into())
        );
        assert_eq!(runtime.get(id).await.unwrap().status, AgentStatus::TimedOut);
        assert!(
            runtime
                .events_after(0)
                .await
                .iter()
                .any(|event| matches!(event.kind, SubagentEventKind::TimedOut))
        );
    }

    #[tokio::test]
    async fn completed_agents_accept_follow_up_as_linked_execution() {
        let executor = Arc::new(GateExecutor::new());
        let runtime =
            SubagentRuntime::new(executor.clone(), RuntimeLimits::default(), None).unwrap();
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
        let child = runtime
            .follow_up(parent, "continue the work")
            .await
            .unwrap();
        assert_ne!(child, parent);
        assert_eq!(runtime.get(child).await.unwrap().parent_id, Some(parent));
        executor.gate.notify_one();
        runtime.wait(child).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn completion_is_retained_before_any_waiter_subscribes() {
        let executor = Arc::new(GateExecutor::new());
        let runtime =
            SubagentRuntime::new(executor.clone(), RuntimeLimits::default(), None).unwrap();
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
            RuntimeLimits::default(),
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
            RuntimeLimits::default(),
            store,
        )
        .await
        .unwrap();
        assert_eq!(reopened.list().await.len(), 1);
        assert_eq!(reopened.wait(id).await.unwrap().unwrap().summary, "done");
    }
}
