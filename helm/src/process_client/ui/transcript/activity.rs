//! Public action summaries; arguments and results are never rendered as payloads.
use super::super::{
    presentation,
    state::{Snapshot, ToolCall, Turn},
};
use super::{Key, Message, Row, State, layout::note};
use crate::process_client::safe;
use ratatui::text::{Line, Span, Text};

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
        "vessel" => format!(
            "{} voyages{}{}",
            presentation::label(field("action")),
            if field("target").is_empty() {
                String::new()
            } else {
                format!(" on {}", field("target"))
            },
            if field("name").is_empty() {
                String::new()
            } else {
                format!(" · {}", field("name"))
            },
        ),
        // Historical transcripts can contain the retired bookkeeping tool.
        "completion" => match field("action") {
            "snapshot" => "Check task outcomes".into(),
            "read" => format!("Read {} outcome", field("kind")),
            "account" => format!("Record {} review", field("kind")),
            "adopt" => format!("Include earlier {} work", field("kind")),
            _ => "Review task outcomes".into(),
        },
        "questions" => format!("Ask: {}", field("question")),
        "todo" => format!(
            "{} tasks {}",
            presentation::label(field("action")),
            field("title")
        ),
        _ => presentation::label(&call.name),
    }
}
// A successful tool invocation can still report a refused or uncertain Vessel
// command. Do not label admission as completed independent work.
fn vessel_outcome(call: &ToolCall, result: &Message) -> Option<&'static str> {
    if call.name != "vessel" {
        return None;
    }
    fn outcome(value: &serde_json::Value) -> Option<&'static str> {
        match value.get("status").and_then(serde_json::Value::as_str) {
            Some("outcome_unknown" | "unknown") => return Some("Unconfirmed"),
            Some("refused" | "rejected") => return Some("Refused"),
            Some("output_limit") => return Some("Output limited"),
            Some("accepted" | "queued") => return Some("Accepted"),
            _ => (),
        }
        // These are protocol-result wrappers, not arbitrary observed target state.
        for field in ["submit", "result", "start", "record"] {
            if let Some(result) = value.get(field).and_then(outcome) {
                return Some(result);
            }
        }
        None
    }
    serde_json::from_str(&result.content)
        .ok()
        .as_ref()
        .and_then(outcome)
}
fn failed(call: &ToolCall, result: &Message) -> bool {
    result.tool_outcome.as_ref().map_or_else(
        || {
            result.tool_success == Some(false)
                || vessel_outcome(call, result) == Some("Refused")
                || (call.name == "shell"
                    && result
                        .content
                        .lines()
                        .next()
                        .and_then(|line| line.strip_prefix("exit: "))
                        .is_some_and(|code| code != "0"))
        },
        |outcome| !outcome.success() || vessel_outcome(call, result) == Some("Refused"),
    )
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
        // Keep the gap outside the clickable accordion heading.
        super::layout::entry_gap(output, Key::Activity(id));
        let outcome = if failures > 0 {
            {
                let mut categories = std::collections::BTreeMap::<&str, usize>::new();
                for (call, result, _) in &entries {
                    if let Some(result) = result
                        && failed(call, result)
                    {
                        let label = result.tool_outcome.as_ref().map_or("Failed", |o| {
                            if o.success() {
                                vessel_outcome(call, result).unwrap_or("Failed")
                            } else {
                                o.label()
                            }
                        });
                        *categories.entry(label).or_default() += 1;
                    }
                }
                format!(
                    " · {}",
                    categories
                        .into_iter()
                        .map(|(label, count)| format!("{count} {label}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
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
                Some(r) if r.tool_outcome.as_ref().is_some_and(|o| !o.success()) => {
                    r.tool_outcome.as_ref().unwrap().label()
                }
                Some(r) if vessel_outcome(call, r).is_some() => vessel_outcome(call, r).unwrap(),
                Some(r) if failed(call, r) => "Failed",
                Some(r) if r.tool_success == Some(true) => "Done",
                Some(_) => "Received",
                None if active => "Working",
                None => "Unconfirmed",
            };
            let outcome = if let Some(outcome) = result.and_then(|r| r.tool_outcome.as_ref()) {
                let exit = match outcome.command {
                    Some(voyage_protocol::tool_result::CommandOutcome::Exited { code })
                        if code != 0 =>
                    {
                        format!(" (exit {code})")
                    }
                    _ => String::new(),
                };
                format!(
                    "{exit}{}",
                    if outcome.incomplete.is_some() && outcome.label() != "Output incomplete" {
                        " · output incomplete"
                    } else {
                        ""
                    }
                )
            } else if call.name == "shell" {
                result
                    .and_then(|r| r.content.lines().next())
                    .and_then(|l| l.strip_prefix("exit: "))
                    .filter(|c| *c != "0" && c.parse::<i32>().is_ok())
                    .map(|c| format!(" (exit {c})"))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let style = match status {
                "Failed" | "Execution error" | "Command failed" | "Command signalled" => {
                    crate::theme::Role::Failed.style()
                }
                "Working" => crate::theme::Role::Running.style(),
                "Done" => crate::theme::Role::Completed.style(),
                _ => crate::theme::Role::Muted.style(),
            };
            let text = Text::from(Line::from(vec![
                Span::styled(format!("{status}{outcome} · "), style),
                Span::raw(compact(
                    &description(call),
                    usize::from(width).saturating_mul(2).saturating_sub(24),
                )),
            ]));
            super::layout::entry_gap(output, Key::Activity(id));
            super::layout::rows(
                output,
                Key::Activity(id),
                crate::markdown::wrap_text(text, width.into()),
            );
            if let Some(result) = result.and_then(|message| message.tool_output.as_ref()) {
                for artifact in result.artifacts() {
                    note(
                        output,
                        Key::Activity(id),
                        format!(
                            "  Attachment · {} · {} · {} bytes",
                            compact(&artifact.name, 80),
                            compact(&artifact.mime_type, 80),
                            artifact.byte_size
                        ),
                        width,
                    );
                    note(
                        output,
                        Key::Activity(id),
                        format!(
                            "  Artifact {} · save with helm connect artifact",
                            artifact.id
                        ),
                        width,
                    );
                }
            }
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

#[cfg(test)]
mod reliability_tests {
    use super::*;
    use serde_json::json;
    use voyage_protocol::tool_result::{
        CommandOutcome, ExecutionOutcome, IncompleteReason, ToolOutcome,
    };
    #[test]
    fn typed_labels_render_without_raw_result_payloads() {
        let snapshot:Snapshot=serde_json::from_value(json!({"session_id":uuid::Uuid::new_v4(),"revision":1,"name":null,"model":"fixture","messages":[],"run":null})).unwrap();
        let call = ToolCall {
            id: "call".into(),
            name: "shell".into(),
            arguments: json!({"command":"fixture"}),
        };
        for (outcome, label) in [
            (
                ToolOutcome {
                    command: Some(CommandOutcome::Exited { code: 7 }),
                    ..Default::default()
                },
                "Command failed (exit 7)",
            ),
            (
                ToolOutcome {
                    execution: ExecutionOutcome::PolicyRefused,
                    ..Default::default()
                },
                "Refused",
            ),
            (
                ToolOutcome {
                    incomplete: Some(IncompleteReason::OutputLimit),
                    ..Default::default()
                },
                "Output incomplete",
            ),
            (
                ToolOutcome {
                    command: Some(CommandOutcome::Exited { code: 8 }),
                    incomplete: Some(IncompleteReason::CaptureLimit),
                    ..Default::default()
                },
                "Command failed (exit 8) · output incomplete",
            ),
            (
                ToolOutcome {
                    execution: ExecutionOutcome::Unknown,
                    ..Default::default()
                },
                "Unconfirmed",
            ),
        ] {
            let message = Message {
                role: "tool".into(),
                tool_call_id: Some("call".into()),
                content: "RAW SUBPROCESS DIAGNOSTIC".into(),
                tool_success: Some(false),
                tool_outcome: Some(outcome),
                ..Default::default()
            };
            let mut rows = vec![];
            let mut calls = vec![(0, &call)];
            flush(
                &mut rows,
                &mut calls,
                &[message],
                &snapshot,
                &State::default(),
                120,
            );
            let rendered = rows
                .iter()
                .map(|r| r.line.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(rendered.contains(label), "{rendered}");
            assert!(!rendered.contains("RAW SUBPROCESS"));
        }
    }
    #[test]
    fn legacy_and_remote_refusal_remain_visible() {
        let shell = ToolCall {
            id: "call".into(),
            name: "shell".into(),
            arguments: json!({}),
        };
        let old = Message {
            content: "exit: 2\nstdout:\n\nstderr:\n".into(),
            tool_success: Some(true),
            ..Default::default()
        };
        assert!(failed(&shell, &old));
        let vessel = ToolCall {
            id: "call".into(),
            name: "vessel".into(),
            arguments: json!({}),
        };
        let refused = Message {
            content: json!({"status":"refused"}).to_string(),
            tool_success: Some(true),
            tool_outcome: Some(ToolOutcome::default()),
            ..Default::default()
        };
        assert!(failed(&vessel, &refused));
        assert_eq!(vessel_outcome(&vessel, &refused), Some("Refused"));
        #[derive(serde::Deserialize)]
        struct OldMessage {
            content: String,
            tool_output: Option<voyage_protocol::tool_result::ToolOutput>,
        }
        let message = json!({"role":"tool","content":"unchanged","tool_output":{"content":[{"type":"text","text":"unchanged"}],"is_error":false},"tool_outcome":{"execution":"succeeded"}});
        let old: OldMessage = serde_json::from_value(message.clone()).unwrap();
        assert_eq!(old.content, "unchanged");
        assert!(old.tool_output.is_some());
        let new: Message = serde_json::from_value(message).unwrap();
        assert!(new.tool_outcome.unwrap().success());
    }
}
