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
    json!({"role":message.role,"content":message.content,"created_at":message.created_at,"operator_name":message.operator_name,"tool_calls":message.tool_calls,"tool_call_id":message.tool_call_id,"tool_success":message.tool_success,"steering":message.steering})
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
        json!({"role":message.role,"content":content,"created_at":message.created_at,"operator_name":message.operator_name,"content_truncated":truncated,"content_bytes":message.content.len(),"tool_calls":[],"tool_calls_omitted":!message.tool_calls.is_empty(),"tool_call_id":message.tool_call_id,"steering":message.steering,"tool_success":message.tool_success,"message_index":index,"projection_truncated":true,"complete_message":"message_chunk"}),
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

/// Only authored startup labels cross this surface, never underlying diagnostics.
pub(super) fn failure_summary(reason: Option<&str>) -> Option<&str> {
    match reason {
        Some(
            reason @ ("Runtime startup failed during runtime policy."
            | "Runtime startup failed during inference accounting."
            | "Runtime startup failed during subagent initialization."
            | "Runtime startup failed during tool initialization."
            | "Runtime startup failed during provider configuration."),
        ) => Some(reason),
        Some("local runtime construction or output failed") => Some("Runtime startup failed."),
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
        "run_id":s.run_id,"phase":s.phase,"message_start":s.message_start,"message_end":s.message_end,"started_at":s.started_at,"finished_at":s.finished_at
    })).collect()
}
