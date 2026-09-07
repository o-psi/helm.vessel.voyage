//! Public action summaries; arguments and results are never rendered as payloads.
use super::super::{
    presentation,
    state::{Snapshot, ToolCall, Turn},
};
use super::{Key, Message, Row, State, layout::note};
use crate::process_client::safe;
use ratatui::{
    style::{Color, Style},
    text::{Line, Span, Text},
};

fn compact(text: &str, limit: usize) -> String {
    let text = safe(text).split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = text.chars();
    let mut shown: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        shown.push('…');
    }
    shown
}
fn description(call: &ToolCall) -> String {
    let args = &call.arguments;
    let field = |name: &str| args[name].as_str().unwrap_or("");
    let path = if field("path").is_empty() {
        "."
    } else {
        field("path")
    };
    match call.name.as_str() {
        "shell" => format!("Run {}", field("command")),
        "read_file" => format!("Read {path}"),
        "write_file" => format!("Write {path} · {} lines", field("content").lines().count()),
        "apply_patch" => {
            let patch = field("patch");
            let added = patch
                .lines()
                .filter(|l| l.starts_with('+') && !l.starts_with("+++"))
                .count();
            let removed = patch
                .lines()
                .filter(|l| l.starts_with('-') && !l.starts_with("---"))
                .count();
            format!("Edit {path} · +{added}/−{removed} lines")
        }
        "search_files" => format!(
            "Search {path} for “{}”{}",
            field("query"),
            if field("glob").is_empty() {
                String::new()
            } else {
                format!(" in {}", field("glob"))
            }
        ),
        "list_directory" => format!(
            "List {path}{}",
            if args["recursive"] == true {
                " including subfolders"
            } else {
                ""
            }
        ),
        "process" => {
            let action = match field("action") {
                "start" => "Start program",
                "read" => "Read program output",
                "write" => "Send program input",
                "interrupt" => "Interrupt program",
                "terminate" => "Stop program",
                "list" => "List programs",
                "rename" => "Rename program",
                "resize" => "Resize program",
                "select" => "Select program",
                _ => "Use program",
            };
            // Never expose input data, environment, or internal program IDs.
            let target = if field("action") == "start" {
                field("command")
            } else {
                field("name")
            };
            format!("{action} {target}")
        }
        "subagent" => format!(
            "{} delegated work {} {}",
            presentation::label(field("action")),
            field("name"),
            if field("action") == "spawn" {
                field("task")
            } else {
                ""
            }
        ),
        "questions" => format!("Ask: {}", field("question")),
        "todo" => format!(
            "{} tasks {}",
            presentation::label(field("action")),
            field("title")
        ),
        _ => presentation::label(&call.name),
    }
}
fn failed(call: &ToolCall, result: &Message) -> bool {
    result.tool_success == Some(false)
        || (call.name == "shell"
            && result
                .content
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("exit: "))
                .is_some_and(|code| code != "0"))
}
pub(super) fn flush(
    output: &mut Vec<Row>,
    calls: &mut Vec<(usize, &ToolCall)>,
    messages: &[Message],
    snapshot: &Snapshot,
    state: &State,
    width: u16,
) {
    let Some((id, _)) = calls.first().copied() else {
        return;
    };
    let entries: Vec<_> = calls
        .iter()
        .map(|(index, call)| {
            let result = messages
                .iter()
                .find(|m| m.tool_call_id.as_deref() == Some(&call.id));
            let active = snapshot.run.as_ref().is_some_and(|r| {
                r.active() && r.message_start.is_some_and(|start| *index >= start)
            });
            (*call, result, active)
        })
        .collect();
    let failures = entries
        .iter()
        .filter(|(c, r, _)| r.is_some_and(|r| failed(c, r)))
        .count();
    let finished = entries.iter().filter(|(_, r, _)| r.is_some()).count();
    let accordion = entries.len() > 3;
    let expanded = state.expanded.get(&id).copied().unwrap_or(state.details);
    if accordion {
        let outcome = if failures > 0 {
            format!(" · {failures} failed")
        } else {
            String::new()
        };
        note(
            output,
            Key::ActivityHeader(id),
            format!(
                "{} {} actions · {finished}/{} finished{outcome} · Click / Ctrl+T",
                if expanded { "▼" } else { "▶" },
                entries.len(),
                entries.len()
            ),
            width,
        );
    }
    if !accordion || expanded {
        for (call, result, active) in entries {
            let status = match result {
                Some(r) if failed(call, r) => "Failed",
                Some(r) if r.tool_success == Some(true) => "Done",
                Some(_) => "Received",
                None if active => "Working",
                None => "Unconfirmed",
            };
            let outcome = if call.name == "shell" {
                result
                    .and_then(|r| r.content.lines().next())
                    .and_then(|l| l.strip_prefix("exit: "))
                    .filter(|c| *c != "0" && c.parse::<i32>().is_ok())
                    .map(|c| format!(" (exit {c})"))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let color = match status {
                "Failed" => Color::Red,
                "Working" => Color::Cyan,
                "Done" => Color::Green,
                _ => Color::DarkGray,
            };
            let text = Text::from(Line::from(vec![
                Span::styled(format!("{status}{outcome} · "), Style::default().fg(color)),
                Span::raw(compact(
                    &description(call),
                    usize::from(width).saturating_mul(2).saturating_sub(24),
                )),
            ]));
            super::layout::rows(
                output,
                Key::Activity(id),
                crate::markdown::wrap_text(text, width.into()),
            );
        }
    }
    calls.clear();
}
pub(super) fn separator(turn: &Turn, label: &str, width: u16) -> String {
    let duration = turn
        .started_at
        .zip(turn.finished_at)
        .and_then(|(start, end)| {
            let ms = (end - start).num_milliseconds();
            (ms >= 0).then(|| {
                let secs = ms / 1000;
                if secs < 1 {
                    "<1s".into()
                } else if secs < 60 {
                    format!("{secs}s")
                } else {
                    format!("{}m {:02}s", secs / 60, secs % 60)
                }
            })
        });
    let text = duration.map_or_else(|| label.to_owned(), |time| format!("{label} · {time}"));
    let text = format!("── {text} ");
    let remaining =
        usize::from(width).saturating_sub(unicode_width::UnicodeWidthStr::width(text.as_str()));
    format!("{text}{}\n", "─".repeat(remaining))
}
