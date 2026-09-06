//! Redact natural text without changing executable or opaque provider data.
use crate::{
    model::{Message, ToolDefinition},
    tools::Redactor,
};
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

/// Project only the outgoing description. Names and schemas define executable
/// behavior, so a known secret there must refuse dispatch rather than alter it.
pub(crate) fn definition(
    definition: &mut ToolDefinition,
    redactor: &Redactor,
) -> Result<(), ProviderError> {
    definition.description = redactor.redact_public_prefix(&definition.description);
    if redactor.contains_secret(&definition.description)
        || redactor.contains_secret(&definition.name)
        || contains(&definition.input_schema, redactor)
    {
        return Err(ProviderError::InvalidResponse(
            "configured secret in executable tool metadata; provider dispatch refused".into(),
        ));
    }
    Ok(())
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
