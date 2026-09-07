use super::super::{App, presentation, state::View};
use super::*;
use crate::{
    markdown::{self, RenderOptions},
    process_client::safe,
};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::Paragraph,
};

fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}
fn heading() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}
pub(super) fn rows(output: &mut Vec<Row>, key: Key, text: Text<'static>) {
    let mut offset = output.last().filter(|r| r.key == key).map_or(0, |r| {
        r.offset
            + r.line
                .to_string()
                .chars()
                .filter(|c| !c.is_whitespace())
                .count()
    });
    for line in text.lines {
        let size = line
            .to_string()
            .chars()
            .filter(|c| !c.is_whitespace())
            .count();
        output.push(Row {
            key: key.clone(),
            offset,
            line,
        });
        offset += size;
    }
}
pub(super) fn note(output: &mut Vec<Row>, key: Key, text: impl Into<String>, width: u16) {
    rows(
        output,
        key,
        markdown::wrap_text(Text::styled(text.into(), muted()), width.into()),
    );
}
fn body(text: &str, width: u16) -> Text<'static> {
    // Authored JSON is content, never a receipt inferred from its keys.
    let source = if serde_json::from_str::<serde_json::Value>(text)
        .is_ok_and(|v| v.is_object() || v.is_array())
    {
        let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        let fence = "`".repeat((longest + 1).max(3));
        format!("{fence}json\n{text}\n{fence}")
    } else {
        text.to_owned()
    };
    let mut rendered = markdown::render_markdown(
        &source,
        RenderOptions {
            width: width.into(),
            max_output_lines: 65_536,
            max_input_bytes: 64 * 1024 * 1024,
            ..Default::default()
        },
    );
    while rendered
        .lines
        .last()
        .is_some_and(|line| line.to_string().trim().is_empty())
    {
        rendered.lines.pop();
    }
    rendered
}
fn build(view: &View, state: &State, width: u16) -> Vec<Row> {
    let mut out = Vec::new();
    let Some(snapshot) = &view.snapshot else {
        note(
            &mut out,
            Key::Notice,
            if view.archived() {
                "Archived · Restore this voyage to load saved history. Unfinished work will not resume automatically."
            } else {
                "Connecting to your conversation…"
            },
            width,
        );
        return out;
    };
    if view.deleted() {
        note(
            &mut out,
            Key::Notice,
            "This conversation has been deleted.",
            width,
        );
        return out;
    }
    if view.archived() {
        note(
            &mut out,
            Key::Notice,
            "Archived · Saved conversation · /restore to continue",
            width,
        );
    }
    let messages = if state.loaded_revision == Some(snapshot.revision) {
        &state.messages
    } else {
        &snapshot.messages
    };
    let first = messages
        .first()
        .map_or(snapshot.message_offset, |m| m.message_index);
    if first > 0 {
        note(
            &mut out,
            Key::Notice,
            if view.archived() {
                "Restore this voyage to load earlier messages."
            } else {
                "Earlier messages available · Ctrl+Home loads more"
            },
            width,
        );
    }
    if messages.is_empty() {
        note(
            &mut out,
            Key::Notice,
            if view.archived() {
                "This archived conversation has no messages."
            } else {
                "Start a conversation\n\nDescribe what you want to do below."
            },
            width,
        );
    }
    let mut calls = Vec::new();
    for message in messages {
        if matches!(message.role.as_str(), "user" | "assistant") && !message.content.is_empty() {
            super::activity::flush(&mut out, &mut calls, messages, snapshot, state, width);
        }
        if matches!(message.role.as_str(), "user" | "assistant") && !message.content.is_empty() {
            let key = Key::Message(message.message_index);
            let final_answer = snapshot.turns.iter().any(|t| {
                t.phase == "completed" && t.message_end == Some(message.message_index + 1)
            });
            let label = if message.operator_name.is_some() {
                "Action"
            } else if message.role == "user" {
                "You"
            } else if final_answer {
                "Answer"
            } else if !message.tool_calls.is_empty() {
                "Update"
            } else {
                "Assistant"
            };
            let time = message
                .created_at
                .map(|t| format!("  {}", t.with_timezone(&chrono::Local).format("%H:%M")))
                .unwrap_or_default();
            rows(
                &mut out,
                Key::MessageHeading(message.message_index),
                Text::from(Line::styled(
                    format!("{label}{time}"),
                    heading().fg(if message.role == "user" {
                        Color::Cyan
                    } else {
                        Color::Reset
                    }),
                )),
            );
            let content = if message.operator_name.is_some() {
                if message.role == "user" {
                    presentation::operator_message(&message.content)
                } else {
                    presentation::structured_message(
                        &message.content,
                        message.operator_name.as_deref().unwrap_or_default(),
                    )
                }
            } else {
                None
            };
            rows(
                &mut out,
                key.clone(),
                body(content.as_deref().unwrap_or(&message.content), width),
            );
            if message.projection_truncated {
                note(
                    &mut out,
                    key.clone(),
                    if view.archived() {
                        "Restore this voyage to load the rest of this message."
                    } else {
                        "This message is incomplete · Loading full text; Ctrl+Home retries"
                    },
                    width,
                );
            }
            if let Some(status) = message.steering["status"].as_str() {
                let label = match status {
                    "queued" => "Queued for the next step",
                    "applied" => "Received during this run",
                    "not_applied" => "Not applied",
                    _ => "Delivery unconfirmed",
                };
                note(&mut out, key.clone(), label, width);
            }
            let ends_turn = snapshot.turns.iter().any(|t| {
                t.message_end == Some(message.message_index + 1)
                    && matches!(t.phase.as_str(), "completed" | "incomplete" | "interrupted")
            });
            if message.tool_calls.is_empty() && !ends_turn {
                note(&mut out, key, "", width);
            }
        }
        calls.extend(
            message
                .tool_calls
                .iter()
                .map(|call| (message.message_index, call)),
        );
        for turn in snapshot
            .turns
            .iter()
            .filter(|t| t.message_end == Some(message.message_index + 1))
        {
            super::activity::flush(&mut out, &mut calls, messages, snapshot, state, width);
            let label = match turn.phase.as_str() {
                "completed" => Some("Completed"),
                "incomplete" => Some("Work ended with unfinished items"),
                "interrupted" => Some("Interrupted · Work was not confirmed complete"),
                _ => None,
            };
            if let Some(label) = label {
                note(
                    &mut out,
                    Key::Turn(turn.run_id),
                    super::activity::separator(turn, label, width),
                    width,
                );
            }
        }
    }
    super::activity::flush(&mut out, &mut calls, messages, snapshot, state, width);
    if let Some(delivery) = &state.delivery {
        let saved = messages.iter().any(|m| {
            m.role == "user" && m.message_index >= delivery.before && m.content == delivery.text
        });
        if !saved {
            note(
                &mut out,
                Key::Pending,
                format!("You · {}", delivery.label),
                width,
            );
            rows(&mut out, Key::Pending, body(&delivery.text, width));
            note(&mut out, Key::Pending, "", width);
        }
    } else if let Some(pending) = &view.pending
        && !pending.preserve_draft
        && !pending.draft.starts_with('/')
    {
        note(
            &mut out,
            Key::Pending,
            "You · Delivery unconfirmed · F4 checks status",
            width,
        );
    }
    if let Some(run) = &snapshot.run
        && (run.state != "completed" || run.live_text.as_ref().is_some_and(|text| !text.is_empty()))
    {
        let key = Key::Live(run.run_id);
        note(
            &mut out,
            key.clone(),
            if run.state == "completed" {
                "Finishing response…"
            } else if run.active() && !snapshot.decisions.is_empty() {
                "Waiting for you"
            } else {
                presentation::run_state(&run.state)
            },
            width,
        );
        let loaded = state
            .live
            .as_ref()
            .filter(|(id, offset, total, _)| {
                *id == run.run_id
                    && Some(*offset) == run.live_text_offset
                    && *total == run.partial_text_bytes
            })
            .map(|(_, _, _, text)| text);
        if let Some(text) = loaded.or(run.live_text.as_ref())
            && !text.is_empty()
        {
            // The next canonical assistant message occupies this index. Keep
            // its reading anchor when the live suffix becomes saved text.
            rows(
                &mut out,
                Key::Message(snapshot.total_messages),
                body(text, width),
            );
        }
        if run.live_text_truncated && loaded.is_none() {
            note(
                &mut out,
                key.clone(),
                "Loading the rest of the live response…",
                width,
            );
        }
        if let Some(reason) = &run.failure_summary {
            note(&mut out, key.clone(), safe(reason), width);
        }
        if !run.active() && snapshot.pending_cleanup_run.is_some() {
            note(
                &mut out,
                key,
                "Cleanup is unconfirmed. Further work is blocked until recovery.",
                width,
            );
        }
    }
    if state.loading && !view.archived() {
        note(&mut out, Key::Notice, "Loading conversation…", width);
    }
    if let Some(error) = &state.error {
        note(
            &mut out,
            Key::Notice,
            format!("{} · Ctrl+Home retries", presentation::notice(error)),
            width,
        );
    }
    if let Some(error) = &view.error {
        note(&mut out, Key::Notice, presentation::notice(error), width);
    }
    out
}
pub(in crate::process_client::ui) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let area = area.inner(ratatui::layout::Margin::new(1, 1));
    let area = if area.width > 100 {
        Rect::new(area.x + (area.width - 100) / 2, area.y, 100, area.height)
    } else {
        area
    };
    let Some(view) = app.selected.and_then(|t| app.views.get(&t)) else {
        frame.render_widget(
            Paragraph::new("Welcome to Helm\n\nCtrl+N starts a voyage."),
            area,
        );
        return;
    };
    if let Some(panel) = &view.panel {
        let text = super::super::panels::display(panel, area.width);
        let maximum = text
            .lines
            .len()
            .saturating_sub(area.height.into())
            .min(u16::MAX as usize) as u16;
        frame.render_widget(
            Paragraph::new(text).scroll((view.scroll.min(maximum), 0)),
            area,
        );
        return;
    }
    let mut state = view.transcript.borrow_mut();
    let resized = state.width != area.width;
    if state.dirty || state.rows.is_empty() || resized {
        state.rows = build(view, &state, area.width);
        state.width = area.width;
        state.dirty = false;
    }
    let height = usize::from(area.height.saturating_sub(1));
    let maximum = state.rows.len().saturating_sub(height);
    let mut top = state
        .anchor
        .as_ref()
        .and_then(|a| {
            state
                .rows
                .iter()
                .enumerate()
                .filter(|(_, r)| r.key == a.key && r.offset <= a.offset)
                .map(|(i, _)| i)
                .next_back()
        })
        .unwrap_or(maximum)
        .min(maximum);
    if state.search_next {
        state.search_next = false;
        let query = state.query.to_lowercase();
        let found = (top + 1..state.rows.len())
            .chain(0..=top.min(state.rows.len().saturating_sub(1)))
            .find(|&i| {
                state
                    .rows
                    .get(i)
                    .is_some_and(|r| r.line.to_string().to_lowercase().contains(&query))
            });
        if let Some(index) = found {
            top = index.min(maximum);
            state.anchor = Some(Anchor {
                key: state.rows[index].key.clone(),
                offset: state.rows[index].offset,
            });
            state.search_error = false;
        } else {
            state.search_error = true;
        }
    }
    let query = state.query.to_lowercase();
    state.hits.clear();
    let visible = state
        .rows
        .iter()
        .skip(top)
        .take(height)
        .enumerate()
        .map(|(y, row)| {
            let mut line = row.line.clone();
            if matches!(row.key, Key::ActivityHeader(_)) {
                let rect = Rect::new(area.x, area.y + y as u16, area.width, 1);
                if app.sidebar.pointer.is_some_and(|p| rect.contains(p)) {
                    line = line.style(
                        Style::default()
                            .bg(Color::DarkGray)
                            .fg(Color::White)
                            .add_modifier(Modifier::UNDERLINED),
                    );
                }
            }
            if !query.is_empty() && line.to_string().to_lowercase().contains(&query) {
                line.style(Style::default().bg(Color::DarkGray).fg(Color::White))
            } else {
                line
            }
        })
        .collect::<Vec<_>>();
    state.hits = state
        .rows
        .iter()
        .skip(top)
        .take(height)
        .enumerate()
        .filter_map(|(y, row)| {
            if let Key::ActivityHeader(id) = row.key {
                Some((Rect::new(area.x, area.y + y as u16, area.width, 1), id))
            } else {
                None
            }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Text::from(visible)),
        Rect {
            height: area.height.saturating_sub(1),
            ..area
        },
    );
    let hint = if let Some(search) = &state.search {
        format!("Find: {search} · Enter next · Esc close")
    } else if state.search_error {
        "No match in loaded messages · Ctrl+Home loads earlier history".into()
    } else if state.anchor.is_some() && area.width < 65 {
        format!(
            "{} · Ctrl+End Latest",
            if state.new_output {
                "New output"
            } else {
                "Reading earlier"
            }
        )
    } else if state.anchor.is_some() {
        format!(
            "Reading earlier{} · Ctrl+End Latest · Ctrl+Home Older",
            if state.new_output {
                " · New output"
            } else {
                ""
            }
        )
    } else if area.width < 45 {
        "^T Activity · ^F Find · PgUp Earlier".into()
    } else {
        "Ctrl+T Activity · Ctrl+F Find · PgUp Earlier".into()
    };
    frame.render_widget(
        Paragraph::new(hint).style(muted()),
        Rect::new(
            area.x,
            area.bottom().saturating_sub(1),
            area.width,
            u16::from(area.height > 0),
        ),
    );
    state.top = top;
    state.height = height;
}
