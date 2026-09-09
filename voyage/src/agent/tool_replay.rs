//! Identity validation is local to one completed provider response. Call IDs are
//! correlation labels, not cross-request, cross-run or durable effect identities.
use crate::{
    model::{Message, Role, ToolCall},
    provider::ProviderError,
};
use std::collections::{BTreeMap, BTreeSet};

/// Repair only the outgoing copy. Missing durable results say nothing about
/// whether an effect happened; never dispatch historical calls or invent success.
pub(super) fn project_interrupted_calls(messages: &mut Vec<Message>) {
    let mut projected = Vec::with_capacity(messages.len());
    let mut pending = Vec::<String>::new();
    for message in std::mem::take(messages) {
        if message.role != Role::Tool && message.role != Role::System {
            append_unknown_results(&mut projected, &mut pending);
        }
        if message.role == Role::Assistant {
            pending.extend(message.tool_calls.iter().map(|call| call.id.clone()));
        } else if message.role == Role::Tool {
            pending.retain(|id| Some(id) != message.tool_call_id.as_ref());
        }
        projected.push(message);
    }
    append_unknown_results(&mut projected, &mut pending);
    *messages = projected;
}

fn append_unknown_results(messages: &mut Vec<Message>, pending: &mut Vec<String>) {
    for id in pending.drain(..) {
        messages.push(Message::tool_result(id,
            "No durable tool result is available from the interrupted run. The outcome is unknown; the operation may have taken effect. Inspect current state before deciding whether to retry. Helm has not replayed this call.",
            false));
    }
}

pub(super) fn normalize(calls: &mut Vec<ToolCall>) -> Result<(), ProviderError> {
    let mut identities = BTreeMap::new();
    // Validate the whole batch before changing it or dispatching its first tool.
    for call in calls.iter() {
        if call.name.trim().is_empty() || call.name.chars().any(char::is_control) {
            return Err(ProviderError::InvalidResponse(
                "tool name is empty or contains control characters".into(),
            ));
        }
        if call.id.trim().is_empty()
            || call.id.len() > 1024
            || call.id.chars().any(char::is_control)
        {
            return Err(ProviderError::InvalidResponse(
                "tool call identity is empty, oversized or contains control characters".into(),
            ));
        }
        if let Some(previous) = identities.insert(call.id.as_str(), call)
            && (previous.name != call.name || previous.arguments != call.arguments)
        {
            return Err(ProviderError::InvalidResponse("conflicting tool calls share an identity within one provider response; no tools were dispatched".into()));
        }
    }
    let mut retained = BTreeSet::new();
    calls.retain(|call| retained.insert(call.id.clone()));
    Ok(())
}
