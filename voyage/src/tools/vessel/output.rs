//! Bounded observations retain identity and explicit omissions, never imply cleanup.
use super::{ToolContext, ToolError, Value, failed, json, redact};
use crate::tools::ToolReport;
use voyage_protocol::tool_result::{ExecutionOutcome, IncompleteReason};

pub(super) fn output(
    value: Value,
    context: &ToolContext,
    request: &Value,
) -> Result<ToolReport, ToolError> {
    let mut value = redact(value, context)?;
    let action = request["action"].as_str().unwrap_or("");
    if action == "message"
        && let Some(message) = value.as_object_mut().and_then(|v| v.remove("message"))
    {
        value["text"] =
            json!(serde_json::to_string(&message).map_err(|_| failed("message encoding failed"))?);
    }
    if value.get("snapshot").is_some() && matches!(action, "inspect" | "details") {
        let source = value;
        let mut limit = request["limit"].as_u64().unwrap_or(50) as usize;
        let mut bytes = super::inspection::preview_bytes();
        loop {
            value = if action == "inspect" {
                super::inspection::overview(&source, request, bytes)
            } else {
                super::inspection::details(&source, request, limit, bytes)?
            };
            if value.to_string().len() <= context.max_output_bytes || (limit == 1 && bytes <= 64) {
                break;
            }
            limit = (limit / 2).max(1);
            bytes = (bytes / 2).max(64);
        }
    } else if matches!(action, "message" | "run_output") && value.get("text").is_some() {
        let source = value;
        let mut bytes = 4096;
        loop {
            value = super::inspection::text_page(&source, request, bytes)?;
            if value.to_string().len() <= context.max_output_bytes || bytes <= 4 {
                break;
            }
            bytes = (bytes / 2).max(4);
        }
    } else if matches!(action, "follow" | "wait") && value.get("cursor").is_some() {
        let mut next = request.clone();
        if value["replay_gap"] == true {
            next = json!({"action":"inspect","session_id":request["session_id"]});
            if let Some(target) = request.get("target") {
                next["target"] = target.clone();
            }
        } else {
            next["after"] = value["cursor"].clone();
        }
        value["next_read"] = next;
    } else if action == "history" && value["messages"].is_array() {
        value = super::history::page(value, request, context.max_output_bytes);
    } else if action == "history_search" && value["entries"].is_array() {
        value = super::history::search_page(value, request, context.max_output_bytes);
    }

    let encoded =
        serde_json::to_string(&value).map_err(|_| failed("Vessel result encoding failed"))?;
    if encoded.len() <= context.max_output_bytes {
        let mut report = ToolReport::text(encoded);
        if value["status"] == "outcome_unknown" {
            report.outcome.execution = ExecutionOutcome::Unknown;
        }
        if matches!(
            value["status"].as_str(),
            Some("source_limit" | "output_limit")
        ) {
            report.outcome.incomplete = Some(IncompleteReason::OutputLimit);
        } else if matches!(
            value["status"].as_str(),
            Some(
                "cursor_expired"
                    | "cursor_mismatch"
                    | "cursor_required"
                    | "source_changed"
                    | "snapshot_unavailable"
                    | "revision_changed"
                    | "path_unavailable"
            )
        ) {
            report.outcome.incomplete = Some(IncompleteReason::Withheld);
        }
        if value["unsearched_messages"].as_u64().is_some_and(|n| n > 0) {
            report.outcome.incomplete = Some(IncompleteReason::Withheld);
        }
        report.synchronize();
        return Ok(report);
    }
    let action = request["action"].as_str().unwrap_or("");
    let mut limited = json!({"status":"output_limit","complete":false,
        "detail":"Output is incomplete. Missing state or cleanup details are unknown, not clear. Do not replay effects."});
    if action == "inspect" {
        let mut next = json!({"action":"details","session_id":request["session_id"],"limit":1});
        if let Some(target) = request.get("target") {
            next["target"] = target.clone();
        }
        limited["next_read"] = next;
        limited["detail"] = json!(
            "The output budget cannot fit this observation. Read individual public snapshot fields through details; missing cleanup and state remain unknown."
        );
    } else if matches!(action, "details" | "history_search") {
        limited["detail"] = json!(
            "The output budget cannot fit even one field or a small text chunk with its identity. Increase the configured output budget; repeating this read unchanged cannot provide details."
        );
    } else if let Some(id) = request.get("command_id") {
        limited["command_id"] = id.clone();
        limited["next_read"] = json!({"action":"receipt","command_id":id});
        if let Some(target) = request.get("target") {
            limited["next_read"]["target"] = target.clone();
        }
        limited["detail"] = json!(
            "Read the retained command receipt. A large receipt may also exceed the budget; do not repeat the mutation. Command admission is not independent-work completion."
        );
    } else if matches!(
        action,
        "history" | "list" | "search" | "operations" | "follow" | "wait"
    ) {
        let mut next = request.clone();
        next["limit"] = json!(1);
        if action == "history"
            && let Some(revision) = value.get("revision")
        {
            next["expected_revision"] = revision.clone();
        }
        if action == "history"
            && request["limit"] == 1
            && let Some(message_read) = value["messages"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|m| m.get("read"))
        {
            next = message_read.clone();
        }
        limited["next_read"] = next;
        limited["detail"] = json!(
            "Follow next_read for a smaller page or full-message chunks. For non-history single entries that still exceed the budget, increase the configured output budget; do not repeat effects."
        );
    } else {
        limited["detail"] = json!(
            "This action has no smaller-page option. Increase the configured output budget; missing details remain unknown."
        );
    }
    let encoded = serde_json::to_string(&redact(limited, context)?)
        .map_err(|_| failed("Vessel result encoding failed"))?;
    let text = if encoded.len() <= context.max_output_bytes {
        encoded
    } else {
        // Never return a malformed JSON prefix or a stripped snapshot suggesting
        // that all outstanding resources were observed.
        let tiny = json!({"status":"output_limit","complete":false,"detail":"Budget cannot fit identity and state safely. Increase output budget; do not replay effects."}).to_string();
        if tiny.len() <= context.max_output_bytes {
            tiny
        } else {
            String::new()
        }
    };
    let mut report = ToolReport::text(text);
    report.outcome.incomplete = Some(IncompleteReason::OutputLimit);
    if value["status"] == "outcome_unknown" {
        report.outcome.execution = ExecutionOutcome::Unknown;
    }
    report.synchronize();
    Ok(report)
}
