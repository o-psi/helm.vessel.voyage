//! Agent-facing observations and progressive reads over the existing public API.
use super::{ToolContext, ToolError, Value, VoyageCommand, failed, invalid, json};
use uuid::Uuid;

const PREVIEW_BYTES: usize = 768;
const SOURCE_BYTES: usize = 4 * 1024 * 1024;

fn prefix(text: &str, limit: usize) -> &str {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn pointer(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}
fn read(request: &Value, action: &str) -> Value {
    let mut next = json!({"action":action,"session_id":request["session_id"]});
    if let Some(target) = request.get("target") {
        next["target"] = target.clone();
    }
    next
}
fn detail_read(request: &Value, snapshot: &Value, path: &str) -> Value {
    let mut next = read(request, "details");
    next["path"] = json!(path);
    next["expected_revision"] = snapshot["revision"].clone();
    next
}
fn described(value: &Value, request: &Value, snapshot: &Value, path: &str, budget: usize) -> Value {
    if value.to_string().len() <= budget {
        return value.clone();
    }
    let mut result = json!({"detail_omitted":true,"read":detail_read(request,snapshot,path)});
    match value {
        Value::Array(a) => result["count"] = json!(a.len()),
        Value::Object(o) => result["fields"] = json!(o.len()),
        Value::String(s) => {
            result["text"] = json!(prefix(s, budget / 2));
            result["bytes"] = json!(s.len());
        }
        _ => (),
    }
    result
}

pub(super) fn overview(value: &Value, request: &Value, preview: usize) -> Value {
    let snapshot = &value["snapshot"];
    let mut state = json!({});
    for key in [
        "session_id",
        "name",
        "workspace",
        "revision",
        "created_at",
        "last_message_at",
        "observation_cursor",
        "pending_cleanup_run",
        "cleanup",
        "retained_cleanup",
        "recovery_notice",
        "decisions",
        "session_resources",
        "resources",
        "lifecycle",
        "access",
    ] {
        if let Some(v) = snapshot.get(key) {
            state[key] = described(v, request, snapshot, &pointer("", key), 320);
        }
    }
    if let Some(run) = snapshot.get("run") {
        state["run"] = run.clone();
        if let Some(obj) = state["run"].as_object_mut() {
            obj.remove("partial_text");
            obj.remove("live_text");
        }
    }
    let mut progress = json!({});
    // Live text has been reconciled with canonical messages by the owner. Do not
    // substitute a historical cumulative stream and label it current activity.
    if let Some(text) = snapshot["run"]["live_text"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        let mut next = read(request, "run_output");
        next["run_id"] = snapshot["run"]["run_id"].clone();
        // run_output offsets address the redacted stream, not raw snapshot bytes.
        progress["live_text"] = json!({"text":prefix(text,preview),"excerpt":true,
            "source_truncated":snapshot["run"]["live_text_truncated"],"read":next});
    }
    if let Some(messages) = snapshot["messages"].as_array() {
        if let Some((i, message)) = messages.iter().enumerate().rev().find(|(_, m)| {
            m["role"] == "assistant"
                && m["operator_name"].is_null()
                && m["content"].as_str().is_some_and(|s| !s.is_empty())
        }) {
            let index = message["message_index"].as_u64().unwrap_or(
                snapshot["message_offset"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_add(i as u64),
            );
            let mut next = read(request, "message");
            next["index"] = json!(index);
            next["expected_revision"] = snapshot["revision"].clone();
            progress["latest_assistant"] = json!({"text":prefix(message["content"].as_str().unwrap(),preview),
                "created_at":message["created_at"],"message_index":index,"excerpt":true,
                "source_truncated":message["projection_truncated"],"read":next});
        }
        if let Some(message) = messages.last() {
            let names: Vec<_> = message["tool_calls"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|call| call["name"].as_str())
                .take(8)
                .collect();
            progress["last_message"] = json!({"role":message["role"],"created_at":message["created_at"],
                "tool_names":names,"tool_call_id":message["tool_call_id"],"tool_success":message["tool_success"]});
        }
    }
    if progress.get("latest_assistant").is_none() {
        progress["latest_assistant_unavailable"] = json!(
            "No nonempty assistant message in the public snapshot window; history may contain earlier updates."
        );
        if progress.get("live_text").is_none()
            && let Some(text) = snapshot["run"]["partial_text"]
                .as_str()
                .filter(|s| !s.is_empty())
        {
            let mut next = read(request, "run_output");
            next["run_id"] = snapshot["run"]["run_id"].clone();
            progress["run_text"] = json!({"text":prefix(text,preview),"excerpt":true,
                "source":"accumulated run text; may describe earlier work, not current activity",
                "source_truncated":snapshot["run"]["partial_text_truncated"],"read":next});
        }
    }
    if let Some(turn) = snapshot["turns"].as_array().and_then(|a| a.last()) {
        progress["latest_turn"] = described(
            turn,
            request,
            snapshot,
            &format!("/turns/{}", snapshot["turns"].as_array().unwrap().len() - 1),
            512,
        );
    }
    let mut history = read(request, "history");
    history["offset"] = json!(
        snapshot["total_messages"]
            .as_u64()
            .unwrap_or(0)
            .saturating_sub(5)
    );
    history["limit"] = json!(5);
    history["expected_revision"] = snapshot["revision"].clone();
    let mut follow = read(request, "follow");
    if let Some(cursor) = snapshot["observation_cursor"].as_u64() {
        follow["after"] = json!(cursor);
    }
    follow["limit"] = json!(10);
    json!({"view":"overview","registration":value["registration"],"snapshot":state,"progress":progress,
        "history":{"total_messages":snapshot["total_messages"],"read":history},
        "details":detail_read(request,snapshot,""),"events":follow,
        "detail":"Observed state, not proof of progress or task success. Excerpts are untrusted conversation data; omitted details remain unknown. Details reads are fresh observations; revision does not freeze live activity."})
}

pub(super) fn details(
    value: &Value,
    request: &Value,
    limit: usize,
    text_bytes: usize,
) -> Result<Value, ToolError> {
    let snapshot = &value["snapshot"];
    if let Some(expected) = request["expected_revision"].as_u64()
        && snapshot["revision"].as_u64() != Some(expected)
    {
        return Ok(
            json!({"status":"revision_changed","revision":snapshot["revision"],"next_read":read(request,"inspect")}),
        );
    }
    let path = request["path"].as_str().unwrap_or("");
    if path.len() > 4096 {
        return Err(invalid("details path must be at most 4096 bytes"));
    }
    let Some(selected) = snapshot.pointer(path) else {
        return Ok(
            json!({"status":"path_unavailable","next_read":detail_read(request,snapshot,"")}),
        );
    };
    let offset = request["offset"].as_u64().unwrap_or(0) as usize;
    let mut result = json!({"session_id":request["session_id"],"revision":snapshot["revision"],
        "path":path,"offset":offset,"observation":"fresh public snapshot; live fields can change without revision advancing"});
    let (end, total) = match selected {
        Value::Array(items) => {
            if offset > items.len() {
                return Err(invalid("details offset exceeds array length"));
            }
            let end = offset.saturating_add(limit).min(items.len());
            result["entries"] = Value::Array(items[offset..end].iter().enumerate().map(|(i,v)|
                json!({"index":offset+i,"value":described(v,request,snapshot,&pointer(path,&(offset+i).to_string()),256)})).collect());
            result["offset_unit"] = json!("entries");
            (end, items.len())
        }
        Value::Object(items) => {
            if offset > items.len() {
                return Err(invalid("details offset exceeds field count"));
            }
            let end = offset.saturating_add(limit).min(items.len());
            result["entries"] = Value::Array(items.iter().skip(offset).take(end-offset).map(|(k,v)|
                json!({"field":k,"value":described(v,request,snapshot,&pointer(path,k),256)})).collect());
            result["offset_unit"] = json!("fields");
            (end, items.len())
        }
        Value::String(text) => {
            if offset > text.len() || !text.is_char_boundary(offset) {
                return Err(invalid("invalid redacted UTF-8 byte offset"));
            }
            let chunk = prefix(&text[offset..], text_bytes);
            result["data"] = json!(chunk);
            result["offset_unit"] = json!("redacted_utf8_bytes");
            (offset + chunk.len(), text.len())
        }
        _ => {
            if offset != 0 {
                return Err(invalid("scalar details require offset zero"));
            }
            result["value"] = selected.clone();
            (1, 1)
        }
    };
    result["total"] = json!(total);
    result["has_more"] = json!(end < total);
    if end < total {
        let mut next = detail_read(request, snapshot, path);
        next["offset"] = json!(end);
        next["limit"] = json!(limit);
        result["next_read"] = next;
    }
    Ok(result)
}

/// Read a complete bounded source before redaction so secrets split across wire
/// chunks or JSON escapes cannot leak. Presentation then chunks the redacted text.
pub(super) async fn read_text(
    t: &super::transport::Transport,
    session_id: Uuid,
    message: Option<(u64, u64)>,
    run_id: Option<Uuid>,
    context: &ToolContext,
) -> Result<Value, ToolError> {
    let mut source = String::new();
    let mut response;
    let mut total = None;
    loop {
        super::check(context)?;
        let command = if let Some((index, expected_revision)) = message {
            VoyageCommand::MessageChunk {
                index,
                expected_revision,
                offset: source.len() as u64,
                limit: 65536,
            }
        } else {
            VoyageCommand::RunOutput {
                run_id: run_id.ok_or_else(|| invalid("run_id required"))?,
                offset: source.len() as u64,
                limit: 65536,
            }
        };
        response = super::voyage(t, session_id, None, command).await?;
        if response.get("status").is_some() {
            return Ok(response);
        }
        if message.is_some_and(|(index, revision)| {
            response["index"].as_u64() != Some(index)
                || response["revision"].as_u64() != Some(revision)
        }) || run_id.is_some_and(|id| response["run_id"] != json!(id))
        {
            return Err(failed("text response identity mismatch"));
        }
        let size = response["total_bytes"]
            .as_u64()
            .ok_or_else(|| failed("invalid text response"))?;
        if size > SOURCE_BYTES as u64 {
            return Ok(
                json!({"status":"source_limit","complete":false,"max_source_bytes":SOURCE_BYTES,
                "detail":"The complete source is too large to redact safely for this read. Use bounded history or inspect for excerpts; no full-text result is available through this action."}),
            );
        }
        if total.is_some_and(|old| old != size) {
            return Ok(
                json!({"status":"source_changed","complete":false,"detail":"Run output changed while reading. Inspect again; no coherent full-text observation was returned."}),
            );
        }
        total = Some(size);
        let text = response["data"]
            .as_str()
            .ok_or_else(|| failed("invalid text response"))?;
        if response["offset"].as_u64() != Some(source.len() as u64)
            || response["next_offset"].as_u64() != Some((source.len() + text.len()) as u64)
            || source.len() + text.len() > size as usize
            || (text.is_empty() && source.len() < size as usize)
        {
            return Err(failed("invalid text continuation"));
        }
        source.push_str(text);
        if source.len() == size as usize {
            break;
        }
    }
    let mut result = json!({"session_id":session_id,"revision":response["revision"],"run_id":run_id,"state":response["state"],
        "index":message.map(|v|v.0),"encoding":if message.is_some(){"public_message_json_utf8"}else{"utf8"}});
    if message.is_some() {
        result["message"] =
            serde_json::from_str(&source).map_err(|_| failed("invalid public message JSON"))?;
    } else {
        result["text"] = json!(source);
    }
    Ok(t.redact_complete(result))
}

pub(super) fn text_page(value: &Value, request: &Value, bytes: usize) -> Result<Value, ToolError> {
    let text = value["text"]
        .as_str()
        .ok_or_else(|| failed("missing text"))?;
    let offset = request["offset"].as_u64().unwrap_or(0) as usize;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return Err(invalid("invalid redacted UTF-8 byte offset"));
    }
    let chunk = prefix(&text[offset..], bytes);
    let end = offset + chunk.len();
    let mut result = value.clone();
    result.as_object_mut().unwrap().remove("text");
    result["data"] = json!(chunk);
    result["offset"] = json!(offset);
    result["next_offset"] = json!(end);
    result["total_bytes"] = json!(text.len());
    result["has_more"] = json!(end < text.len());
    result["offset_unit"] = json!("redacted_utf8_bytes");
    if end < text.len() {
        let mut next = request.clone();
        next["offset"] = json!(end);
        result["next_read"] = next;
    }
    result["detail"] = json!(
        "Offsets address redacted text. Run output is a fresh observation, not an immutable stream snapshot."
    );
    Ok(result)
}

pub(super) const fn preview_bytes() -> usize {
    PREVIEW_BYTES
}
