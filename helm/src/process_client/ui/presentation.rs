//! Bounded, readable presentation of operator metadata. Never dump wire payloads.
use super::safe;
use serde_json::Value;

pub(super) fn label(value: &str) -> String {
    let value = safe(value).replace('_', " ");
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

pub(super) fn fields(value: &Value) -> String {
    fn visit(value: &Value, depth: usize, lines: &mut Vec<String>) {
        if lines.len() >= 1500 {
            return;
        }
        let indent = "  ".repeat(depth.min(8));
        if depth > 8 {
            lines.push(format!("{indent}[Further details omitted]"));
            return;
        }
        match value {
            Value::Object(values) => {
                for (key, value) in values {
                    if matches!(
                        key.as_str(),
                        "session_id"
                            | "run_id"
                            | "command_id"
                            | "decision_id"
                            | "incarnation"
                            | "revision"
                            | "generation"
                            | "digest"
                            | "schema"
                            | "schema_version"
                            | "input_schema"
                            | "provider_state"
                            | "observation_cursor"
                    ) {
                        continue;
                    }
                    if lines.len() >= 1500 {
                        break;
                    }
                    if value.is_object() || value.is_array() {
                        lines.push(format!("{indent}{}", label(key)));
                        visit(value, depth + 1, lines);
                    } else {
                        lines.push(format!("{indent}{}: {}", label(key), scalar(value)));
                    }
                }
            }
            Value::Array(values) => {
                if values.is_empty() {
                    lines.push(format!("{indent}None"));
                }
                for (index, value) in values.iter().enumerate() {
                    if lines.len() >= 1500 {
                        break;
                    }
                    if value.is_object() || value.is_array() {
                        lines.push(format!("{indent}-- {} --", index + 1));
                        visit(value, depth + 1, lines);
                    } else {
                        lines.push(format!("{indent}- {}", scalar(value)));
                    }
                }
            }
            _ => lines.push(format!("{indent}{}", scalar(value))),
        }
    }
    let mut lines = Vec::new();
    visit(value, 0, &mut lines);
    if lines.len() >= 1500 {
        lines.push("[Display limit reached]".into());
    }
    lines.join("\n")
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => "Not set".into(),
        Value::Bool(value) => if *value { "Yes" } else { "No" }.into(),
        Value::String(value) => safe(value),
        Value::Number(value) => value.to_string(),
        _ => "Details available in panel".into(),
    }
}

pub(super) fn receipt(value: &Value) -> String {
    let status = match value["status"].as_str().unwrap_or("received") {
        "accepted" => "Request received. Waiting for the result.",
        "applied" => "Update confirmed.",
        "completed" => "Done.",
        "rejected" | "not_admitted" => "The request wasn't accepted. Your draft is saved.",
        "unknown" => "Not confirmed yet. Helm checks its status automatically.",
        "failed" => "The request couldn't be completed.",
        _ => "Status updated.",
    };
    let detail = value
        .get("detail")
        .or_else(|| value.get("reason"))
        .and_then(Value::as_str);
    detail.map_or_else(
        || status.into(),
        |detail| format!("{status} {}", notice(detail)),
    )
}

pub(super) const HELP: &str = "HELM / YOUR WORKSPACE\n  Ctrl+G /vessels   Manage Vessels (also clickable in header)\n\nMove around\n  Drag sidebar edge Resize voyage navigation (↔ divider)\n  Tab / Shift+Tab   Switch voyages\n  Up / Down         Navigate voyages with empty composer or sidebar focus\n  Right, Enter      Focus the sidebar ⋮ button and open Actions\n  F9                Open selected voyage Actions (also in narrow terminals)\n  Ctrl+N            Start a new voyage\n  F1                Help\n  F5                Switch between current and archived voyages\n  F8                Explore tools, tasks, models and permissions\n  Esc               Go back without losing your draft\n  PageUp / PageDown Scroll the current view\n  Ctrl+Home         Load earlier messages\n  Ctrl+End          Return to latest output\n  Ctrl+F            Find in loaded messages; Enter next, Esc close\n  Ctrl+T            Expand or collapse activity groups\n  Double-click call Expand or collapse saved tool details\n\nTalk to the assistant\n  /                 Open command menu\n  Up / Down         Select command or argument while menu is open\n  Tab               Complete selection; Enter executes\n  Esc               Dismiss command menu\n  Enter             Send your message\n  Alt+Enter         Add a line\n  Ctrl+V / Alt+V    Paste clipboard text or images into the composer\n  Alt+P             Show/hide owned attachment previews\n  Paste image path Attach a local PNG, JPEG or WebP (drag/drop paths work)\n  Backspace/Delete Remove the adjacent owned image marker\n  Esc               Cancel an in-progress clipboard read\n  Up / Down         Recall an earlier message while editing a draft\n  /cancel           Ask the assistant to stop\n\nQuestions and permissions\n  Requests open ready for your response.\n  Up / Down         Choose an answer or permission decision\n  Enter             Confirm selection; open or send a custom answer\n  Esc               Skip question / deny permission; leave custom editor\n  Left / Right      Previous / next request\n  PageUp / PageDown Read request details\n\nPrograms and passwords\n  F3                Find your running programs\n  Up / Down         Choose a program\n  Enter             Open its private terminal\n  Ctrl+]            Return to Helm\n  Type passwords only in the private terminal.\n  A password prompt may show no characters while you type.\n\nSee what is happening\n  /todos            Tasks\n  /subagents        Agents working on your request\n  /tools            Available tools\n  /policy           Permissions\n  /access           Change access (Read only / Ask first / Unrestricted)\n  /workflows        Saved workflows\n  /models           Available models\n  /host_resources   Machine capacity\n\nInference for the next turn\n  /model [ID]       Choose or type a model\n  /thinking [VALUE] Choose or type reasoning effort\n  /service [VALUE]  Choose or type service tier\n  inherit           Clear a thinking/service override\n  /service default  Disable catalog tier (API: standard)\n  Click composer controls for the same pickers.\n  Picker: type to search, Up/Down select, Enter apply, Esc cancel.\n  Model changes review existing overrides; never silently reset.\n  Unknown capabilities are not support; runtime/provider validates.\n  Pending changes keep text; Helm checks their outcome automatically.\n\nOrganize your work\n  /rename A name    Name this voyage\n  /branch A name    Continue in a separate voyage\n  /archive          Preserve history and stop this idle voyage after cleanup\n  /archived         Browse archived voyages (F5)\n  /voyages          Return to current voyages\n  /restore          Restart and restore the selected archived voyage\n  /export PATH      Save your conversation as Markdown\n\nLeave\n  Ctrl+C / Ctrl+Q   Close Helm\n  Active turns keep running when you leave. Finished voyages resume when you send again.";

/// Recency is derived from the durable turn completion, never observation time.
/// Failure/cancellation and outstanding cleanup retain their own visible state.
pub(super) fn voyage_state(snapshot: &super::state::Snapshot) -> &'static str {
    if snapshot.pending_cleanup_run.is_some()
        && snapshot.run.as_ref().is_none_or(|run| !run.active())
    {
        return match snapshot
            .cleanup
            .as_ref()
            .map(|cleanup| cleanup.phase.as_str())
        {
            Some("running") => "Finishing cleanup",
            Some("blocked") => "Cleanup needs attention",
            _ => "Cleanup pending",
        };
    }
    let Some(run) = &snapshot.run else {
        return "Ready";
    };
    if run.state == "completed"
        && snapshot
            .turns
            .iter()
            .rev()
            .find(|turn| turn.run_id == run.run_id)
            .and_then(|turn| turn.finished_at)
            .is_some_and(|finished| chrono::Utc::now() - finished >= chrono::Duration::hours(24))
    {
        "Settled"
    } else {
        run_state(&run.state)
    }
}

pub(super) fn run_state(state: &str) -> &'static str {
    match state {
        "accepted" => "Starting",
        "running" => "Working",
        "awaiting_decision" => "Waiting for you",
        "cancel_requested" => "Stopping",
        "completed" => "Finished",
        "failed" => "Needs attention",
        "cancelled" => "Stopped",
        "interrupted" => "Interrupted",
        _ => "Needs attention",
    }
}

pub(super) fn notice(value: &str) -> String {
    let lower = value.to_lowercase();
    if lower.contains("deadline elapsed") || lower.contains("timed out") {
        return "The connection is taking longer than expected. Your work may still be running."
            .into();
    }
    if lower.contains("identity mismatch") {
        return "This view changed before the action finished. Refresh and check its status."
            .into();
    }
    if lower.contains("revision conflict") {
        return "This voyage changed. Review the latest view before trying again.".into();
    }
    if lower.contains("incarnation") || lower.contains("stale control") {
        return "The voyage reconnected. Refresh before trying again.".into();
    }
    safe(value)
        .split_whitespace()
        .map(|word| {
            let token = word.trim_matches(|ch: char| !ch.is_ascii_hexdigit() && ch != '-');
            if uuid::Uuid::parse_str(token).is_ok() {
                "this request".to_owned()
            } else {
                word.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Wrap by display cells before measuring scroll offsets, preserving span styles.
pub(super) fn wrap(text: ratatui::text::Text<'_>, width: u16) -> ratatui::text::Text<'static> {
    use ratatui::text::{Line, Span, Text};
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    for line in text.lines {
        let mut spans = Vec::new();
        let mut used = 0;
        for span in line.spans {
            for grapheme in span.content.graphemes(true) {
                let size = grapheme.width();
                if grapheme == "\n" || (used > 0 && used + size > width) {
                    lines.push(Line::from(std::mem::take(&mut spans)).style(line.style));
                    used = 0;
                    if grapheme == "\n" {
                        continue;
                    }
                }
                spans.push(Span::styled(grapheme.to_owned(), span.style));
                used += size;
            }
        }
        lines.push(Line::from(spans).style(line.style));
    }
    Text::from(lines)
}

/// Older operator turns stored their arguments in canonical user text.
/// Present that known envelope as fields without changing the saved conversation.
pub(super) fn operator_message(content: &str) -> Option<String> {
    let (name, arguments) = content.strip_prefix("Operator tool ")?.split_once(": ")?;
    let value: Value = serde_json::from_str(arguments).ok()?;
    Some(match name {
        "process" if value["action"] == "start" => format!(
            "Open a terminal: {}",
            safe(value["name"].as_str().unwrap_or("Interactive program"))
        ),
        "questions" => safe(value["question"].as_str().unwrap_or("Ask a question")),
        "shell" => format!(
            "Run a command\n\n{}",
            safe(value["command"].as_str().unwrap_or_default())
        ),
        _ => format!("{}\n\n{}", label(name), fields(&value)),
    })
}

/// Stored operator results receive user-facing copy; canonical history is unchanged.
pub(super) fn structured_message(content: &str, operator: &str) -> Option<String> {
    if operator == "process"
        && content
            .strip_prefix("started PTY process ")
            .is_some_and(|id| uuid::Uuid::parse_str(id.trim()).is_ok())
    {
        return Some("Terminal opened. Press F3 to view it.".into());
    }
    let value: Value = serde_json::from_str(content).ok()?;
    match value["status"].as_str() {
        Some("selected" | "custom") if operator == "questions" && value["answer"].is_string() => {
            Some(format!(
                "Answer sent: {}",
                safe(value["answer"].as_str().unwrap_or_default())
            ))
        }
        Some("cancelled") if operator == "questions" => Some("Question skipped.".into()),
        Some("unavailable") if operator == "questions" => {
            Some("The question couldn't be shown.".into())
        }
        _ => (value.is_object() || value.is_array()).then(|| fields(&value)),
    }
}
