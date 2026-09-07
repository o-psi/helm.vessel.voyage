use super::super::App;
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::json;

impl App {
    pub(in crate::process_client::ui) fn interaction_input(
        &mut self,
        event: &Event,
    ) -> Result<bool> {
        let mut review = self.interactions.borrow_mut();
        let Some((target, decision_id)) = review.displayed else {
            // Navigation must repaint before focused input can target the next request.
            return Ok(review.focused
                && review
                    .selected
                    .is_some_and(|(target, _)| self.selected == Some(target)));
        };
        if self.selected != Some(target) {
            return Ok(false);
        }
        let snapshot = self
            .views
            .get(&target)
            .and_then(|view| view.snapshot.as_ref())
            .context("waiting for interaction snapshot")?;
        let Some(decision) = snapshot
            .decisions
            .iter()
            .find(|decision| decision.decision_id == decision_id)
        else {
            // Do not send keystrokes intended for a resolved question into the main composer.
            return Ok(review.focused);
        };
        let kind = decision.request["kind"].as_str().unwrap_or("");
        if let Event::Paste(text) = event {
            if review.focused && kind == "question" {
                let answer = review.answers.entry((target, decision_id)).or_default();
                let text: String = super::super::safe(text)
                    .chars()
                    .filter(|ch| !ch.is_control())
                    .collect();
                ensure!(
                    answer.text.text.len().saturating_add(text.len()) <= 4096,
                    "answer limit is 4096 bytes"
                );
                answer.option = None;
                answer.text.insert_str(&text);
                return Ok(true);
            }
            return Ok(false);
        }
        let Event::Key(key) = event else {
            return Ok(false);
        };
        let control = key.modifiers == KeyModifiers::CONTROL;
        if key.kind != KeyEventKind::Press {
            return Ok(review.focused || (control && matches!(key.code, KeyCode::Char('a' | 'd'))));
        }
        match key.code {
            KeyCode::F(2) => review.focused = !review.focused,
            KeyCode::Esc if review.focused => review.focused = false,
            KeyCode::F(6) | KeyCode::F(7) => {
                let index = snapshot
                    .decisions
                    .iter()
                    .position(|decision| decision.decision_id == decision_id)
                    .unwrap_or(0);
                let next = if key.code == KeyCode::F(6) {
                    (index + snapshot.decisions.len() - 1) % snapshot.decisions.len()
                } else {
                    (index + 1) % snapshot.decisions.len()
                };
                review.selected = Some((target, snapshot.decisions[next].decision_id));
                review.displayed = None;
                review.scroll = 0;
            }
            KeyCode::PageUp if review.focused => review.scroll = review.scroll.saturating_sub(10),
            KeyCode::PageDown if review.focused => review.scroll = review.scroll.saturating_add(10),
            KeyCode::Char('a' | 'd') if control && kind == "approval" => {
                let response = json!(if key.code == KeyCode::Char('a') {
                    "approved"
                } else {
                    "denied"
                });
                drop(review);
                self.respond_to_interaction(target, decision_id, response)?;
            }
            KeyCode::Char('d') if control && kind == "question" && review.focused => {
                drop(review);
                self.respond_to_interaction(target, decision_id, json!({"status":"cancelled"}))?;
            }
            _ if review.focused && kind == "question" => {
                let options = decision.request["question"]["options"]
                    .as_array()
                    .context("question options unavailable")?;
                let answer = review.answers.entry((target, decision_id)).or_default();
                match key.code {
                    KeyCode::Up | KeyCode::Down => {
                        let index = answer.option.unwrap_or(options.len());
                        let next = if key.code == KeyCode::Up {
                            (index + options.len()) % (options.len() + 1)
                        } else {
                            (index + 1) % (options.len() + 1)
                        };
                        answer.option = (next < options.len()).then_some(next);
                    }
                    KeyCode::Enter if key.modifiers.is_empty() => {
                        let response = if let Some(index) = answer.option {
                            json!({"status":"selected","index":index,"answer":options[index]})
                        } else {
                            json!({"status":"custom","answer":answer.text.text})
                        };
                        drop(review);
                        self.respond_to_interaction(target, decision_id, response)?;
                    }
                    KeyCode::Char(ch)
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        ensure!(
                            answer.text.text.len() + ch.len_utf8() <= 4096 && !ch.is_control(),
                            "answer limit is 4096 bytes without control characters"
                        );
                        answer.option = None;
                        answer.text.insert(ch);
                    }
                    KeyCode::Backspace => {
                        answer.option = None;
                        answer.text.backspace();
                    }
                    KeyCode::Delete => {
                        answer.option = None;
                        answer.text.delete();
                    }
                    KeyCode::Home => answer.text.line_start(),
                    KeyCode::End => answer.text.line_end(),
                    KeyCode::Left => {
                        answer.text.cursor = answer.text.text[..answer.text.cursor]
                            .char_indices()
                            .next_back()
                            .map_or(0, |(index, _)| index)
                    }
                    KeyCode::Right => {
                        if let Some(ch) = answer.text.text[answer.text.cursor..].chars().next() {
                            answer.text.cursor += ch.len_utf8();
                        }
                    }
                    _ => {}
                }
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
