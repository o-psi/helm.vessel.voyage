use super::composer::Composer;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;
use voyage_protocol::process::ProcessInfo;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub route: usize,
    pub session: Uuid,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct Run {
    pub run_id: Uuid,
    pub state: String,
    #[serde(default)]
    pub partial_text: String,
}

impl Run {
    pub fn active(&self) -> bool {
        matches!(
            self.state.as_str(),
            "accepted" | "running" | "awaiting_decision" | "cancel_requested"
        )
    }
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct Snapshot {
    pub session_id: Uuid,
    pub revision: u64,
    pub name: Option<String>,
    pub model: String,
    pub messages: Vec<Message>,
    pub run: Option<Run>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub history_truncated: bool,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct Decision {
    pub decision_id: Uuid,
    pub run_id: Uuid,
    pub incarnation: Uuid,
    pub expires_at_ms: u64,
    pub request: serde_json::Value,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Pending {
    pub command_id: Uuid,
    pub incarnation: Uuid,
    pub draft: String,
}

pub struct View {
    pub process: ProcessInfo,
    pub snapshot: Option<Snapshot>,
    pub draft: Composer,
    pub pending: Option<Pending>,
    pub scroll: u16,
    pub unread: bool,
    pub observed: Option<Instant>,
    pub error: Option<String>,
    pub rendered: std::cell::RefCell<Option<(u16, ratatui::text::Text<'static>)>>,
}

impl View {
    pub fn new(process: ProcessInfo) -> Self {
        Self {
            process,
            snapshot: None,
            draft: Composer::default(),
            pending: None,
            scroll: 0,
            unread: false,
            observed: None,
            error: None,
            rendered: Default::default(),
        }
    }
    pub fn title(&self) -> String {
        self.snapshot
            .as_ref()
            .and_then(|s| s.name.clone())
            .unwrap_or_else(|| self.process.session_id.to_string()[..8].to_string())
    }
}
