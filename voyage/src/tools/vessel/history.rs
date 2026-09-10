//! Bounded conversation browsing and regex search over redacted canonical text.
use super::{ToolContext, ToolError, Value, VoyageCommand, failed, invalid, json, redact};
use crate::model::Role;
use regex::{Regex, RegexBuilder};
use uuid::Uuid;

const SCAN_MESSAGES: u32 = 32;
const SCAN_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn compile(pattern: &str) -> Result<Regex, ToolError> {
    if pattern.is_empty() || pattern.len() > 4096 {
        return Err(invalid("pattern must contain 1..4096 UTF-8 bytes"));
    }
    RegexBuilder::new(pattern)
        .size_limit(1024 * 1024)
        .dfa_size_limit(1024 * 1024)
        .nest_limit(64)
        .build()
        .map_err(|_| {
            invalid(
                "invalid or oversized Rust regex; look-around and backreferences are unsupported",
            )
        })
}
fn prefix(text: &str, bytes: usize) -> &str {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}
fn request(source: &Value, action: &str, revision: &Value) -> Value {
    let mut next =
        json!({"action":action,"session_id":source["session_id"],"expected_revision":revision});
    if let Some(target) = source.get("target") {
        next["target"] = target.clone();
    }
    next
}
fn message_read(source: &Value, revision: &Value, index: u64) -> Value {
    let mut next = request(source, "message", revision);
    next["index"] = json!(index);
    next
}

pub(super) fn page(mut value: Value, source: &Value, budget: usize) -> Value {
    let revision = value["revision"].clone();
    let offset = value["message_offset"].as_u64().unwrap_or(0);
    let old_more = value["has_more"] == true;
    let original = value["messages"].as_array().unwrap().clone();
    let mut count = original.len();
    let preview = (budget / 8).clamp(32, 768);
    value["messages"] = Value::Array(
        original
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let index = m["message_index"].as_u64().unwrap_or(offset + i as u64);
                let mut m = m.clone();
                m["read"] = message_read(source, &revision, index);
                m
            })
            .collect(),
    );
    let mut condensed = false;
    loop {
        let end = offset.saturating_add(count as u64);
        let more = old_more || count < original.len();
        value["next_offset"] = json!(end);
        value["has_more"] = json!(more);
        if more {
            let mut next = source.clone();
            next["offset"] = json!(end);
            next["expected_revision"] = revision.clone();
            value["next_read"] = next;
        } else {
            value.as_object_mut().unwrap().remove("next_read");
        }
        if offset > 0 {
            let mut previous = request(source, "history", &revision);
            previous["offset"] = json!(offset.saturating_sub(count.max(1) as u64));
            previous["limit"] = json!(count.max(1));
            value["previous_read"] = previous;
        }
        if value.to_string().len() <= budget || (count <= 1 && condensed) {
            return value;
        }
        if !condensed {
            for m in value["messages"].as_array_mut().unwrap() {
                if m.to_string().len() > (budget / 3).max(256) {
                    *m = json!({"message_index":m["read"]["index"],"role":m["role"],"created_at":m["created_at"],
                        "tool_call_id":m["tool_call_id"],"tool_success":m["tool_success"],"tool_outcome":m["tool_outcome"],
                        "content":prefix(m["content"].as_str().unwrap_or(""),preview),"projection_truncated":true,"read":m["read"],
                        "detail":"Excerpt; use read for the complete public message, including omitted fields."});
                }
            }
            condensed = true;
            continue;
        }
        value["messages"].as_array_mut().unwrap().pop();
        count -= 1;
    }
}

fn hit(message: &Value, regex: &Regex, index: u64) -> Option<Value> {
    let text = message["content"].as_str()?;
    let found = regex.find(text)?;
    let mut start = found.start().saturating_sub(128);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let excerpt = prefix(&text[start..], 512);
    Some(
        json!({"kind":"match","message_index":index,"role":message["role"],"created_at":message["created_at"],
        "match_start":found.start(),"match_end":found.end(),"excerpt_start":start,"excerpt":excerpt,
        "excerpt_truncated":start>0 || start+excerpt.len()<text.len(),"offset_unit":"redacted_content_utf8_bytes"}),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn search(
    t: &super::transport::Transport,
    session_id: Uuid,
    pattern: &str,
    role: Option<Role>,
    offset: u64,
    limit: u32,
    expected_revision: Option<u64>,
    context: &ToolContext,
) -> Result<Value, ToolError> {
    let regex = compile(pattern)?;
    if context.redactor.contains_secret(pattern)
        || t.redact_complete(json!(pattern)) != json!(pattern)
    {
        return Err(invalid(
            "pattern contains a configured secret; search redacted text instead",
        ));
    }
    let history = super::voyage(
        t,
        session_id,
        None,
        VoyageCommand::History {
            offset,
            limit: SCAN_MESSAGES,
            expected_revision,
        },
    )
    .await?;
    if history.get("status").is_some() {
        return Ok(history);
    }
    let revision = history["revision"]
        .as_u64()
        .ok_or_else(|| failed("history revision missing"))?;
    let total = history["total_messages"]
        .as_u64()
        .ok_or_else(|| failed("history total missing"))?;
    let messages = history["messages"]
        .as_array()
        .ok_or_else(|| failed("invalid history response"))?;
    if history["message_offset"].as_u64() != Some(offset) || offset > total {
        return Err(failed("history offset mismatch"));
    }
    if messages.is_empty() && offset < total {
        return Err(failed("history made no progress"));
    }
    let mut next = offset;
    let mut entries = Vec::new();
    let mut matched = 0;
    let mut bytes = 0;
    for (i, projected) in messages.iter().enumerate() {
        super::check(context)?;
        let index = offset
            .checked_add(i as u64)
            .ok_or_else(|| failed("history index overflow"))?;
        if index >= total
            || projected["message_index"]
                .as_u64()
                .is_some_and(|v| v != index)
        {
            return Err(failed("history index mismatch"));
        }
        next = index + 1;
        if role
            .as_ref()
            .is_some_and(|role| projected["role"] != json!(role))
        {
            continue;
        }
        let message = if projected["projection_truncated"] != false {
            let full =
                super::inspection::read_text(t, session_id, Some((index, revision)), None, context)
                    .await?;
            if full["status"] == "source_limit" {
                entries.push(json!({"kind":"unsearched","message_index":index,"reason":"source_limit",
                    "detail":"Full public message exceeds the 4 MiB safe-redaction source limit; its excerpt was not treated as a complete search."}));
                continue;
            }
            if full.get("status").is_some() {
                // A refusal or revision change invalidates the observation; do
                // not return earlier matches as though the scan completed.
                return Ok(full);
            }
            full["message"].clone()
        } else {
            projected.clone()
        };
        let message = redact(message, context)?;
        bytes += message["content"].as_str().map_or(0, str::len);
        if let Some(found) = hit(&message, &regex, index) {
            entries.push(found);
            matched += 1;
        }
        if matched >= limit || bytes >= SCAN_BYTES {
            break;
        }
    }
    Ok(
        json!({"session_id":session_id,"revision":revision,"offset":offset,"next_offset":next,
        "total_messages":total,"has_more":next<total,"entries":entries,
        "scope":"redacted message content; one result per matching message; no attachment bytes or structured tool-call arguments",
        "detail":"An empty page is not no matches when has_more is true. Accumulate unsearched entries across pages; any such entry prevents claiming a complete negative search."}),
    )
}

pub(super) fn search_page(mut value: Value, source: &Value, budget: usize) -> Value {
    let revision = value["revision"].clone();
    for entry in value["entries"].as_array_mut().unwrap() {
        let index = entry["message_index"].as_u64().unwrap();
        entry["read"] = message_read(source, &revision, index);
        let mut context = request(source, "history", &revision);
        context["offset"] = json!(index.saturating_sub(2));
        context["limit"] = json!(5);
        entry["context_read"] = context;
    }
    loop {
        let end = value["next_offset"].as_u64().unwrap();
        let more = end < value["total_messages"].as_u64().unwrap();
        value["has_more"] = json!(more);
        value["scanned_messages"] = json!(end.saturating_sub(value["offset"].as_u64().unwrap()));
        value["matched_messages"] = json!(
            value["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "match")
                .count()
        );
        value["unsearched_messages"] = json!(
            value["entries"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["kind"] == "unsearched")
                .count()
        );
        if more {
            let mut next = source.clone();
            next["offset"] = json!(end);
            next["expected_revision"] = revision.clone();
            value["next_read"] = next;
        } else {
            value.as_object_mut().unwrap().remove("next_read");
        }
        if value.to_string().len() <= budget || value["entries"].as_array().unwrap().len() <= 1 {
            return value;
        }
        let removed = value["entries"].as_array_mut().unwrap().pop().unwrap();
        // Rewind to the first withheld entry so pagination cannot lose a match.
        value["next_offset"] = removed["message_index"].clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regex_supports_flags_unicode_and_zero_width_but_rejects_unsafe_dialects() {
        for pattern in ["(?i)error|quota", "(?m)^α", "^", "", "(?=error)", r"(x)\1"] {
            let compiled = compile(pattern);
            if matches!(pattern, "" | "(?=error)" | r"(x)\1") {
                assert!(compiled.is_err());
            } else {
                assert!(
                    hit(
                        &json!({"content":"ERROR\nα🦀","role":"tool"}),
                        &compiled.unwrap(),
                        5
                    )
                    .is_some()
                );
            }
        }
        assert!(compile(&"x".repeat(4097)).is_err());
    }
    #[test]
    fn regex_excerpts_keep_valid_boundaries_and_exact_redacted_offsets() {
        let text = format!("{}ERROR{}", "🦀".repeat(100), "α".repeat(500));
        let result = hit(
            &json!({"content":text,"role":"tool"}),
            &compile("ERROR").unwrap(),
            3,
        )
        .unwrap();
        let start = result["excerpt_start"].as_u64().unwrap() as usize;
        let excerpt = result["excerpt"].as_str().unwrap();
        assert_eq!(result["match_start"], 400);
        assert_eq!(&text[start..start + excerpt.len()], excerpt);
        assert!(result["excerpt_truncated"].as_bool().unwrap());
        let redactor = crate::tools::Redactor::new(["secret-value".to_owned()]);
        let redacted = redactor.redact("secret-value");
        assert!(
            hit(
                &json!({"content":redacted}),
                &compile("secret-value").unwrap(),
                0
            )
            .is_none()
        );
    }
}
