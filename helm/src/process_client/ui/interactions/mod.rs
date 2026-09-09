//! Every runtime decision shares one review surface; questions never grant authority.
mod input;
mod render;
mod response;
use super::{composer::Composer, state::Target};
use anyhow::{Result, ensure};
pub(super) use render::draw;
use std::collections::BTreeMap;
use uuid::Uuid;

type Identity = (Target, Uuid);
#[derive(Clone, Copy)]
enum Control {
    Choice(Option<usize>),
    Confirm,
    Cancel,
    Previous,
    Next,
    ScrollUp,
    ScrollDown,
    Paste,
    Clear,
}

struct AnswerDraft {
    text: Composer,
    option: Option<usize>,
    editing: bool,
}
impl Default for AnswerDraft {
    fn default() -> Self {
        Self {
            text: Composer::default(),
            option: Some(0),
            editing: false,
        }
    }
}
#[derive(Default)]
pub(super) struct Review {
    // Only the last rendered identity can receive a response.
    displayed: Option<Identity>,
    selected: Option<Identity>,
    hits: Vec<(ratatui::layout::Rect, Control)>,
    area: ratatui::layout::Rect,
    pub(super) focused: bool,
    scroll: u16,
    follow_selection: bool,
    answers: BTreeMap<Identity, AnswerDraft>,
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |time| {
            time.as_millis().min(u64::MAX as u128) as u64
        })
}
fn validate_response(request: &serde_json::Value, response: &serde_json::Value) -> Result<()> {
    match request["kind"].as_str() {
        Some("approval") => ensure!(
            response == "approved" || response == "denied",
            "invalid approval response"
        ),
        Some("question") => match response["status"].as_str() {
            Some("selected") => {
                let index = response["index"]
                    .as_u64()
                    .and_then(|index| usize::try_from(index).ok());
                ensure!(
                    index.and_then(|index| request["question"]["options"].get(index))
                        == Some(&response["answer"]),
                    "answer does not match a question option"
                );
            }
            Some("custom") => {
                let answer = response["answer"].as_str().unwrap_or("");
                ensure!(
                    !answer.trim().is_empty()
                        && answer.len() <= 4096
                        && !answer.chars().any(char::is_control),
                    "answer must be nonblank, at most 4096 bytes, and contain no control characters"
                );
            }
            Some("cancelled") => {}
            _ => anyhow::bail!("invalid question response"),
        },
        _ => anyhow::bail!("this runtime interaction kind is not supported by this Helm"),
    }
    Ok(())
}

impl super::App {
    pub(super) fn sync_interactions(&self) {
        let mut review = self.interactions.borrow_mut();
        let next = self.selected.and_then(|target| {
            let view = self.views.get(&target)?;
            if self.help
                || self.explore.is_some()
                || self.sidebar.menu.is_some()
                || view.panel.is_some()
                || view.terminals.open
            {
                return None;
            }
            let snapshot = view.snapshot.as_ref()?;
            let decision = snapshot
                .decisions
                .iter()
                .find(|d| review.selected == Some((target, d.decision_id)))
                .or_else(|| snapshot.decisions.first())?;
            Some((target, decision.decision_id))
        });
        if review.selected != next {
            review.selected = next;
            review.displayed = None;
            review.hits.clear();
            review.area = Default::default();
            review.scroll = 0;
            review.follow_selection = true;
        }
        review.focused = next.is_some();
    }
}

impl super::App {
    pub(super) fn question_editor(&self) -> Option<(super::right_panel::Editor, String)> {
        let review = self.interactions.borrow();
        let (target, id) = review.displayed?;
        if !review.focused || review.selected != review.displayed || self.selected != Some(target) {
            return None;
        }
        let view = self.views.get(&target)?;
        let decision = view
            .snapshot
            .as_ref()?
            .decisions
            .iter()
            .find(|d| d.decision_id == id)?;
        let answer = review.answers.get(&(target, id))?;
        if view.pending.is_some()
            || decision.expires_at_ms <= now_ms()
            || !answer.editing
            || decision.request["kind"].as_str() != Some("question")
        {
            return None;
        }
        Some((
            super::right_panel::Editor::Question(target, id),
            answer.text.text.clone(),
        ))
    }
}
