//! A readable projection of canonical messages, activity and provisional output.
mod activity;
mod history;
mod input;
mod layout;
use super::state::Message;
use ratatui::text::Line;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Message(usize),
    MessageHeading(usize),
    Activity(usize),
    ActivityHeader(usize),
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
    expanded: std::collections::BTreeMap<usize, bool>,
    hits: Vec<(ratatui::layout::Rect, usize)>,
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
    pub parts: Vec<voyage_protocol::content::ContentPart>,
    pub before: usize,
    pub label: String,
}
impl Delivery {
    pub(super) fn matches(&self, message: &Message) -> bool {
        message.role == "user" && message.message_index >= self.before
            && message.content == self.text && message.parts == self.parts
    }
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
                            && old.parts == new.parts
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

#[cfg(test)]
mod attachment_tests {
    use super::*;
    use voyage_protocol::content::{ContentPart, ImageAttachment, ImageMediaType};
    #[test]
    fn attachment_delivery_equality_requires_metadata_not_only_empty_text() {
        let attachment = ImageAttachment {
            id: uuid::Uuid::from_u128(74), name: "image.png".into(),
            media_type: ImageMediaType::Png, byte_size: 70, width: 1, height: 1,
                sha256: "a".repeat(64),
        };
        let mut message: Message = serde_json::from_value(serde_json::json!({
            "role": "user", "content": "", "message_index": 3
        })).unwrap();
        assert!(message.parts.is_empty());
        let delivery = Delivery {
            text: String::new(), before: 3, label: "Sending".into(),
            parts: vec![ContentPart::Image { attachment }],
        };
        assert!(!delivery.matches(&message));
        message.parts = delivery.parts.clone();
        assert!(delivery.matches(&message));
        message.message_index = 2;
        assert!(!delivery.matches(&message));
        message.message_index = 3;
        let ContentPart::Image { attachment } = &mut message.parts[0] else { unreachable!() };
        attachment.id = uuid::Uuid::from_u128(75);
        assert!(!delivery.matches(&message));
    }
}
