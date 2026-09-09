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
    if let Some(title) = definition
        .annotations
        .as_mut()
        .and_then(|hints| hints.title.as_mut())
    {
        *title = redactor.redact_public_prefix(title);
    }
    if redactor.contains_secret(&definition.description)
        || redactor.contains_secret(&definition.name)
        || contains(&definition.input_schema, redactor)
        || definition
            .output_schema
            .as_ref()
            .is_some_and(|schema| contains(schema, redactor))
        || definition
            .annotations
            .as_ref()
            .and_then(|hints| hints.title.as_ref())
            .is_some_and(|title| redactor.contains_secret(title))
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
    if let Some(output) = &message.tool_output {
        let value = serde_json::to_value(output)
            .map_err(|_| ProviderError::Request("invalid tool result".into()))?;
        if contains(&value, redactor) {
            return Err(ProviderError::Request(
                "configured secret in typed tool result; provider dispatch refused".into(),
            ));
        }
    }
    for part in &mut message.parts {
        if let voyage_protocol::content::ContentPart::Text { text } = part {
            *text = redactor.redact_public_prefix(text);
        }
    }
    if redactor.contains_secret(&crate::images::text(&message.parts)) {
        return Err(ProviderError::Request(
            "configured secret spans content parts; provider dispatch refused".into(),
        ));
    }
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
mod image_tests {
    use super::*;
    #[test]
    fn secrets_split_across_ordered_parts_are_not_dispatched() {
        let redactor = Redactor::new(["private-token".into()]);
        let mut message = Message::new(crate::model::Role::User, "private-token");
        message.parts = vec![
            voyage_protocol::content::ContentPart::Text {
                text: "private-".into(),
            },
            voyage_protocol::content::ContentPart::Text {
                text: "token".into(),
            },
        ];
        assert!(super::message(&mut message, &redactor).is_err());
    }
}
