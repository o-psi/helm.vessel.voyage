//! Bounded observations preserve an explicit route to the complete public message.
use crate::model::Message;
use anyhow::Result;
use serde_json::{Value, json};
const MESSAGE_BYTES: usize = 32768;
const PAGE_BYTES: usize = 512 * 1024;

pub(super) fn text_prefix(text: &str, limit: usize) -> (&str, bool) {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], end < text.len())
}
pub(super) fn full(message: &Message) -> Value {
    json!({"role":message.role,"content":message.content,"tool_calls":message.tool_calls,"tool_call_id":message.tool_call_id,"tool_success":message.tool_success,"steering":message.steering})
}
fn bounded(message: &Message, index: usize) -> Result<Value> {
    let mut value = full(message);
    if serde_json::to_vec(&value)?.len() <= MESSAGE_BYTES {
        value["message_index"] = json!(index);
        value["projection_truncated"] = json!(false);
        return Ok(value);
    }
    let (content, truncated) = text_prefix(&message.content, 4096);
    Ok(
        json!({"role":message.role,"content":content,"content_truncated":truncated,"content_bytes":message.content.len(),"tool_calls":[],"tool_calls_omitted":!message.tool_calls.is_empty(),"tool_success":message.tool_success,"message_index":index,"projection_truncated":true,"complete_message":"message_chunk"}),
    )
}
pub(super) fn page(messages: &[Message], offset: usize, limit: usize) -> Result<Vec<Value>> {
    let mut output = Vec::new();
    let mut bytes = 0;
    for (index, message) in messages.iter().enumerate().skip(offset).take(limit) {
        let value = bounded(message, index)?;
        let length = serde_json::to_vec(&value)?.len();
        if bytes + length > PAGE_BYTES && !output.is_empty() {
            break;
        }
        bytes += length;
        output.push(value);
    }
    Ok(output)
}
pub(super) fn recent(messages: &[Message]) -> Result<(Vec<Value>, usize)> {
    let mut output = Vec::new();
    let mut bytes = 0;
    let mut offset = messages.len();
    for (index, message) in messages.iter().enumerate().rev().take(128) {
        let value = bounded(message, index)?;
        let length = serde_json::to_vec(&value)?.len();
        if bytes + length > PAGE_BYTES && !output.is_empty() {
            break;
        }
        bytes += length;
        offset = index;
        output.push(value);
    }
    output.reverse();
    Ok((output, offset))
}
