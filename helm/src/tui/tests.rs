use super::*;
use crate::supervision::SupervisionError;
use crate::terminal::{TerminalCell, TerminalError, TerminalState};
use crate::{
    config::CONFIG_OVERRIDE_SPECS,
    provider::ModelInfo,
    supervision::{
        AgentEvent as SupervisionEvent, AgentEventKind as SupervisionEventKind, AgentId,
        AgentStatus, AgentView,
    },
    terminal::{TerminalEvent, TerminalId, TerminalSnapshot, TerminalSummary},
    todo::{NewTodo, Priority, TodoStatus},
    tools::Approver,
};
use async_trait::async_trait;
use chrono::Utc;
use ratatui::style::{Color, Modifier};
use std::{collections::BTreeSet, path::PathBuf};
use std::{sync::Mutex, time::Duration};
use tokio::sync::oneshot;
use uuid::Uuid;

struct FakeSupervisor {
    agents: Vec<AgentView>,
    events: Vec<SupervisionEvent>,
    actions: Mutex<Vec<String>>,
    sender: tokio::sync::broadcast::Sender<SupervisionEvent>,
}

impl FakeSupervisor {
    fn new(agents: Vec<AgentView>) -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(16);
        Self {
            agents,
            events: Vec::new(),
            actions: Mutex::new(Vec::new()),
            sender,
        }
    }
}

#[async_trait]
impl AgentSupervisor for FakeSupervisor {
    async fn tree(&self) -> Result<Vec<AgentView>, SupervisionError> {
        Ok(self.agents.clone())
    }

    async fn inspect(
        &self,
        id: AgentId,
        after: Option<u64>,
    ) -> Result<Vec<SupervisionEvent>, SupervisionError> {
        if !self.agents.iter().any(|agent| agent.id == id) {
            return Err(SupervisionError::NotFound(id));
        }
        Ok(self
            .events
            .iter()
            .filter(|event| event.agent_id == id && after.is_none_or(|seq| event.sequence > seq))
            .cloned()
            .collect())
    }

    async fn send_message(&self, id: AgentId, text: String) -> Result<(), SupervisionError> {
        self.actions
            .lock()
            .unwrap()
            .push(format!("message:{id}:{text}"));
        Ok(())
    }

    async fn follow_up(&self, id: AgentId, text: String) -> Result<AgentId, SupervisionError> {
        self.actions
            .lock()
            .unwrap()
            .push(format!("follow:{id}:{text}"));
        Ok(AgentId(Uuid::new_v4()))
    }

    async fn cancel(&self, id: AgentId) -> Result<(), SupervisionError> {
        self.actions.lock().unwrap().push(format!("cancel:{id}"));
        Ok(())
    }

    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<SupervisionEvent> {
        self.sender.subscribe()
    }
}

fn agent(id: AgentId, parent: Option<AgentId>, task: &str) -> AgentView {
    AgentView {
        id,
        parent,
        task: task.into(),
        status: AgentStatus::Running,
        started_at: Some(Utc::now()),
        elapsed: Duration::from_secs(65),
        worktree: Some(PathBuf::from("/tmp/worktree")),
        recent_progress: vec!["running checks".into()],
        result: None,
        error: None,
    }
}

fn todo_store(directory: &tempfile::TempDir) -> Arc<TodoStore> {
    Arc::new(TodoStore::new(
        directory.path().join("todos.json"),
        crate::todo::TodoScope {
            workspace: directory.path().into(),
            session_id: None,
        },
    ))
}

async fn receive_todo_action(rx: &mut mpsc::UnboundedReceiver<UiEvent>) {
    assert!(matches!(rx.recv().await, Some(UiEvent::TodoAction(Ok(_)))));
    assert!(matches!(
        rx.recv().await,
        Some(UiEvent::TodoSnapshot(Ok(_)))
    ));
}

struct FakeTerminals {
    id: TerminalId,
    writes: Mutex<Vec<Vec<u8>>>,
    resizes: Mutex<Vec<(u16, u16)>>,
    events: tokio::sync::broadcast::Sender<TerminalEvent>,
}

impl FakeTerminals {
    fn new() -> Self {
        let (events, _) = tokio::sync::broadcast::channel(8);
        Self {
            id: TerminalId(uuid::Uuid::new_v4()),
            writes: Mutex::new(Vec::new()),
            resizes: Mutex::new(Vec::new()),
            events,
        }
    }
}

#[async_trait]
impl InteractiveTerminals for FakeTerminals {
    async fn list(&self) -> Result<Vec<TerminalSummary>, TerminalError> {
        Ok(vec![TerminalSummary {
            id: self.id,
            title: "shell".into(),
            state: TerminalState::Running,
        }])
    }
    async fn snapshot(&self, id: TerminalId) -> Result<TerminalSnapshot, TerminalError> {
        if id != self.id {
            return Err(TerminalError::NotFound(id));
        }
        Ok(TerminalSnapshot {
            id,
            title: "shell".into(),
            state: TerminalState::Running,
            revision: 1,
            cells: vec![vec![TerminalCell {
                text: "$ ready".into(),
                ..TerminalCell::default()
            }]],
            cursor: Some((2, 0)),
            dropped_unread_bytes: 0,
        })
    }
    async fn write(&self, id: TerminalId, bytes: Vec<u8>) -> Result<(), TerminalError> {
        if id != self.id {
            return Err(TerminalError::NotFound(id));
        }
        self.writes.lock().unwrap().push(bytes);
        Ok(())
    }
    async fn resize(&self, id: TerminalId, columns: u16, rows: u16) -> Result<(), TerminalError> {
        if id != self.id {
            return Err(TerminalError::NotFound(id));
        }
        self.resizes.lock().unwrap().push((columns, rows));
        Ok(())
    }
    fn subscribe(&self) -> tokio::sync::broadcast::Receiver<TerminalEvent> {
        self.events.subscribe()
    }
}

mod commands;
mod composer;
mod conversation;
mod palette;
mod questions;
mod supervisor;
mod terminals;
mod todos;

mod routing;
