//! UI-facing adapter for supervising concurrent agent runs.
//!
//! Runtime implementations may be local or remote. TUI code depends only on this
//! bounded snapshot/event/action interface and never holds runtime task handles.

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tokio::sync::broadcast;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AgentId(pub Uuid);

impl std::fmt::Display for AgentId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentStatus {
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    TimedOut,
    Interrupted,
    Cancelled,
}

impl AgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::TimedOut | Self::Interrupted | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug)]
pub struct AgentView {
    pub id: AgentId,
    pub parent: Option<AgentId>,
    pub task: String,
    pub status: AgentStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub elapsed: Duration,
    pub worktree: Option<PathBuf>,
    pub recent_progress: Vec<String>,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AgentEvent {
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub agent_id: AgentId,
    pub kind: AgentEventKind,
}

#[derive(Clone, Debug)]
pub enum AgentEventKind {
    Queued,
    Started,
    Progress { text: String },
    MessageQueued,
    FollowUpQueued { child: Option<AgentId> },
    Completed { result: String },
    Failed { error: String },
    TimedOut { error: String },
    Interrupted { reason: String },
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum SupervisionError {
    #[error("agent {0} does not exist")]
    NotFound(AgentId),
    #[error("supervisor is unavailable")]
    Unavailable,
    #[error("supervisor operation failed: {0}")]
    Failed(String),
}

#[async_trait]
pub trait AgentSupervisor: Send + Sync {
    async fn tree(&self) -> Result<Vec<AgentView>, SupervisionError>;
    async fn inspect(
        &self,
        id: AgentId,
        after_sequence: Option<u64>,
    ) -> Result<Vec<AgentEvent>, SupervisionError>;
    async fn send_message(&self, id: AgentId, text: String) -> Result<(), SupervisionError>;
    async fn follow_up(&self, id: AgentId, text: String) -> Result<AgentId, SupervisionError>;
    async fn cancel(&self, id: AgentId) -> Result<(), SupervisionError>;
    fn subscribe(&self) -> broadcast::Receiver<AgentEvent>;
}

/// Converts the durable local runtime into the bounded, UI-facing contract.
pub struct RuntimeAgentSupervisor {
    runtime: Arc<crate::subagent::SubagentRuntime>,
    events: broadcast::Sender<AgentEvent>,
}

impl RuntimeAgentSupervisor {
    pub fn new(runtime: Arc<crate::subagent::SubagentRuntime>) -> Self {
        let (events, _) = broadcast::channel(256);
        let mut source = runtime.subscribe();
        let sink = events.clone();
        tokio::spawn(async move {
            loop {
                match source.recv().await {
                    Ok(event) => {
                        let _ = sink.send(convert_event(event));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        Self { runtime, events }
    }
}

#[async_trait]
impl AgentSupervisor for RuntimeAgentSupervisor {
    async fn tree(&self) -> Result<Vec<AgentView>, SupervisionError> {
        Ok(self
            .runtime
            .list()
            .await
            .into_iter()
            .map(convert_record)
            .collect())
    }

    async fn inspect(
        &self,
        id: AgentId,
        after_sequence: Option<u64>,
    ) -> Result<Vec<AgentEvent>, SupervisionError> {
        self.runtime
            .get(crate::subagent::AgentId(id.0))
            .await
            .map_err(runtime_error)?;
        Ok(self
            .runtime
            .events_after(after_sequence.unwrap_or(0))
            .await
            .into_iter()
            .filter(|event| event.agent_id.0 == id.0)
            .map(convert_event)
            .collect())
    }

    async fn send_message(&self, id: AgentId, text: String) -> Result<(), SupervisionError> {
        self.runtime
            .send_message(crate::subagent::AgentId(id.0), text)
            .await
            .map_err(runtime_error)
    }

    async fn follow_up(&self, id: AgentId, text: String) -> Result<AgentId, SupervisionError> {
        self.runtime
            .follow_up(crate::subagent::AgentId(id.0), text)
            .await
            .map(|id| AgentId(id.0))
            .map_err(runtime_error)
    }

    async fn cancel(&self, id: AgentId) -> Result<(), SupervisionError> {
        self.runtime
            .cancel(crate::subagent::AgentId(id.0))
            .await
            .map_err(runtime_error)
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }
}

fn convert_record(record: crate::subagent::AgentRecord) -> AgentView {
    let now = Utc::now();
    let end = record.finished_at.unwrap_or(now);
    let elapsed = record
        .started_at
        .and_then(|start| (end - start).to_std().ok())
        .unwrap_or_default();
    AgentView {
        id: AgentId(record.id.0),
        parent: record.parent_id.map(|id| AgentId(id.0)),
        task: record.task,
        status: convert_status(record.status),
        started_at: record.started_at,
        elapsed,
        worktree: record.worktree,
        recent_progress: record.recent_progress,
        result: record.result,
        error: record.error,
    }
}

fn convert_status(status: crate::subagent::AgentStatus) -> AgentStatus {
    match status {
        crate::subagent::AgentStatus::Queued => AgentStatus::Queued,
        crate::subagent::AgentStatus::Running => AgentStatus::Running,
        crate::subagent::AgentStatus::Waiting => AgentStatus::Waiting,
        crate::subagent::AgentStatus::Completed => AgentStatus::Completed,
        crate::subagent::AgentStatus::Failed => AgentStatus::Failed,
        crate::subagent::AgentStatus::TimedOut => AgentStatus::TimedOut,
        crate::subagent::AgentStatus::Interrupted => AgentStatus::Interrupted,
        crate::subagent::AgentStatus::Cancelled => AgentStatus::Cancelled,
    }
}

fn convert_event(event: crate::subagent::SubagentEvent) -> AgentEvent {
    use crate::subagent::SubagentEventKind as Source;
    let kind = match event.kind {
        Source::Queued => AgentEventKind::Queued,
        Source::Started => AgentEventKind::Started,
        Source::Progress { text } => AgentEventKind::Progress { text },
        Source::MessageQueued => AgentEventKind::MessageQueued,
        Source::FollowUpQueued => AgentEventKind::FollowUpQueued { child: None },
        Source::Completed { result } => AgentEventKind::Completed {
            result: result.summary,
        },
        Source::Failed { error } => AgentEventKind::Failed { error },
        Source::TimedOut => AgentEventKind::TimedOut {
            error: "runtime budget exhausted".into(),
        },
        Source::Cancelled => AgentEventKind::Cancelled,
    };
    AgentEvent {
        sequence: event.sequence,
        timestamp: event.timestamp,
        agent_id: AgentId(event.agent_id.0),
        kind,
    }
}

fn runtime_error(error: crate::subagent::RuntimeError) -> SupervisionError {
    match error {
        crate::subagent::RuntimeError::Unknown(id) => SupervisionError::NotFound(AgentId(id.0)),
        other => SupervisionError::Failed(other.to_string()),
    }
}

pub struct NoAgentSupervisor {
    events: broadcast::Sender<AgentEvent>,
}

impl Default for NoAgentSupervisor {
    fn default() -> Self {
        let (events, _) = broadcast::channel(1);
        Self { events }
    }
}

#[async_trait]
impl AgentSupervisor for NoAgentSupervisor {
    async fn tree(&self) -> Result<Vec<AgentView>, SupervisionError> {
        Ok(Vec::new())
    }

    async fn inspect(
        &self,
        _: AgentId,
        _: Option<u64>,
    ) -> Result<Vec<AgentEvent>, SupervisionError> {
        Ok(Vec::new())
    }

    async fn send_message(&self, _: AgentId, _: String) -> Result<(), SupervisionError> {
        Err(SupervisionError::Unavailable)
    }

    async fn follow_up(&self, _: AgentId, _: String) -> Result<AgentId, SupervisionError> {
        Err(SupervisionError::Unavailable)
    }

    async fn cancel(&self, _: AgentId) -> Result<(), SupervisionError> {
        Err(SupervisionError::Unavailable)
    }

    fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.events.subscribe()
    }
}
