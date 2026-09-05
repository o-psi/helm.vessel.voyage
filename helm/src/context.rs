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
    // Count serialization without allocating a second copy of potentially large
    // tool results or continuation envelopes.
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
    let bytes = if serde_json::to_writer(&mut counter, request).is_ok() {
        counter.0
    } else {
        usize::MAX
    };
    bytes
        .saturating_add(4096)
        .saturating_add(request.messages.len().saturating_mul(256))
        .saturating_add(request.tools.len().saturating_mul(256))
        .saturating_add(request.max_tokens.unwrap_or(8192) as usize)
}

/// Keep all system guidance and the latest user turn, including its complete tool
/// groups. Drop oldest complete turns until the projection fits. A single large
/// active turn fails locally; silently truncating its tools would falsify evidence.
pub fn preflight(request: &mut ModelRequest, limit: usize) -> Result<ContextReport, ContextError> {
    let mut omitted = 0;
    let mut has_marker = false;
    loop {
        let estimated = estimate(request);
        if estimated <= limit && limit > 0 {
            return Ok(ContextReport {
                estimated,
                limit,
                omitted_messages: omitted,
            });
        }
        let users: Vec<_> = request
            .messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| (message.role == Role::User).then_some(index))
            .take(2)
            .collect();
        if users.len() < 2 {
            return Err(ContextError { estimated, limit });
        }
        let end = users[1];
        let before = request.messages.len();
        let mut index = 0;
        request.messages.retain(|message| {
            let keep = index >= end || message.role == Role::System;
            index += 1;
            keep
        });
        omitted += before - request.messages.len();
        // This marker is runtime context only, never inserted in canonical history.
        let marker = format!(
            "[Context projection: {omitted} older messages omitted; canonical transcript retained locally.]"
        );
        if !has_marker {
            request
                .messages
                .insert(0, crate::model::Message::new(Role::System, marker));
            has_marker = true;
        } else {
            request.messages[0].content = marker;
        }
    }
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
}
