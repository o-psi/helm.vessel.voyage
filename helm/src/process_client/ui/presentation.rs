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
    let status = value["status"].as_str().unwrap_or("received");
    let detail = value
        .get("detail")
        .or_else(|| value.get("reason"))
        .and_then(Value::as_str);
    match detail {
        Some(detail) => format!("{}: {}", label(status), safe(detail)),
        None => format!("Command {}", label(status).to_lowercase()),
    }
}

pub(super) const HELP: &str = "HELM / KEYBOARD GUIDE\n\nNavigate\n  Tab / Shift+Tab   Switch voyages\n  Ctrl+N            Create a voyage\n  F1                Open this guide\n  Esc               Return to conversation\n  PageUp / PageDown Scroll the current view\n\nTerminals and passwords\n  F3                Open the terminal browser\n  Up / Down         Select a named terminal\n  Enter             Attach to the selected terminal\n  Ctrl+]            Leave private terminal and return to Helm\n  Only type passwords after entering the private terminal.\n  A running terminal may be busy or waiting; Helm does not guess.\n\nApprovals and questions\n  F2                Focus interactions\n  F6 / F7           Previous / next request\n  Ctrl+A / Ctrl+D   Approve once / deny approval\n  Up / Down         Select an answer in a focused question\n  Enter             Send focused answer\n  Ctrl+D            Skip focused question\n\nConversation\n  Enter             Send message or steer active work\n  Alt+Enter         New line\n  Up / Down         Recall prompts\n  /cancel           Request cancellation\n  /receipt          Resolve uncertain delivery\n\nInspect\n  /tools  /policy  /todos  /subagents\n  /workflows  /models  /host_resources\n  /terminals        Open terminal browser\n  /conversation     Return to chat\n\nManage\n  /new [absolute-workspace]  /use UUID\n  /rename NAME  /model NAME  /configure /host/path\n  /branch [name]  /archive  /restore\n  /compact N  /clear UUID  /delete UUID\n  /export PATH      Save conversation as Markdown\n  /tool NAME JSON   Advanced operator tool command\n  /approve UUID  /deny UUID  /answer UUID text\n\nLeave\n  Ctrl+C / Ctrl+Q / /quit  Detach Helm\n  Disconnecting Helm does not cancel voyage work.";

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
    Some(format!(
        "Operator action: {}\n\n{}",
        safe(name),
        fields(&value)
    ))
}
