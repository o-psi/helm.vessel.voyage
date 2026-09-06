//! Redact natural text without changing executable or opaque provider data.
use crate::{model::Message, tools::Redactor};
use serde_json::Value;

use super::ProviderError;

fn contains(value: &Value, redactor: &Redactor) -> bool {
    match value {
        Value::String(text) => redactor.contains_secret(text),
        Value::Array(values) => values.iter().any(|value| contains(value, redactor)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| redactor.contains_secret(key) || contains(value, redactor)),
        _ => false,
    }
}

/// Called on newly received messages and outgoing history copies. Never rewrite
/// tool arguments, identities, reasoning signatures, or unknown continuation.
pub(crate) fn message(message: &mut Message, redactor: &Redactor) -> Result<(), ProviderError> {
    message.content = redactor.redact_public_prefix(&message.content);
    if let Some(state) = &mut message.provider_state {
        // Responses owns this versioned local envelope. Its natural text is a
        // replay copy of assistant output, not an opaque continuation token.
        if state["kind"] == "openai_responses_replay"
            && state["version"] == 1
            && let Some(items) = state.get_mut("items").and_then(Value::as_array_mut)
        {
            for item in items {
                if item["type"] == "function_call"
                    && let Some(arguments) = item["arguments"].as_str()
                    && let Ok(arguments) = serde_json::from_str::<Value>(arguments)
                    && contains(&arguments, redactor)
                {
                    return Err(ProviderError::InvalidResponse(
                        "configured secret in executable provider replay data".into(),
                    ));
                }
                if item["type"] == "message"
                    && let Some(content) = item.get_mut("content").and_then(Value::as_array_mut)
                {
                    for block in content {
                        let field = match block["type"].as_str() {
                            Some("output_text") => "text",
                            Some("refusal") => "refusal",
                            _ => continue,
                        };
                        if let Some(text) = block[field].as_str() {
                            block[field] = Value::String(redactor.redact_public_prefix(text));
                        }
                    }
                }
            }
        }
    }
    if redactor.contains_secret(&message.content)
        || message
            .tool_call_id
            .as_ref()
            .is_some_and(|id| redactor.contains_secret(id))
        || message.tool_calls.iter().any(|call| {
            redactor.contains_secret(&call.id)
                || redactor.contains_secret(&call.name)
                || contains(&call.arguments, redactor)
        })
        || message
            .provider_state
            .as_ref()
            .is_some_and(|state| contains(state, redactor))
    {
        return Err(ProviderError::InvalidResponse(
            "configured secret in executable or opaque provider data; response cannot be safely retained or replayed".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Role, ToolCall};
    use serde_json::json;

    #[test]
    fn responses_text_redacts_without_changing_nonsecret_opaque_items() {
        let redactor = Redactor::new(["private-\"\n秘密".into()]);
        let opaque =
            json!({"type":"reasoning","id":"reason-1","encrypted_content":"opaque-signed-bytes"});
        let mut value = Message::new(Role::Assistant, "before private-\"\n秘密 after");
        value.provider_state = Some(
            json!({"kind":"openai_responses_replay","version":1,"items":[opaque.clone(),
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":value.content},{"type":"refusal","refusal":"private-\"\n秘密"}]}]}),
        );
        message(&mut value, &redactor).unwrap();
        assert_eq!(value.content, "before [REDACTED] after");
        let state = value.provider_state.unwrap();
        assert_eq!(state["items"][0], opaque);
        assert_eq!(state["items"][1]["content"][0]["text"], value.content);
        assert_eq!(state["items"][1]["content"][1]["refusal"], "[REDACTED]");
    }

    #[test]
    fn opaque_state_is_preserved_or_refused_never_silently_rewritten() {
        let redactor = Redactor::new(["private-secret".into()]);
        let mut value = Message::new(Role::Assistant, "ordinary");
        value.provider_state = Some(json!({"custom":["signed-continuation",1]}));
        let original = value.clone();
        message(&mut value, &redactor).unwrap();
        assert_eq!(value.provider_state, original.provider_state);
        value.provider_state = Some(json!({"opaque":"private-secret"}));
        assert!(message(&mut value, &redactor).is_err());
        assert_eq!(value.provider_state.unwrap()["opaque"], "private-secret");
    }

    #[test]
    fn executable_arguments_and_identity_are_refused_before_rewriting() {
        let secret = "private-\"\n秘密";
        let redactor = Redactor::new([secret.into()]);
        let mut value = Message::new(Role::Assistant, "ordinary");
        value.tool_calls.push(ToolCall {
            id: "call-1".into(),
            name: "shell".into(),
            arguments: json!({"command":secret}),
        });
        assert!(message(&mut value, &redactor).is_err());
        assert_eq!(value.tool_calls[0].arguments["command"], secret);
        value.tool_calls.clear();
        value.tool_call_id = Some(secret.into());
        assert!(message(&mut value, &redactor).is_err());
    }
}
