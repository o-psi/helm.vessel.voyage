//! Identity validation is local to one completed provider response. Call IDs are
//! correlation labels, not cross-request, cross-run or durable effect identities.
use crate::{model::ToolCall, provider::ProviderError};
use std::collections::{BTreeMap, BTreeSet};

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
}
