use super::{super::App, Control};
use anyhow::{Context, Result, ensure};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use serde_json::json;

impl App {
    pub(in crate::process_client::ui) fn interaction_input(
        &mut self,
        event: &Event,
    ) -> Result<bool> {
        let mut review = self.interactions.borrow_mut();
        if !review.focused {
            return Ok(false);
        }
        if matches!(event, Event::Resize(..)) {
            review.follow_selection = true;
            review.hits.clear();
            review.area = Default::default();
            review.displayed = None;
            return Ok(true);
        }
        let Some((target, decision_id)) = review.displayed else {
            return Ok(true);
        };
        if self.selected != Some(target) || review.selected != review.displayed {
            return Ok(true);
        }
        let view = self.views.get(&target).context("request unavailable")?;
        let snapshot = view.snapshot.as_ref().context("waiting for request")?;
        let Some(decision) = snapshot
            .decisions
            .iter()
            .find(|d| d.decision_id == decision_id)
        else {
            return Ok(true);
        };
        // An unconfirmed response cannot be replaced by another action.
        if view.pending.is_some() || decision.expires_at_ms <= super::now_ms() {
            return Ok(true);
        }
        let kind = decision.request["kind"].as_str().unwrap_or("");
        if let Event::Paste(text) = event {
            let answer = review.answers.entry((target, decision_id)).or_default();
            if kind == "question" && answer.editing {
                let text: String = super::super::safe(text)
                    .chars()
                    .filter(|c| !c.is_control())
                    .collect();
                ensure!(
                    answer.text.text.len() + text.len() <= 4096,
                    "Answer is too long."
                );
                answer.text.insert_str(&text);
            }
            return Ok(true);
        }
        let key = match event {
            Event::Key(key) => *key,
            Event::Mouse(mouse) => {
                let point = (mouse.column, mouse.row).into();
                if !review.area.contains(point) {
                    return Ok(true);
                }
                match mouse.kind {
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                        review.scroll = if mouse.kind == MouseEventKind::ScrollUp {
                            review.scroll.saturating_sub(3)
                        } else {
                            review.scroll.saturating_add(3)
                        };
                        review.follow_selection = false;
                        review.hits.clear();
                        return Ok(true);
                    }
                    MouseEventKind::Down(MouseButton::Left) => {}
                    _ => return Ok(true),
                }
                let control = review
                    .hits
                    .iter()
                    .find(|(area, _)| area.contains(point))
                    .map(|(_, control)| *control);
                let Some(control) = control else {
                    return Ok(true);
                };
                review.hits.clear();
                let code = match control {
                    Control::Choice(option) => {
                        let answer = review.answers.entry((target, decision_id)).or_default();
                        answer.option = option;
                        answer.editing = kind == "question" && option.is_none();
                        review.follow_selection = true;
                        return Ok(true);
                    }
                    Control::Confirm => KeyCode::Enter,
                    Control::Cancel => KeyCode::Esc,
                    Control::Previous => KeyCode::Left,
                    Control::Next => KeyCode::Right,
                };
                KeyEvent::new(code, KeyModifiers::NONE)
            }
            _ => return Ok(true),
        };
        if key.kind != KeyEventKind::Press {
            return Ok(true);
        }
        // Geometry is valid only for the state painted before this input.
        review.hits.clear();
        let editing = review
            .answers
            .get(&(target, decision_id))
            .is_some_and(|a| a.editing);
        if !editing
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
            && snapshot.decisions.len() > 1
        {
            let index = snapshot
                .decisions
                .iter()
                .position(|d| d.decision_id == decision_id)
                .unwrap_or(0);
            let next = if key.code == KeyCode::Left {
                (index + snapshot.decisions.len() - 1) % snapshot.decisions.len()
            } else {
                (index + 1) % snapshot.decisions.len()
            };
            review.selected = Some((target, snapshot.decisions[next].decision_id));
            review.displayed = None;
            review.scroll = 0;
            review.follow_selection = true;
            return Ok(true);
        }
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            review.scroll = if key.code == KeyCode::PageUp {
                review.scroll.saturating_sub(5)
            } else {
                review.scroll.saturating_add(5)
            };
            review.follow_selection = false;
            return Ok(true);
        }
        if !matches!(kind, "question" | "approval") {
            return Ok(true);
        }
        let options = if kind == "question" {
            decision.request["question"]["options"]
                .as_array()
                .context("Question choices unavailable")?
                .len()
        } else {
            2
        };
        let answer = review.answers.entry((target, decision_id)).or_default();
        let mut response = None;
        match key.code {
            KeyCode::Esc if answer.editing => {
                answer.editing = false;
                review.follow_selection = true;
            }
            KeyCode::Esc => {
                response = Some(if kind == "approval" {
                    json!("denied")
                } else {
                    json!({"status":"cancelled"})
                })
            }
            KeyCode::Up | KeyCode::Down if !answer.editing => {
                let count = options + usize::from(kind == "question");
                let index = answer.option.unwrap_or(options).min(count - 1);
                let next = if key.code == KeyCode::Up {
                    (index + count - 1) % count
                } else {
                    (index + 1) % count
                };
                answer.option = (next < options).then_some(next);
                review.follow_selection = true;
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                response = if kind == "approval" {
                    Some(json!(if answer.option == Some(1) {
                        "approved"
                    } else {
                        "denied"
                    }))
                } else if let Some(index) = answer.option {
                    Some(
                        json!({"status":"selected","index":index,"answer":decision.request["question"]["options"][index]}),
                    )
                } else if !answer.editing {
                    answer.editing = true;
                    review.follow_selection = true;
                    None
                } else if answer.text.text.trim().is_empty() {
                    None
                } else {
                    Some(json!({"status":"custom","answer":answer.text.text}))
                };
            }
            KeyCode::Char(ch)
                if answer.editing
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                ensure!(
                    answer.text.text.len() + ch.len_utf8() <= 4096 && !ch.is_control(),
                    "Answer is too long."
                );
                answer.text.insert(ch);
            }
            KeyCode::Backspace if answer.editing => answer.text.backspace(),
            KeyCode::Delete if answer.editing => answer.text.delete(),
            KeyCode::Home if answer.editing => answer.text.line_start(),
            KeyCode::End if answer.editing => answer.text.line_end(),
            KeyCode::Left if answer.editing => {
                answer.text.cursor = answer.text.text[..answer.text.cursor]
                    .char_indices()
                    .next_back()
                    .map_or(0, |(index, _)| index)
            }
            KeyCode::Right if answer.editing => {
                if let Some(ch) = answer.text.text[answer.text.cursor..].chars().next() {
                    answer.text.cursor += ch.len_utf8();
                }
            }
            _ => {}
        }
        if let Some(response) = response {
            drop(review);
            self.respond_to_interaction(target, decision_id, response)?;
        }
        Ok(true)
    }
}
