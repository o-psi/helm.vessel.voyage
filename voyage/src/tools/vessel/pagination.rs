//! Private bounded continuation state; every resumed read rechecks remote authority.
use super::json_stream::Stream;
use super::{ToolContext, ToolError, Value, VoyageCommand, failed, invalid, json};
use crate::tools::ToolReport;
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};
use uuid::Uuid;

const WIRE_BYTES: u32 = 65536;
const CALL_BYTES: usize = 256 * 1024;
const CURSORS: usize = 16;
const CURSOR_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Clone)]
struct Cursor {
    binding: [u8; 32],
    seen: Instant,
    stream: Stream,
    source: String,
    position: usize,
    wire_offset: u64,
    source_end: u64,
    emitted: u64,
    metadata: Value,
}
#[derive(Default)]
pub(super) struct Pages {
    cursors: Mutex<VecDeque<(Uuid, Cursor)>>,
}

fn normalized(request: &Value) -> Value {
    let mut value = request.clone();
    let o = value.as_object_mut().unwrap();
    o.remove("cursor");
    o.remove("offset");
    o.entry("target").or_insert(json!("local"));
    value
}
fn restart(request: &Value, status: &str) -> Value {
    let mut next = request.clone();
    next.as_object_mut().unwrap().remove("cursor");
    next["offset"] = json!(0);
    json!({"status":status,"complete":false,"next_read":next,
        "detail":"Continuation is unavailable or does not match this read. Restart this read from offset zero; a message revision change requires fresh inspection."})
}
fn cut(text: &str, size: usize) -> usize {
    let mut end = text.len().min(size);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}
fn id(request: &Value, key: &str) -> Result<Uuid, ToolError> {
    serde_json::from_value(request[key].clone()).map_err(|_| invalid("invalid page identity"))
}
async fn chunk(
    t: &super::transport::Transport,
    request: &Value,
    offset: u64,
    limit: u32,
) -> Result<Value, ToolError> {
    let session = id(request, "session_id")?;
    let command = if request["action"] == "message" {
        VoyageCommand::MessageChunk {
            index: request["index"]
                .as_u64()
                .ok_or_else(|| invalid("message index required"))?,
            expected_revision: request["expected_revision"]
                .as_u64()
                .ok_or_else(|| invalid("message revision required"))?,
            offset,
            limit,
        }
    } else {
        VoyageCommand::RunOutput {
            run_id: id(request, "run_id")?,
            offset,
            limit,
        }
    };
    let result = super::voyage(t, session, None, command).await?;
    if result.get("status").is_some() {
        return Ok(t.redact_complete(result));
    }
    if (request["action"] == "message"
        && (result["index"] != request["index"]
            || result["revision"] != request["expected_revision"]))
        || (request["action"] == "run_output" && result["run_id"] != request["run_id"])
    {
        return Err(failed("text response identity mismatch"));
    }
    let data = result["data"]
        .as_str()
        .ok_or_else(|| failed("invalid text response"))?;
    let total = result["total_bytes"]
        .as_u64()
        .ok_or_else(|| failed("invalid text length"))?;
    let end = offset
        .checked_add(data.len() as u64)
        .ok_or_else(|| failed("text offset overflow"))?;
    if result["offset"].as_u64() != Some(offset)
        || result["next_offset"].as_u64() != Some(end)
        || end > total
        || data.len() > limit as usize
        || (data.is_empty() && end < total)
    {
        return Err(failed("invalid text continuation"));
    }
    Ok(result)
}
impl Pages {
    pub(super) async fn read(
        &self,
        t: &super::transport::Transport,
        context: &ToolContext,
        request: &Value,
    ) -> Result<ToolReport, ToolError> {
        super::check(context)?;
        let redactor = t.page_redactor(&context.redactor);
        let mut hash = Sha256::new();
        hash.update(normalized(request).to_string());
        hash.update(t.page_identity());
        hash.update(
            serde_json::to_vec(&redactor.secrets)
                .map_err(|_| failed("redaction identity unavailable"))?,
        );
        let binding: [u8; 32] = hash.finalize().into();
        let offset = request["offset"].as_u64().unwrap_or(0);
        let mut state = if let Some(cursor) = request["cursor"].as_str() {
            let cursor = Uuid::parse_str(cursor).map_err(|_| invalid("invalid cursor"))?;
            let saved = {
                let mut cache = self
                    .cursors
                    .lock()
                    .map_err(|_| failed("page cache unavailable"))?;
                cache.retain(|(_, s)| s.seen.elapsed() < CURSOR_TTL);
                cache
                    .iter()
                    .find(|(key, _)| *key == cursor)
                    .map(|(_, s)| s.clone())
            };
            let Some(state) = saved else {
                return super::output(restart(request, "cursor_expired"), context, request);
            };
            if state.binding != binding || state.emitted != offset {
                return super::output(restart(request, "cursor_mismatch"), context, request);
            }
            // Cached bytes are not authority. This also fences a message's
            // revision before releasing another page from the private cache.
            let probe = chunk(t, request, 0, 4).await?;
            if probe.get("status").is_some() {
                return super::output(probe, context, request);
            }
            let total = probe["total_bytes"].as_u64().unwrap();
            if total < state.source_end
                || (request["action"] == "message" && total != state.source_end)
            {
                return super::output(
                    json!({"status":"source_changed","complete":false,"detail":"Source length changed; restart inspection."}),
                    context,
                    request,
                );
            }
            state
        } else {
            if offset != 0 {
                return super::output(restart(request, "cursor_required"), context, request);
            }
            let initial = chunk(t, request, 0, WIRE_BYTES).await?;
            if initial.get("status").is_some() {
                return super::output(initial, context, request);
            }
            Cursor {
                binding,
                seen: Instant::now(),
                stream: Stream::new(request["action"] == "message", &redactor)?,
                source: initial["data"].as_str().unwrap().to_owned(),
                position: 0,
                wire_offset: initial["next_offset"].as_u64().unwrap(),
                source_end: initial["total_bytes"].as_u64().unwrap(),
                emitted: 0,
                metadata: json!({"session_id":request["session_id"],"revision":initial["revision"],"run_id":initial["run_id"],"state":initial["state"],"index":initial["index"]}),
            }
        };
        let mut fetched = 0;
        while state.stream.ready.len() < 4096 && !state.stream.done {
            super::check(context)?;
            if state.position < state.source.len() {
                let c = state.source[state.position..].chars().next().unwrap();
                state.position += c.len_utf8();
                state.stream.push(c)?;
            } else if state.wire_offset >= state.source_end {
                state.stream.finish()?;
            } else {
                if fetched >= CALL_BYTES {
                    break;
                }
                let amount = (state.source_end - state.wire_offset).min(WIRE_BYTES as u64) as u32;
                let next = chunk(t, request, state.wire_offset, amount).await?;
                if next.get("status").is_some() {
                    return super::output(next, context, request);
                }
                let total = next["total_bytes"].as_u64().unwrap();
                if total < state.source_end
                    || (request["action"] == "message" && total != state.source_end)
                {
                    return super::output(
                        json!({"status":"source_changed","complete":false,"detail":"Source length changed; restart inspection."}),
                        context,
                        request,
                    );
                }
                state.source = next["data"].as_str().unwrap().to_owned();
                state.position = 0;
                state.wire_offset = next["next_offset"].as_u64().unwrap();
                fetched += state.source.len();
            }
        }
        let cursor = Uuid::new_v4();
        let mut bytes = cut(&state.stream.ready, 4096);
        let text = loop {
            let more = !state.stream.done || bytes < state.stream.ready.len();
            let mut result = super::redact(t.redact_complete(state.metadata.clone()), context)?;
            result["encoding"] = json!(if request["action"] == "message" {
                "public_message_json_utf8"
            } else {
                "utf8"
            });
            result["data"] = json!(&state.stream.ready[..bytes]);
            result["offset"] = json!(offset);
            result["next_offset"] = json!(offset + bytes as u64);
            result["total_bytes"] = if state.stream.done {
                json!(offset + state.stream.ready.len() as u64)
            } else {
                Value::Null
            };
            result["source_total_bytes"] = json!(state.source_end);
            result["has_more"] = json!(more);
            result["offset_unit"] = json!("redacted_utf8_bytes");
            result["detail"] = json!(
                "Follow next_read exactly. Cursor state is private and expires; offsets alone cannot resume. Run text is the prefix observed at the first read, not proof of task completion."
            );
            if more {
                let mut next = request.clone();
                next["offset"] = json!(offset + bytes as u64);
                next["cursor"] = json!(cursor);
                result["next_read"] = next;
            }
            let text =
                serde_json::to_string(&result).map_err(|_| failed("page encoding failed"))?;
            if text.len() <= context.max_output_bytes {
                break text;
            }
            if bytes == 0 {
                return super::output(
                    json!({"status":"output_limit","complete":false,"detail":"Output budget cannot fit a page with its continuation; increase the budget."}),
                    context,
                    request,
                );
            }
            bytes = cut(&state.stream.ready, bytes / 2);
        };
        state.stream.ready.drain(..bytes);
        state.emitted += bytes as u64;
        state.seen = Instant::now();
        if !state.stream.done || !state.stream.ready.is_empty() {
            let mut cache = self
                .cursors
                .lock()
                .map_err(|_| failed("page cache unavailable"))?;
            while cache.len() >= CURSORS {
                cache.pop_front();
            }
            cache.push_back((cursor, state));
        }
        // String values were redacted before JSON re-encoding. Applying generic
        // redaction to a JSON fragment here could corrupt its structural syntax.
        let mut report = ToolReport::text(text);
        report.synchronize();
        Ok(report)
    }
}
