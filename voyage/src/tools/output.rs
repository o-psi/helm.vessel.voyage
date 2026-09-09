//! Secret-safe projections preserve typed result structure and artifact identities.
use super::{Redactor, ToolError};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use voyage_protocol::tool_result::ToolOutput;

fn refused() -> ToolError {
    ToolError::Failed(
        "configured secret in tool result metadata or binary content; result withheld".into(),
    )
}
fn contains(value: &Value, redactor: &Redactor) -> bool {
    match value {
        Value::String(text) => redactor.contains_secret(text),
        Value::Array(items) => items.iter().any(|value| contains(value, redactor)),
        Value::Object(items) => items
            .iter()
            .any(|(key, value)| redactor.contains_secret(key) || contains(value, redactor)),
        _ => false,
    }
}
fn scrub(value: &mut Value, redactor: &Redactor, key: &str) -> Result<(), ToolError> {
    // Structured output is executable data governed by the server's schema.
    // Changing a string merely because its key is "text" would corrupt it.
    if matches!(key, "structuredContent" | "structured_content") {
        return if contains(value, redactor) {
            Err(refused())
        } else {
            Ok(())
        };
    }
    match value {
        Value::String(text) => {
            if matches!(key, "data" | "blob") {
                if redactor.contains_secret(text) {
                    return Err(refused());
                }
                if let Ok(bytes) = STANDARD.decode(text.as_bytes())
                    && redactor.contains_secret(&String::from_utf8_lossy(&bytes))
                {
                    return Err(refused());
                }
            } else if matches!(key, "text" | "description") {
                *text = redactor.redact_public_prefix(text);
            } else if redactor.contains_secret(text) {
                return Err(refused());
            }
        }
        Value::Array(items) => {
            for item in items {
                scrub(item, redactor, key)?;
            }
        }
        Value::Object(items) => {
            for (key, item) in items {
                if redactor.contains_secret(key) {
                    return Err(refused());
                }
                scrub(item, redactor, key)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn reject_split_text(value: &Value, redactor: &Redactor) -> Result<(), ToolError> {
    let Some(blocks) = value.get("content").and_then(Value::as_array) else {
        return Ok(());
    };
    let joined: String = blocks
        .iter()
        .filter_map(|block| {
            block.get("text").and_then(Value::as_str).or_else(|| {
                block
                    .get("resource")
                    .and_then(|resource| resource.get("text"))
                    .and_then(Value::as_str)
            })
        })
        .collect();
    if redactor.contains_secret(&joined) {
        Err(refused())
    } else {
        Ok(())
    }
}
pub(super) fn redact(output: &mut ToolOutput, redactor: &Redactor) -> Result<(), ToolError> {
    let mut value = serde_json::to_value(&*output).map_err(|_| refused())?;
    scrub(&mut value, redactor, "")?;
    reject_split_text(&value, redactor)?;
    *output = serde_json::from_value(value).map_err(|_| refused())?;
    Ok(())
}
pub(super) fn ingest(
    mut value: Value,
    context: &super::ToolContext,
) -> Result<ToolOutput, ToolError> {
    if serde_json::to_vec(&value).map_err(|_| refused())?.len() > context.max_output_bytes {
        return Err(ToolError::Failed(
            "MCP result exceeds configured max_output_bytes".into(),
        ));
    }
    scrub(&mut value, &context.redactor, "")?;
    reject_split_text(&value, &context.redactor)?;
    let scope = context
        .artifact_scope
        .as_ref()
        .ok_or_else(|| ToolError::Failed("MCP results require a session-owned runtime".into()))?;
    context
        .policy
        .check_execution_authority()
        .map_err(|_| ToolError::Denied("execution authority unavailable".into()))?;
    crate::artifacts::Store::open(&scope.directory, scope.session)
        .and_then(|mut store| store.ingest_mcp(&value))
        .map_err(|_| ToolError::Failed("invalid MCP result or artifact storage unavailable".into()))
}
