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
    if value["status"] == "applied"
        && value["canonical_preserved"] == true
        && let Some(count) = value["compacted_messages"].as_u64()
    {
        if count == 0 {
            return "No further safe working-context reduction was needed. Full history is retained.".into();
        }
        return format!(
            "Working context reduced for {count} messages. Full history is retained; subsequent requests use the saved projection."
        );
    }
    let status = match value["status"].as_str().unwrap_or("received") {
        "accepted" => "Request received.",
        "applied" => "Update confirmed.",
        "completed" => "Done.",
        "rejected" | "not_admitted" => {
            "The request wasn't accepted. Your draft is retained in memory."
        }
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

pub(super) const HELP: &str = "HELM / YOUR WORKSPACE\n  Ctrl+P            Open the Helm menu (also clickable)\n  Ctrl+G /vessels   Manage Vessels (also clickable in header)\n\nMove around\n  F2                Find voyages and drafts by name (all widths)\n  Drag sidebar edge Resize voyage navigation (↔ divider)\n  Tab / Shift+Tab   Switch voyages\n  Up / Down         Navigate voyages with empty composer or sidebar focus\n  Right, Enter      Focus the sidebar ⋮ button and open Actions\n  F9                Open selected voyage Actions (also in narrow terminals)\n  Ctrl+N            Start a new voyage\n  F1                Help\n  F5                Switch between current and archived voyages\n  F6 /browser       Open the local shared-browser companion\n  F8                Search actions, scope and availability\n  Esc               Go back without losing your draft\n  PageUp / PageDown Scroll the current view\n  Ctrl+Home         Load earlier messages\n  Ctrl+End          Return to latest output\n  Ctrl+F            Find loaded display only; Enter next / Shift+Enter previous\n  Ctrl+Up/Down      Previous/next saved user message\n  Ctrl+Shift+Up/Down Move to tool/reasoning disclosure\n  Ctrl+Space        Toggle the disclosure at the reading position\n  Ctrl+T            Expand or collapse activity groups\n  Double-click call Expand or collapse saved tool details\n\nTalk to the assistant\n  /                 Open command menu\n  Up / Down         Select command or argument while menu is open\n  Tab               Complete selection; Enter executes\n  Esc               Dismiss command menu\n  Enter             Submit idle / Steer active run\n  Alt+Enter         Add a line\n  Ctrl+V / Alt+V    Paste clipboard text or images into the composer\n  Alt+P             Show/hide owned attachment previews\n  Paste image path Attach a local PNG, JPEG or WebP (drag/drop paths work)\n  Backspace/Delete Remove the adjacent owned image marker\n  Esc               Cancel an in-progress clipboard read\n  Up / Down         Recall an earlier message while editing a draft\n  /stop /cancel     Review Stop for the exact active run\n\nQuestions and permissions\n  Requests open ready for your response.\n  Up / Down         Choose an answer or permission decision\n  Enter             Confirm selection; open or send a custom answer\n  Esc               Skip question / deny permission; leave custom editor\n  Left / Right      Previous / next request\n  PageUp / PageDown Read request details\n\nLocal shared browser\n  /browser          Open the local companion for the selected voyage\n  /browser private  Fence agent input and capture locally\n  /browser takeover Take human control\n  /browser close    Close owned browser; Voyage continues\n  Return to agent requires explicit local companion review.\n\nPrograms and passwords\n  F3                Find your running programs\n  Up / Down         Choose a program\n  Enter             Open its private terminal\n  Ctrl+]            Return to Helm\n  Private terminal entry is strongly recommended for passwords.\n  A password prompt may show no characters while you type.\n\nSee what is happening\n  /inspect /diff    Read-only executing-host workspace inspection\n  /copy             Review canonical response clipboard disclosure\n  Alt+M             Focus next visible answer actions\n  Alt+C / Alt+D     Copy selected answer / inspect workspace changes\n  /todos            Tasks\n  /subagents        Agents working on your request\n  /tools            Available tools\n  /policy           Permissions\n  /access           Change access (Read only / Ask first / Unrestricted)\n  /workflows        Saved workflows\n  /models           Available models\n  /host_resources   Machine cleanup records\n\nInference for the next turn\n  /account          Choose account / private device sign-in\n  /model [ID]       Choose or type a model\n  /thinking [VALUE] Choose or type reasoning effort\n  /service [VALUE]  Choose or type service tier\n  inherit           Clear a thinking/service override\n  /service default  Disable catalog tier (API: standard)\n  Click Model to choose directly; rows preview, Use model applies. /preferences opens it too.\n  Picker: type to search, Up/Down select, Enter apply, Esc cancel.\n  Model changes review existing overrides; never silently reset.\n  Unknown capabilities are not support; runtime/provider validates.\n  Pending changes keep text; Helm checks their outcome automatically.\n\nOrganize your work\n  /rename A name    Name this voyage\n  /branch A name    Continue in a separate voyage\n  /archive          Preserve history and stop this idle voyage after cleanup\n  /archived         Browse archived voyages (F5)\n  /voyages          Return to current voyages\n  /restore          Restart and restore the selected archived voyage\n  /export PATH      Save your conversation as Markdown\n\nLeave\n  Ctrl+C / Ctrl+Q   Detach Helm; voyages continue\n  Active turns keep running when you leave. Finished voyages resume when you send again.";

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
    if run.state == "running"
        && let Some(attempt) = run.provider_attempts.last()
    {
        use voyage_protocol::provider_attempt::RetryDecision;
        match attempt.decision {
            RetryDecision::RetryScheduled => return "Reconnecting to provider",
            RetryDecision::ContinuationScheduled => return "Continuing interrupted response",
            RetryDecision::RecoveryInterrupted => return "Recovery needs attention",
            _ => {}
        }
    }
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

#[cfg(test)]
mod compaction_receipt_tests {
    use super::*;
    #[test]
    fn manual_compaction_displays_verified_count_and_noop_without_claiming_legacy_preservation() {
        let applied = serde_json::json!({"status":"applied","canonical_preserved":true,"compacted_messages":12,"removed_messages":0});
        let text = receipt(&applied);
        assert!(text.contains("12 messages") && text.contains("Full history is retained"));
        assert!(receipt(&serde_json::json!({"status":"applied","canonical_preserved":true,"compacted_messages":0})).contains("No further safe"));
        assert!(
            !receipt(&serde_json::json!({"status":"applied","removed_messages":12}))
                .contains("history is retained")
        );
    }
}

#[cfg(test)]
mod recovery_status_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn active_recovery_status_tracks_latest_attempt_and_clears_at_run_stop() {
        let mut snapshot: super::super::state::Snapshot = serde_json::from_value(json!({
            "session_id":uuid::Uuid::new_v4(),"revision":1,"model":"fixture","messages":[],
            "run":{"run_id":uuid::Uuid::new_v4(),"state":"running","provider_attempts":[{
                "request_id":uuid::Uuid::new_v4(),"attempt_id":uuid::Uuid::new_v4(),
                "provider":"fixture","model":"fixture","attempt":1,"limit":8,
                "started_at_ms":1,"duration_ms":1,"phase":"backoff","category":"connection",
                "http_status":null,"text_observed":false,"tool_fragment_observed":false,
                "retry_delay_ms":1000,"decision":"retry_scheduled"
            }]}
        }))
        .unwrap();
        assert_eq!(voyage_state(&snapshot), "Reconnecting to provider");
        use voyage_protocol::provider_attempt::RetryDecision;
        snapshot.run.as_mut().unwrap().provider_attempts[0].decision =
            RetryDecision::ContinuationScheduled;
        assert_eq!(voyage_state(&snapshot), "Continuing interrupted response");
        for (state, label) in [
            ("failed", "Needs attention"),
            ("interrupted", "Interrupted"),
            ("cancel_requested", "Stopping"),
            ("cancelled", "Stopped"),
            ("completed", "Finished"),
            ("awaiting_decision", "Waiting for you"),
        ] {
            snapshot.run.as_mut().unwrap().state = state.into();
            assert_eq!(voyage_state(&snapshot), label);
        }
        snapshot.run.as_mut().unwrap().state = "running".into();
        let mut next = snapshot.run.as_ref().unwrap().provider_attempts[0].clone();
        next.attempt_id = uuid::Uuid::new_v4();
        next.attempt = 2;
        next.decision = RetryDecision::InFlight;
        snapshot.run.as_mut().unwrap().provider_attempts.push(next);
        assert_eq!(voyage_state(&snapshot), "Working");
        snapshot.run.as_mut().unwrap().provider_attempts[1].decision =
            RetryDecision::RecoveryInterrupted;
        assert_eq!(voyage_state(&snapshot), "Recovery needs attention");
        snapshot.run.as_mut().unwrap().provider_attempts.clear();
        assert_eq!(voyage_state(&snapshot), "Working");
        snapshot.run = None;
        assert_eq!(voyage_state(&snapshot), "Ready");
    }
}

/// Composer intent follows observed runtime admission, never an invented follow-up queue.
pub(super) fn composer_intent(view: &super::state::View) -> &'static str {
    if view.pending.is_some() {
        return "Pending · checking original identity · draft retained";
    }
    let Some(snapshot) = &view.snapshot else {
        return "Waiting for voyage · draft retained";
    };
    if snapshot.recovery_pending {
        return "Recovery pending · no replay · draft retained";
    }
    if let Some(run) = snapshot.run.as_ref().filter(|run| run.active()) {
        if run.state == "cancel_requested" {
            return "Stopping · wait for cleanup · draft retained";
        }
        if !view.images.is_empty() {
            return "Images cannot steer · wait then Submit explicitly · no after-run queue";
        }
        return "Enter Steer active run · admitted before delivered · no after-run queue";
    }
    if snapshot.pending_cleanup_run.is_some() {
        return "Cleanup pending · Submit unavailable · draft retained";
    }
    "Enter Submit new turn"
}

pub(super) fn steering_status(status: &str) -> &'static str {
    match status {
        "queued" => "Steering admitted · awaiting delivery in this run",
        "applied" => "Steering delivered to this run",
        "not_applied" => "Steering not delivered · no automatic follow-up",
        _ => "Steering delivery unknown · checking original identity",
    }
}

#[cfg(test)]
mod composer_tests {
    use super::*;

    #[test]
    fn steering_admission_is_not_delivery_or_after_run_queue() {
        assert_eq!(
            steering_status("queued"),
            "Steering admitted · awaiting delivery in this run"
        );
        assert_eq!(steering_status("applied"), "Steering delivered to this run");
        assert_eq!(
            steering_status("not_applied"),
            "Steering not delivered · no automatic follow-up"
        );
        assert!(steering_status("future_wire_state").contains("unknown"));
    }
}

#[cfg(test)]
#[path = "presentation_coverage_tests.rs"]
mod coverage_tests;
