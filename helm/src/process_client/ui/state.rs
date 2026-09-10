use super::composer::{Composer, PromptHistory};
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;
use voyage_protocol::vessel::ProcessInfo;

/// An immutable connection plus one observation activation. Never a vector position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Route {
    pub id: Uuid,
    pub generation: u64,
}
impl Route {
    pub fn of(client: &crate::process_client::transport::Client) -> Self {
        Self {
            id: client.id(),
            generation: client.generation(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Target {
    pub route: Route,
    pub session: Uuid,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Message {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordination: Option<voyage_protocol::coordination::CoordinationSource>,
    #[serde(default)]
    pub message_index: usize,
    #[serde(default)]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub operator_name: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: Option<String>,
    #[serde(default)]
    pub tool_success: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<voyage_protocol::tool_result::ToolOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_outcome: Option<voyage_protocol::tool_result::ToolOutcome>,
    #[serde(default)]
    pub steering: serde_json::Value,
    #[serde(default)]
    pub projection_truncated: bool,
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<voyage_protocol::content::ContentPart>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}
#[derive(Clone, Deserialize, PartialEq)]
pub struct Turn {
    #[serde(default)]
    pub failure_summary: Option<String>,
    #[serde(default)]
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub run_id: Uuid,
    pub phase: String,
    pub message_start: Option<usize>,
    pub message_end: Option<usize>,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct Run {
    #[serde(default)]
    pub message_start: Option<usize>,
    #[serde(default)]
    pub live_text: Option<String>,
    #[serde(default)]
    pub live_text_truncated: bool,
    #[serde(default)]
    pub live_text_offset: Option<u64>,
    #[serde(default)]
    pub partial_text_bytes: u64,
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
pub struct Cleanup {
    pub run_id: Uuid,
    pub phase: String,
    #[serde(default)]
    pub pending: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub retryable: bool,
}

#[derive(Clone, Deserialize, PartialEq)]
pub struct Snapshot {
    #[serde(default)]
    pub total_messages: usize,
    #[serde(default)]
    pub message_offset: usize,
    #[serde(default)]
    pub turns: Vec<Turn>,
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
    #[serde(default)]
    pub inference: Option<super::inference::Settings>,
    #[serde(default)]
    pub inference_next_turn: bool,
    #[serde(default)]
    pub inference_current: Option<super::inference::Settings>,
    #[serde(default)]
    pub access: Option<String>,
    pub messages: Vec<Message>,
    pub run: Option<Run>,
    #[serde(default)]
    pub pending_cleanup_run: Option<Uuid>,
    #[serde(default)]
    pub cleanup: Option<Cleanup>,
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
    /// Exact public request retained before dispatch; never private input.
    #[serde(default)]
    pub original: Option<Box<voyage_protocol::vessel::VoyageCommand>>,
    /// Vessel lifecycle operations have separate admission semantics.
    #[serde(default)]
    pub receipt_only: bool,
}

impl Pending {
    pub fn resolution(&self) -> voyage_protocol::vessel::VoyageCommand {
        use voyage_protocol::vessel::VoyageCommand;
        if self.receipt_only
            || (self.original.is_none()
                && (self.draft.trim() == "/branch"
                    || self.draft.trim_start().starts_with("/branch ")
                    || self.draft.trim() == "/restore"))
        {
            VoyageCommand::Receipt {
                command_id: self.command_id,
            }
        } else {
            VoyageCommand::Resolve {
                command_id: self.command_id,
                original: self.original.clone(),
            }
        }
    }
}

pub struct View {
    pub process: ProcessInfo,
    pub snapshot: Option<Snapshot>,
    pub draft: Composer,
    pub images: Vec<super::attachments::Image>,
    pub history: PromptHistory,
    pub pending: Option<Pending>,
    pub panel: Option<String>,
    pub terminals: super::terminals::Browser,
    pub scroll: u16,
    pub transcript: std::cell::RefCell<super::transcript::State>,
    pub unread: bool,
    /// Helm-local acknowledgement, bound to a run rather than process incarnation.
    pub acknowledged_completion: Option<Uuid>,
    /// Set only when the result conversation is actually rendered this visit.
    pub viewed_completion: std::cell::Cell<Option<Uuid>>,
    pub observed: Option<Instant>,
    pub error: Option<String>,
    /// A route-level transport failure, distinct from a conversation read failure.
    pub connection_unavailable: bool,
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
            images: Vec::new(),
            history: PromptHistory::default(),
            pending: None,
            panel: None,
            terminals: Default::default(),
            scroll: 0,
            transcript: Default::default(),
            unread: false,
            acknowledged_completion: None,
            viewed_completion: Default::default(),
            observed: None,
            error: None,
            connection_unavailable: false,
            rendered: Default::default(),
        }
    }
    pub fn title(&self) -> String {
        // Archived owners no longer participate in snapshot refresh. Their final
        // metadata must take precedence over any retained live snapshot.
        if let Some(name) = self.process.archive.as_ref().and_then(|a| a.name.as_ref()) {
            return name.clone();
        }
        let name = self
            .snapshot
            .as_ref()
            .and_then(|s| s.name.as_deref())
            .or(self.process.name.as_deref())
            .or_else(|| {
                self.process
                    .archive
                    .as_ref()
                    .and_then(|a| a.name.as_deref())
            });
        if let Some(name) = name
            && !default_name(name)
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
            .unwrap_or_else(|| {
                let label = if self.error.is_some() || self.connection_unavailable {
                    "Unavailable voyage"
                } else if self.snapshot.is_none() {
                    "Loading voyage"
                } else {
                    "Saved voyage"
                };
                format!("{label} {}", &self.process.session_id.to_string()[..8])
            })
    }
}

fn default_name(name: &str) -> bool {
    name.is_empty()
        || name.strip_prefix("session-").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_hexdigit())
        })
}

impl super::App {
    /// One activity order for both presentation and keyboard navigation. Unknown
    /// timestamps (older owners or unavailable snapshots) sort last, not as now.
    pub(super) fn ordered_targets(&self) -> Vec<Target> {
        let mut targets: Vec<_> = self
            .views
            .iter()
            .filter(|(target, view)| {
                !view.deleted()
                    && view.archived() == self.archives
                    && self.vessel_filter.is_none_or(|id| target.route.id == id)
            })
            .map(|(target, _)| *target)
            .collect();
        targets.sort_by_key(|target| {
            let activity = self.views[target]
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.last_message_at.or(snapshot.created_at));
            (
                self.views[target].sidebar_suspended(),
                std::cmp::Reverse(activity),
                *target,
            )
        });
        targets
    }
}
