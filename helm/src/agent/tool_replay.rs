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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn call(id: &str, args: serde_json::Value) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "tool".into(),
            arguments: args,
        }
    }
    #[test]
    fn exact_semantic_duplicates_preserve_order_and_conflicts_are_nonmutating() {
        let mut calls = vec![
            call("a", json!({"x":1,"y":2})),
            call("b", json!({})),
            call("a", json!({"y":2,"x":1})),
        ];
        normalize(&mut calls).unwrap();
        assert_eq!(
            calls
                .iter()
                .map(|call| call.id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        for change_name in [false, true] {
            let mut bad = calls.clone();
            let mut duplicate = bad[0].clone();
            if change_name {
                duplicate.name = "other".into();
            } else {
                duplicate.arguments = json!({"x":3});
            }
            bad.push(duplicate);
            let before = serde_json::to_value(&bad).unwrap();
            assert!(normalize(&mut bad).is_err());
            assert_eq!(serde_json::to_value(&bad).unwrap(), before);
        }
        // Another model response starts a new identity scope, including same args.
        normalize(&mut calls).unwrap();
        assert_eq!(calls.len(), 2);
    }
    #[test]
    fn malformed_ids_never_leak_arguments_into_errors() {
        for id in ["".to_string(), " ".into(), "x\n".into(), "x".repeat(1025)] {
            let error = normalize(&mut vec![call(&id, json!({"secret":"canary"}))])
                .unwrap_err()
                .to_string();
            assert!(!error.contains("canary"));
        }
        normalize(&mut vec![call(&"x".repeat(1024), json!({}))]).unwrap();
    }

    #[test]
    fn interrupted_projection_preserves_results_and_scopes_reused_ids() {
        let mut assistant = Message::new(Role::Assistant, "batch");
        assistant.tool_calls = vec![call("a", json!({})), call("b", json!({}))];
        let mut messages = vec![
            assistant.clone(),
            Message::tool("b", "completed"),
            Message::new(Role::User, "continue"),
            assistant,
            Message::tool_result("a", "denied", false),
        ];
        project_interrupted_calls(&mut messages);
        assert_eq!(messages.len(), 7);
        assert_eq!(messages[1].content, "completed");
        assert_eq!(messages[2].tool_call_id.as_deref(), Some("a"));
        assert_eq!(messages[2].tool_success, Some(false));
        assert!(messages[2].content.contains("outcome is unknown"));
        assert_eq!(messages[3].role, Role::User);
        assert_eq!(messages[5].content, "denied");
        assert_eq!(messages[6].tool_call_id.as_deref(), Some("b"));
        let before = serde_json::to_value(&messages).unwrap();
        project_interrupted_calls(&mut messages);
        assert_eq!(serde_json::to_value(messages).unwrap(), before);
    }
}
