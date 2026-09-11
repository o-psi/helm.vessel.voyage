//! Bounded observations preserve an explicit route to the complete public message.
use crate::model::Message;
use anyhow::Result;
use serde_json::{Value, json};
const MESSAGE_BYTES: usize = 32768;
const PAGE_BYTES: usize = 512 * 1024;

pub(super) fn text_prefix(text: &str, limit: usize) -> (&str, bool) {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], end < text.len())
}
pub(super) fn full(message: &Message) -> Value {
    json!({"coordination":message.coordination,"role":message.role,"content":message.content,"parts":message.parts,"tool_output":message.tool_output,"created_at":message.created_at,"operator_name":message.operator_name,"tool_calls":message.tool_calls,"tool_call_id":message.tool_call_id,"tool_outcome":message.tool_outcome,"tool_success":message.tool_success,"steering":message.steering})
}
fn bounded(message: &Message, index: usize) -> Result<Value> {
    let mut value = full(message);
    if serde_json::to_vec(&value)?.len() <= MESSAGE_BYTES {
        value["message_index"] = json!(index);
        value["projection_truncated"] = json!(false);
        return Ok(value);
    }
    let (content, truncated) = text_prefix(&message.content, 4096);
    Ok(
        json!({"coordination":message.coordination,"role":message.role,"content":content,"created_at":message.created_at,"operator_name":message.operator_name,"content_truncated":truncated,"content_bytes":message.content.len(),"tool_calls":[],"tool_calls_omitted":!message.tool_calls.is_empty(),"tool_call_id":message.tool_call_id,"steering":message.steering,"tool_outcome":message.tool_outcome,"tool_success":message.tool_success,"message_index":index,"projection_truncated":true,"complete_message":"message_chunk"}),
    )
}
pub(super) fn page(messages: &[Message], offset: usize, limit: usize) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    let mut bytes = 0;
    for (index, message) in messages.iter().enumerate().skip(offset).take(limit) {
        let value = bounded(message, index)?;
        let length = serde_json::to_vec(&value)?.len();
        if bytes + length > PAGE_BYTES && !output.is_empty() {
            break;
        }
        bytes += length;
        output.push(value);
    }
    Ok(output)
}
pub(super) fn recent(messages: &[Message]) -> Result<(Vec<Value>, usize)> {
    let mut output = Vec::new();
    let mut bytes = 0;
    let mut offset = messages.len();
    for (index, message) in messages.iter().enumerate().rev().take(128) {
        let value = bounded(message, index)?;
        let length = serde_json::to_vec(&value)?.len();
        if bytes + length > PAGE_BYTES && !output.is_empty() {
            break;
        }
        bytes += length;
        offset = index;
        output.push(value);
    }
    output.reverse();
    Ok((output, offset))
}

/// Only authored failure labels cross this surface, never underlying diagnostics.
pub(super) fn failure_summary(reason: Option<&str>) -> Option<&str> {
    match reason {
        Some(
            reason @
            ("Host execution capacity exhausted. Wait for active voyages to finish or reconcile stopped owners' cleanup reservations."
            | "Host resource cleanup tracking could not be initialized. Check host resource accounting on the executing machine."
            | "Host resource accounting is busy. Retry this turn."
            | "Host execution capacity could not be reserved. Check host resource accounting on the executing machine."
            | "Runtime startup failed during runtime policy."
            | "Runtime startup failed during inference accounting."
            | "Runtime startup failed during subagent initialization."
            | "Runtime startup failed during tool initialization."
            | "Runtime startup failed during provider configuration."
            | "Operator action failed during setup."
            | "Operator tool failed."
            | "Operator action failed during finalization."
            | "Provider authentication failed. Check credentials on the executing machine."
            | "Provider rate limit prevented completion."
            | "Provider temporarily unavailable."
            | "Provider request timed out."
            | "Provider rejected the request."
            | "Provider returned an invalid or incomplete response."
            | "Provider stopped before completing its response."
            | "Local inference admission or accounting failed."
            | "Completion records could not be verified."
            | "Configured context limit prevented the request."
            | "Provider context exhausted after safe compaction. Full history is retained; narrow the task or select a larger-context model."
            | "Execution policy prevented the run."
            | "Workspace instructions could not be loaded."
            | "Provider usage accounting overflowed."),
        ) => Some(reason),
        Some("durable checkpoint failed") => Some("Durable checkpoint failed."),
        Some("local runtime construction or output failed") => Some("Runtime startup failed."),
        Some(crate::provider::USAGE_LIMIT_MESSAGE) => Some(crate::provider::USAGE_LIMIT_MESSAGE),
        Some("provider or runtime failed") => Some("Provider or runtime failed."),
        _ => None,
    }
}

/// Run-owned ranges reconcile the provisional stream with saved assistant text.
/// An ambiguous older history receives no speculative preview.
pub(super) fn run(
    session: &crate::session::Session,
    run: &crate::attachment::journal::RunRecord,
) -> Value {
    let summary = session.run_summaries.iter().find(|s| s.run_id == run.id);
    let committed = summary
        .and_then(|s| s.message_start)
        .filter(|start| *start <= session.messages.len())
        .map(|start| {
            session.messages[start..]
                .iter()
                .filter(|m| m.role == crate::model::Role::Assistant && m.operator_name.is_none())
                .map(|m| m.content.as_str())
                .collect::<String>()
        });
    let offset = committed.as_ref().and_then(|text| {
        if run.partial_text.starts_with(text) {
            Some(text.len())
        } else if text.starts_with(&run.partial_text) {
            Some(run.partial_text.len())
        } else {
            None
        }
    });
    let live = offset.map(|start| &run.partial_text[start..]).unwrap_or("");
    let (live, live_truncated) = text_prefix(live, 65536);
    let (partial, truncated) = text_prefix(&run.partial_text, 65536);
    json!({"run_id":run.id,"state":run.state,"failure_summary":failure_summary(run.terminal_reason.as_deref()),
        "partial_text":partial,"partial_text_truncated":truncated,"partial_text_bytes":run.partial_text.len(),
        "live_text":live,"live_text_truncated":live_truncated,"live_text_offset":offset,
        "stream_reconciled":offset.is_some(),"message_start":summary.and_then(|s| s.message_start)})
}
pub(super) fn turns(session: &crate::session::Session) -> Vec<Value> {
    session.run_summaries.iter().rev().take(1024).rev().map(|s| json!({
        "run_id":s.run_id,"phase":s.phase,"failure_summary":failure_summary(s.detail.as_deref()),"message_start":s.message_start,"message_end":s.message_end,"started_at":s.started_at,"finished_at":s.finished_at
    })).collect()
}

#[cfg(test)]
mod reliability_tests {
    use super::*;
    #[test]
    fn typed_outcome_survives_full_and_bounded_projection() {
        let mut message = Message::tool_result("call", "x".repeat(40000), false);
        message.tool_outcome = Some(voyage_protocol::tool_result::ToolOutcome {
            command: Some(voyage_protocol::tool_result::CommandOutcome::Exited { code: 7 }),
            incomplete: Some(voyage_protocol::tool_result::IncompleteReason::CaptureLimit),
            ..Default::default()
        });
        let a = full(&message);
        let b = bounded(&message, 3).unwrap();
        assert_eq!(a["tool_outcome"], b["tool_outcome"]);
        assert_eq!(b["projection_truncated"], true);
        assert_eq!(b["tool_call_id"], "call");
    }
}
