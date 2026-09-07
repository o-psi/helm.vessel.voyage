use super::composer::{Composer, PromptHistory};
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
    #[serde(default)]
    pub failure_summary: Option<String>,
    #[serde(default)]
    pub partial_text_truncated: bool,
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
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub last_message_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub observation_cursor: Option<u64>,
    pub name: Option<String>,
    pub model: String,
    pub messages: Vec<Message>,
    pub run: Option<Run>,
    #[serde(default)]
    pub pending_cleanup_run: Option<Uuid>,
    #[serde(default)]
    pub decisions: Vec<Decision>,
    #[serde(default)]
    pub history_truncated: bool,
    #[serde(default)]
    pub lifecycle: serde_json::Value,
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
    #[serde(default)]
    pub preserve_draft: bool,
}

pub struct View {
    pub process: ProcessInfo,
    pub snapshot: Option<Snapshot>,
    pub draft: Composer,
    pub history: PromptHistory,
    pub pending: Option<Pending>,
    pub panel: Option<String>,
    pub terminals: super::terminals::Browser,
    pub scroll: u16,
    pub unread: bool,
    pub observed: Option<Instant>,
    pub error: Option<String>,
    pub rendered: std::cell::RefCell<Option<(u16, ratatui::text::Text<'static>)>>,
}

impl View {
    pub(super) fn deleted(&self) -> bool {
        self.process.deletion.is_some()
            || self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.lifecycle["deleted"] == true)
    }
    pub(super) fn archived(&self) -> bool {
        self.process.archive.is_some()
            || self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.lifecycle["archived"] == true)
    }
    pub fn new(process: ProcessInfo) -> Self {
        Self {
            process,
            snapshot: None,
            draft: Composer::default(),
            history: PromptHistory::default(),
            pending: None,
            panel: None,
            terminals: Default::default(),
            scroll: 0,
            unread: false,
            observed: None,
            error: None,
            rendered: Default::default(),
        }
    }
    pub fn title(&self) -> String {
        let name = self
            .snapshot
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .or_else(|| {
                self.process
                    .archive
                    .as_ref()
                    .and_then(|a| a.name.as_deref())
            });
        if let Some(name) = name
            && !name
                .strip_prefix("session-")
                .is_some_and(|suffix| suffix.chars().all(|ch| ch.is_ascii_hexdigit()))
        {
            return name.to_owned();
        }
        self.snapshot
            .as_ref()
            .and_then(|s| {
                s.messages
                    .iter()
                    .find(|m| m.role == "user" && !m.content.starts_with("Operator tool "))
            })
            .map(|m| {
                m.content
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .take(7)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "New voyage".into())
    }
}

impl super::App {
    /// One activity order for both presentation and keyboard navigation. Unknown
    /// timestamps (older owners or unavailable snapshots) sort last, not as now.
    pub(super) fn ordered_targets(&self) -> Vec<Target> {
        let mut targets: Vec<_> = self
            .views
            .iter()
            .filter(|(_, view)| !view.deleted() && view.archived() == self.archives)
            .map(|(target, _)| *target)
            .collect();
        targets.sort_by_key(|target| {
            let activity = self.views[target]
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.last_message_at.or(snapshot.created_at));
            (std::cmp::Reverse(activity), *target)
        });
        targets
    }
}
