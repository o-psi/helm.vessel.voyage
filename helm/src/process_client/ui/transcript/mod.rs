//! A readable projection of canonical messages, activity and provisional output.
mod history;
mod input;
mod layout;
use super::state::Message;
use ratatui::text::Line;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Message(usize),
    Activity(usize),
    Turn(uuid::Uuid),
    Live(uuid::Uuid),
    Pending,
    Notice,
}
#[derive(Clone)]
pub(super) struct Row {
    key: Key,
    offset: usize,
    line: Line<'static>,
}
#[derive(Clone)]
pub(super) struct Anchor {
    key: Key,
    offset: usize,
}
#[derive(Default)]
pub(in crate::process_client::ui) struct State {
    rows: Vec<Row>,
    width: u16,
    pub dirty: bool,
    top: usize,
    height: usize,
    anchor: Option<Anchor>,
    new_output: bool,
    pub details: bool,
    pub requested_from: Option<usize>,
    pub attempted: Option<u64>,
    pub attempted_from: Option<usize>,
    pub loaded_revision: Option<u64>,
    pub messages: Vec<Message>,
    pub loading: bool,
    pub live: Option<(uuid::Uuid, u64, u64, String)>,
    pub live_loading: bool,
    pub live_attempt: Option<(uuid::Uuid, u64, u64)>,
    pub error: Option<String>,
    pub search: Option<String>,
    query: String,
    search_next: bool,
    search_error: bool,
    pub delivery: Option<Delivery>,
}
#[derive(Clone)]
pub(super) struct Delivery {
    pub text: String,
    pub before: usize,
    pub label: String,
}
impl State {
    pub fn observe_growth(&mut self, grew: bool) {
        self.new_output |= grew && self.anchor.is_some();
    }
    fn remember(&mut self, top: usize) {
        self.top = top;
        self.anchor = self.rows.get(top).map(|r| Anchor {
            key: r.key.clone(),
            offset: r.offset,
        });
    }
    fn scroll(&mut self, up: bool, amount: usize) {
        let maximum = self.rows.len().saturating_sub(self.height);
        let top = if up {
            self.top.saturating_sub(amount)
        } else {
            (self.top + amount).min(maximum)
        };
        if !up && top == maximum {
            self.anchor = None;
            self.new_output = false;
            self.top = top;
        } else {
            self.remember(top);
        }
    }
}
pub(in crate::process_client::ui) use layout::draw;

impl State {
    /// Canonical history is append-only between explicit history changes. Preserve
    /// already hydrated text only while the overlapping snapshot agrees.
    pub fn merge_snapshot(&mut self, snapshot: &super::state::Snapshot) {
        if snapshot.lifecycle["deleted"] == true {
            self.messages.clear();
            self.rows.clear();
            self.loaded_revision = None;
            self.delivery = None;
            return;
        }
        if self.loaded_revision.is_none() || self.loaded_revision == Some(snapshot.revision) {
            return;
        }
        let next = self.messages.last().map_or(0, |m| m.message_index + 1);
        let agrees = snapshot.message_offset <= next
            && snapshot.messages.iter().all(|new| {
                self.messages
                    .iter()
                    .find(|old| old.message_index == new.message_index)
                    .is_none_or(|old| {
                        old.role == new.role
                            && old.tool_call_id == new.tool_call_id
                            && if new.projection_truncated {
                                old.content.starts_with(&new.content)
                            } else {
                                old.content == new.content
                            }
                    })
            });
        if !agrees {
            self.messages.clear();
            self.loaded_revision = None;
            self.anchor = None;
            return;
        }
        for new in &snapshot.messages {
            if let Some(old) = self
                .messages
                .iter_mut()
                .find(|m| m.message_index == new.message_index)
            {
                if !new.projection_truncated {
                    *old = new.clone();
                }
            } else {
                self.messages.push(new.clone());
            }
        }
        self.messages.sort_by_key(|m| m.message_index);
        // New incomplete messages still require hydration at the new revision.
        if self
            .messages
            .iter()
            .all(|m| !m.projection_truncated || m.role == "tool")
        {
            self.loaded_revision = Some(snapshot.revision);
        }
    }
}
