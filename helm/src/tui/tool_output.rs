//! Human-readable tool activity. Canonical arguments and results stay untouched.

use crate::model::{Message, ToolCall};
use ratatui::{
    style::{Color, Style},
    text::Line,
};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use super::text::display_safe;

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn title(call: &ToolCall) -> String {
    let args = &call.arguments;
    let detail = match call.name.as_str() {
        "shell" => string(args, "command").map(str::to_owned),
        "subagent" | "todo" | "process" => {
            let action = string(args, "action").unwrap_or("request");
            let target = ["name", "title", "command", "id"]
                .into_iter()
                .find_map(|key| string(args, key));
            Some(match target {
                Some(target) => format!("{action} · {target}"),
                None => match args.get("ids").and_then(Value::as_array) {
                    Some(ids) => format!("{action} · {} agents", ids.len()),
                    None => action.to_owned(),
                },
            })
        }
        _ => ["path", "pattern", "query"]
            .into_iter()
            .find_map(|key| string(args, key))
            .map(str::to_owned),
    };
    match detail {
        Some(detail) => format!("{} · {detail}", call.name),
        None => call.name.clone(),
    }
}

/// Wrap by display cells and graphemes, preserving indentation and empty lines.
/// Stop after one extra row for previews, even for very large single-line output.
fn rows(text: &str, width: usize, limit: usize) -> Vec<String> {
    let width = width.max(1);
    let safe = display_safe(&text.replace("\r\n", "\n")).replace('\t', "    ");
    let mut rows = Vec::new();
    for logical in safe.split('\n') {
        let mut row = String::new();
        let mut cells = 0;
        for grapheme in logical.graphemes(true) {
            let size = grapheme.width();
            if cells + size > width && !row.is_empty() {
                rows.push(std::mem::take(&mut row));
                if rows.len() >= limit {
                    return rows;
                }
                cells = 0;
            }
            if size > width {
                row.push('�');
                cells += 1;
            } else {
                row.push_str(grapheme);
                cells += size;
            }
        }
        rows.push(row);
        if rows.len() >= limit {
            break;
        }
    }
    rows
}

fn block(lines: &mut Vec<Line<'static>>, text: &str, width: usize, limit: usize, color: Color) {
    let indent = if width >= 4 { "  " } else { "" };
    let wrapped = rows(
        text,
        width.saturating_sub(indent.len()),
        limit.saturating_add(1),
    );
    let omitted = wrapped.len() > limit;
    lines.extend(
        wrapped
            .into_iter()
            .take(limit)
            .map(|row| Line::styled(format!("{indent}{row}"), Style::default().fg(color))),
    );
    if omitted {
        lines.extend(
            rows("… more · Ctrl+O details", width, usize::MAX)
                .into_iter()
                .map(|row| Line::styled(row, Style::default().fg(Color::DarkGray))),
        );
    }
}

struct ShellOutput<'a> {
    exit: &'a str,
    stdout: &'a str,
    stderr: Option<&'a str>,
}

fn shell_output(text: &str) -> Option<ShellOutput<'_>> {
    let (exit, body) = text.strip_prefix("exit: ")?.split_once("\nstdout:\n")?;
    if exit != "signal" && exit.parse::<i32>().is_err() {
        return None;
    }
    // Legacy shell results use a text envelope. Keep full source available in
    // details; use the last separator so a marker in stdout usually stays there.
    let (stdout, stderr) = match body.rsplit_once("\nstderr:\n") {
        Some((stdout, stderr)) => (stdout, Some(stderr)),
        None => (body, None), // Output may have hit the runtime's byte limit.
    };
    Some(ShellOutput {
        exit,
        stdout,
        stderr,
    })
}

fn child_failed(value: &Value) -> bool {
    string(value, "status")
        .is_some_and(|s| matches!(s, "failed" | "cancelled" | "timed_out" | "interrupted"))
        || value
            .get("results")
            .and_then(Value::as_array)
            .is_some_and(|results| results.iter().any(child_failed))
}

fn summary(value: &Value) -> Option<String> {
    if let Some(items) = value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
    {
        let mut output = vec![format!("{} items", items.len())];
        output.extend(
            items
                .iter()
                .take(6)
                .map(|item| summary(item).unwrap_or_else(|| pretty(item))),
        );
        if items.len() > 6 {
            output.push("… more · Ctrl+O details".into());
        }
        return Some(output.join("\n"));
    }
    if let Some(results) = value.get("results").and_then(Value::as_array) {
        let failed = results.iter().filter(|result| child_failed(result)).count();
        let mut output = vec![format!(
            "{} agents returned · {failed} unsuccessful",
            results.len()
        )];
        // Put failures first so successful results cannot bury an error.
        for (index, result) in results
            .iter()
            .enumerate()
            .filter(|(_, r)| child_failed(r))
            .chain(results.iter().enumerate().filter(|(_, r)| !child_failed(r)))
            .take(6)
        {
            let identity = value
                .get("ids")
                .and_then(Value::as_array)
                .and_then(|ids| ids.get(index))
                .and_then(Value::as_str)
                .map(|id| id.chars().take(8).collect::<String>())
                .unwrap_or_else(|| format!("agent {}", index + 1));
            output.push(format!(
                "{identity} · {}",
                summary(result).unwrap_or_else(|| pretty(result))
            ));
        }
        if results.len() > 6 {
            output.push("… more · Ctrl+O details".into());
        }
        return Some(output.join("\n"));
    }
    let mut parts = Vec::new();
    for key in ["title", "name", "status", "error"] {
        if let Some(text) = string(value, key) {
            parts.push(text.to_owned());
        }
    }
    if let Some(blockers) = value.get("blockers").and_then(Value::as_array) {
        for blocker in blockers.iter().filter_map(Value::as_str) {
            parts.push(format!("Blocked: {blocker}"));
        }
    }
    if value.get("cancel_requested").and_then(Value::as_bool) == Some(true) {
        parts.push("Cancellation requested".into());
    }
    if value.get("queued").and_then(Value::as_bool) == Some(true) {
        parts.push("Queued".into());
    }
    if let Some(result) = value.get("result")
        && let Some(text) = result.as_str().or_else(|| string(result, "summary"))
    {
        parts.push(text.to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub(super) fn render(
    call: &ToolCall,
    result: Option<&Message>,
    running: bool,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let parsed = result.and_then(|r| serde_json::from_str::<Value>(&r.content).ok());
    let shell = (call.name == "shell")
        .then(|| result.and_then(|r| shell_output(&r.content)))
        .flatten();
    let failed = result.is_some_and(|r| r.tool_success == Some(false))
        || shell.as_ref().is_some_and(|s| s.exit != "0")
        || (call.name == "subagent" && parsed.as_ref().is_some_and(child_failed));
    let (symbol, state, color) = if failed {
        ("✗", "failed", Color::Red)
    } else if result.is_some() {
        ("✓", "", Color::Green)
    } else if running {
        ("▶", "running", Color::Yellow)
    } else {
        ("■", "interrupted / no result", Color::DarkGray)
    };
    let mut lines = Vec::new();
    let heading = if state.is_empty() {
        format!("{symbol} {}", title(call))
    } else {
        format!("{symbol} {} · {state}", title(call))
    };
    // Reserve a distinct status row: a long command must never hide failure.
    block(
        &mut lines,
        &heading,
        width,
        if expanded { usize::MAX } else { 3 },
        color,
    );
    if failed {
        block(&mut lines, "Failed", width, usize::MAX, Color::Red);
    }
    if expanded {
        block(&mut lines, "Arguments", width, usize::MAX, Color::DarkGray);
        block(
            &mut lines,
            &pretty(&call.arguments),
            width,
            usize::MAX,
            Color::Gray,
        );
        if let Some(result) = result {
            block(&mut lines, "Result", width, usize::MAX, Color::DarkGray);
            let content = parsed
                .as_ref()
                .map(pretty)
                .unwrap_or_else(|| result.content.clone());
            block(
                &mut lines,
                &content,
                width,
                usize::MAX,
                if failed { Color::Red } else { Color::Gray },
            );
        }
    } else if let Some(shell) = shell {
        block(
            &mut lines,
            &format!("exit {}", shell.exit),
            width,
            usize::MAX,
            color,
        );
        // Errors get their own preview budget and are never hidden by stdout.
        if let Some(stderr) = shell.stderr.filter(|s| !s.trim().is_empty()) {
            block(&mut lines, "stderr", width, usize::MAX, Color::Yellow);
            block(
                &mut lines,
                stderr.trim_end_matches('\n'),
                width,
                4,
                Color::Yellow,
            );
        }
        if !shell.stdout.trim().is_empty() {
            block(&mut lines, "stdout", width, usize::MAX, Color::DarkGray);
            block(
                &mut lines,
                shell.stdout.trim_end_matches('\n'),
                width,
                5,
                Color::Gray,
            );
        }
    } else if let Some(result) = result {
        let content = if matches!(call.name.as_str(), "todo" | "subagent") {
            parsed
                .as_ref()
                .and_then(summary)
                .unwrap_or_else(|| result.content.clone())
        } else {
            parsed
                .as_ref()
                .map(pretty)
                .unwrap_or_else(|| result.content.clone())
        };
        block(
            &mut lines,
            &content,
            width,
            6,
            if failed { Color::Red } else { Color::Gray },
        );
    }
    lines
}
