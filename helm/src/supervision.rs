//! UI-facing adapter for supervising concurrent agent runs.
//!
//! Runtime implementations may be local or remote. TUI code depends only on this
//! bounded snapshot/event/action interface and never holds runtime task handles.

use std::{path::PathBuf, time::Duration};

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
