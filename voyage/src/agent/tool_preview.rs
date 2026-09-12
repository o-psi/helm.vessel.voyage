//! A bounded display accumulator, separate from the provider's executable assembly.
use crate::{provider::ProviderDelta, tools::Redactor};
use std::collections::BTreeMap;
use voyage_protocol::tool_preview::{MAX_ARGUMENT_BYTES, MAX_CALLS, ToolPreview};

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    raw: String,
    truncated: bool,
}
#[derive(Default)]
pub(super) struct Previews(BTreeMap<usize, Call>);

fn prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

// Decode JSON string escapes before redaction, including unfinished strings. Never
// pass the raw JSON through: escaped credentials must have the same confidentiality
// as plain credentials. Incomplete escape sequences remain private until complete.
fn readable(raw: &str) -> String {
    let mut result = String::new();
    let mut start = 0;
    while start < raw.len() {
        let Some(relative) = raw[start..].find('"') else {
            result.push_str(&raw[start..]);
            break;
        };
        let begin = start + relative;
        result.push_str(&raw[start..begin]);
        let mut escaped = false;
        let mut end = None;
        for (i, ch) in raw[begin + 1..].char_indices() {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                end = Some(begin + 1 + i + 1);
                break;
            }
        }
        if let Some(end) = end {
            let Ok(text) = serde_json::from_str::<String>(&raw[begin..end]) else {
                break;
            };
            result.push_str(&text);
            start = end;
        } else {
            // At most a surrogate pair / unfinished escape is held back. Invalid
            // JSON is never repaired for execution; this is a labelled preview.
            let mut finish = raw.len();
            for _ in 0..14 {
                if let Ok(text) =
                    serde_json::from_str::<String>(&format!("{}\"", &raw[begin..finish]))
                {
                    result.push_str(&text);
                    break;
                }
                if finish <= begin + 1 {
                    break;
                }
                finish -= 1;
                while !raw.is_char_boundary(finish) {
                    finish -= 1;
                }
            }
            break;
        }
    }
    result
}

impl Previews {
    pub(super) fn update(&mut self, delta: &ProviderDelta) {
        let ProviderDelta::ToolCall {
            index,
            id,
            name,
            arguments,
        } = delta
        else {
            return;
        };
        if !self.0.contains_key(index) && self.0.len() >= MAX_CALLS {
            return;
        }
        let call = self.0.entry(*index).or_default();
        if let Some(id) = id {
            call.id
                .push_str(prefix(id, 1024usize.saturating_sub(call.id.len())));
        }
        if let Some(name) = name {
            call.name
                .push_str(prefix(name, 128usize.saturating_sub(call.name.len())));
        }
        if !call.truncated {
            let text = prefix(arguments, MAX_ARGUMENT_BYTES.saturating_sub(call.raw.len()));
            call.raw.push_str(text);
            call.truncated = text.len() != arguments.len();
        }
    }
    pub(super) fn public(&self, attempt_id: uuid::Uuid, redactor: &Redactor) -> Vec<ToolPreview> {
        self.0
            .iter()
            .map(|(&index, call)| {
                // Conservative tool-aware gate: process input/env, workflow private
                // inputs, browser uploads and unknown tools never stream arguments.
                let allowed = matches!(
                    call.name.as_str(),
                    "shell"
                        | "apply_patch"
                        | "write_file"
                        | "read_file"
                        | "search_files"
                        | "list_directory"
                );
                let decoded = readable(&call.raw);
                let stable = redactor.stable_prefix(&decoded, false);
                let arguments = if allowed {
                    redactor.redact_public_prefix(&decoded[..stable])
                } else {
                    "[arguments withheld during generation]".into()
                };
                let name_end = redactor.stable_prefix(&call.name, false);
                ToolPreview {
                    attempt_id,
                    index,
                    call_id: (!call.id.is_empty()
                        && !redactor.contains_secret(&call.id)
                        && redactor.stable_prefix(&call.id, false) == call.id.len())
                    .then(|| call.id.clone()),
                    name: prefix(&redactor.redact_public_prefix(&call.name[..name_end]), 128)
                        .to_owned(),
                    truncated: call.truncated || arguments.len() > MAX_ARGUMENT_BYTES,
                    arguments: prefix(&arguments, MAX_ARGUMENT_BYTES).to_owned(),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn delta(index: usize, name: Option<&str>, arguments: &str) -> ProviderDelta {
        ProviderDelta::ToolCall {
            index,
            id: None,
            name: name.map(str::to_owned),
            arguments: arguments.into(),
        }
    }
    #[test]
    fn decoded_prefix_handles_unicode_escapes_and_partial_strings() {
        assert_eq!(
            readable(r#"{"command":"hello\n世\u754c"#),
            "{command:hello\n世界"
        );
        assert_eq!(readable(r#"{"command":"hello\u75"#), "{command:hello");
        assert_eq!(readable(r#"{"command":"\uD83D\uDE00"}"#), "{command:😀}");
    }
    #[test]
    fn credentials_are_held_across_chunks_and_decoded_before_redaction() {
        let redactor = Redactor::default().with_additional(["secret-token".into()]);
        let mut previews = Previews::default();
        previews.update(&delta(0, Some("shell"), r#"{"command":"secret-"#));
        assert!(
            !previews.public(uuid::Uuid::nil(), &redactor)[0]
                .arguments
                .contains("secret")
        );
        previews.update(&delta(0, None, r#"\u0074oken"}"#));
        let text = &previews.public(uuid::Uuid::nil(), &redactor)[0].arguments;
        assert!(!text.contains("secret"));
        assert!(text.contains("[REDACTED]"));
    }
    #[test]
    fn interleaved_calls_are_bounded_and_private() {
        let redactor = Redactor::default();
        let mut previews = Previews::default();
        previews.update(&delta(7, Some("process"), r#"{"data":"private"}"#));
        previews.update(&delta(2, Some("shell"), r#"{"command":"hi "#));
        previews.update(&delta(2, None, "世"));
        let values = previews.public(uuid::Uuid::nil(), &redactor);
        assert_eq!(values[0].index, 2);
        assert!(values[0].arguments.contains("hi 世"));
        assert!(!values[1].arguments.contains("private"));
        previews.update(&delta(2, None, &"世".repeat(MAX_ARGUMENT_BYTES)));
        let values = previews.public(uuid::Uuid::nil(), &redactor);
        assert!(values[0].truncated);
        assert!(values[0].arguments.len() <= MAX_ARGUMENT_BYTES);
        for index in 0..100 {
            previews.update(&delta(index, Some("unknown"), "hidden"));
        }
        assert_eq!(previews.0.len(), MAX_CALLS);
    }
}
