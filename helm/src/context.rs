//! Conservative request budgeting. Reduction affects only a request projection.
use crate::model::{ModelRequest, Role};
use thiserror::Error;

pub const DEFAULT_CONTEXT_WINDOW: usize = 65_536;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error(
    "request context does not fit: estimated {estimated} tokens including output reserve, limit {limit}; reduce the prompt/tool output or configure the model's context_window"
)]
pub struct ContextError {
    pub estimated: usize,
    pub limit: usize,
}

#[derive(Clone, Debug)]
pub struct ContextReport {
    pub estimated: usize,
    pub limit: usize,
    pub omitted_messages: usize,
}

/// One token per serialized UTF-8 byte, plus explicit per-item/framing headroom.
/// Includes replay metadata, tool schemas and the response allowance. This is a
/// conservative estimator, not provider-reported usage or a universal tokenizer.
pub fn estimate(request: &ModelRequest) -> usize {
    serialized_size(request)
        .saturating_add(4096)
        .saturating_add(request.messages.len().saturating_mul(256))
        .saturating_add(request.tools.len().saturating_mul(256))
        .saturating_add(request.max_tokens.unwrap_or(8192) as usize)
}

fn serialized_size(value: &impl serde::Serialize) -> usize {
    // Count without allocating a second copy of potentially large tool results.
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    if serde_json::to_writer(&mut counter, value).is_ok() {
        counter.0
    } else {
        usize::MAX
    }
}

/// Preserve system guidance and the latest root user turn, including steering and
/// complete tool groups. Find the smallest removable prefix in one linear scan;
/// repeatedly serializing the remaining history would be quadratic.
pub fn preflight(request: &mut ModelRequest, limit: usize) -> Result<ContextReport, ContextError> {
    let estimated = estimate(request);
    if estimated <= limit && limit > 0 {
        return Ok(ContextReport {
            estimated,
            limit,
            omitted_messages: 0,
        });
    }
    if limit == 0 || estimated == usize::MAX {
        return Err(ContextError { estimated, limit });
    }
    let mut omitted = 0;
    let mut removed_cost = 0usize;
    let mut seen_root = false;
    let mut selected = None;
    let mut smallest = estimated;
    for (index, message) in request.messages.iter().enumerate() {
        if message.role == Role::User && message.steering.is_none() {
            if seen_root {
                let marker = crate::model::Message::new(
                    Role::System,
                    format!(
                        "[Context projection: {omitted} older messages omitted; canonical transcript retained locally.]"
                    ),
                );
                // The retained list is nonempty, so each removed/added item also
                // removes/adds exactly one JSON comma and 256 framing tokens.
                let projected = estimated
                    .saturating_sub(removed_cost)
                    .saturating_add(serialized_size(&marker))
                    .saturating_add(257);
                smallest = smallest.min(projected);
                if projected <= limit {
                    selected = Some((index, omitted, marker));
                    break;
                }
            }
            seen_root = true;
        }
        if message.role != Role::System {
            removed_cost = removed_cost
                .saturating_add(serialized_size(message))
                .saturating_add(257);
            omitted += 1;
        }
    }
    let Some((end, omitted, marker)) = selected else {
        return Err(ContextError {
            estimated: smallest,
            limit,
        });
    };
    let mut index = 0;
    request.messages.retain(|message| {
        let keep = index >= end || message.role == Role::System;
        index += 1;
        keep
    });
    request.messages.insert(0, marker);
    let estimated = estimate(request);
    if estimated > limit {
        return Err(ContextError { estimated, limit });
    }
    Ok(ContextReport {
        estimated,
        limit,
        omitted_messages: omitted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Message, ToolCall, ToolDefinition};

    fn request() -> ModelRequest {
        ModelRequest {
            model: "synthetic".into(),
            messages: vec![
                Message::new(Role::System, "policy"),
                Message::new(Role::User, "latest"),
            ],
            tools: vec![],
            temperature: None,
            max_tokens: Some(100),
        }
    }

    #[test]
    fn exact_boundary_and_zero_fail_closed() {
        let mut r = request();
        let size = estimate(&r);
        assert_eq!(preflight(&mut r, size).unwrap().estimated, size);
        assert_eq!(preflight(&mut r, size - 1).unwrap_err().estimated, size);
        assert!(preflight(&mut r, 0).is_err());
    }

    #[test]
    fn accounts_for_schemas_replay_unicode_and_output() {
        let mut r = request();
        let initial = estimate(&r);
        r.max_tokens = Some(200);
        assert!(estimate(&r) >= initial + 100);
        let initial = estimate(&r);
        r.messages[1].content.push_str(&"🦀\u{1b}".repeat(100));
        r.messages[1].provider_state = Some(serde_json::json!({"replay": "x".repeat(500)}));
        r.tools.push(ToolDefinition {
            name: "tool".into(),
            description: "description".repeat(50),
            input_schema: serde_json::json!({"type": "object"}),
        });
        assert!(estimate(&r) > initial + 1500);
        assert!(preflight(&mut r, initial).is_err());
    }

    #[test]
    fn reduces_whole_turns_repeatedly_and_preserves_canonical_source() {
        let mut r = request();
        for _ in 0..4 {
            r.messages
                .insert(1, Message::new(Role::Assistant, "old reply".repeat(400)));
            r.messages
                .insert(1, Message::new(Role::User, "old prompt".repeat(400)));
        }
        let canonical = r.messages.clone();
        let report = preflight(&mut r, 6500).unwrap();
        assert_eq!(report.omitted_messages, 8);
        assert!(report.estimated <= 6500);
        assert_eq!(r.messages[1].content, "policy");
        assert_eq!(r.messages.last().unwrap().content, "latest");
        assert_eq!(canonical.len(), 10);
        assert!(!canonical[0].content.contains("projection"));
        assert_eq!(preflight(&mut r, 6500).unwrap().omitted_messages, 0);
    }

    #[test]
    fn active_tool_group_is_indivisible_and_old_groups_are_removed_together() {
        let mut r = request();
        let mut assistant = Message::new(Role::Assistant, "");
        assistant.tool_calls.push(ToolCall {
            id: "call".into(),
            name: "read".into(),
            arguments: serde_json::json!({}),
        });
        r.messages.push(assistant);
        r.messages.push(Message::tool("call", "x".repeat(10_000)));
        let original = r.messages.len();
        assert!(preflight(&mut r, 6500).is_err());
        assert_eq!(r.messages.len(), original);
        r.messages.push(Message::new(Role::User, "next turn"));
        let report = preflight(&mut r, 6500).unwrap();
        assert_eq!(report.omitted_messages, 3);
        assert!(
            !r.messages
                .iter()
                .any(|m| m.role == Role::Tool || !m.tool_calls.is_empty())
        );
    }

    #[test]
    fn steering_cannot_replace_the_active_root_task_during_reduction() {
        let mut r = request();
        r.messages[1].content = "original task".repeat(2000);
        for text in ["use CSV", "keep the header"] {
            let mut message = Message::new(Role::User, text);
            message.steering = Some(crate::model::SteeringReceipt {
                id: uuid::Uuid::new_v4(),
                status: crate::model::SteeringStatus::Applied,
            });
            r.messages.push(message);
        }
        assert!(preflight(&mut r, 6500).is_err());
        assert_eq!(r.messages[1].content, "original task".repeat(2000));
        r.messages.push(Message::new(Role::User, "a new task"));
        assert_eq!(preflight(&mut r, 6500).unwrap().omitted_messages, 3);
    }

    #[test]
    fn large_many_turn_history_reduces_with_exact_accounting() {
        let mut r = request();
        r.messages.truncate(1);
        for index in 0..10_000 {
            r.messages
                .push(Message::new(Role::User, format!("turn {index} 雪")));
            r.messages.push(Message::new(Role::Assistant, "reply"));
        }
        let report = preflight(&mut r, 6500).unwrap();
        assert!(report.omitted_messages > 19_990);
        assert_eq!(report.estimated, estimate(&r));
        assert!(report.estimated <= 6500);
        assert!(
            r.messages
                .iter()
                .any(|message| message.content == "turn 9999 雪")
        );
    }
}
