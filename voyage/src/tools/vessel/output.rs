//! Bounded observations retain identity and explicit omissions, never imply cleanup.
use super::{ToolContext, ToolError, Value, failed, json, redact};
use crate::tools::ToolReport;
use voyage_protocol::tool_result::{ExecutionOutcome, IncompleteReason};

pub(super) fn output(
    value: Value,
    context: &ToolContext,
    request: &Value,
) -> Result<ToolReport, ToolError> {
    let value = redact(value, context)?;
    let encoded =
        serde_json::to_string(&value).map_err(|_| failed("Vessel result encoding failed"))?;
    if encoded.len() <= context.max_output_bytes {
        let mut report = ToolReport::text(encoded);
        if value["status"] == "outcome_unknown" {
            report.outcome.execution = ExecutionOutcome::Unknown;
        }
        report.synchronize();
        return Ok(report);
    }
    let action = request["action"].as_str().unwrap_or("");
    let mut limited = json!({"status":"output_limit","complete":false,
        "detail":"Output is incomplete. Missing state or cleanup details are unknown, not clear. Do not replay effects."});
    if action == "inspect" && value.get("snapshot").is_some() {
        let mut snapshot = value["snapshot"].clone();
        let mut omitted = Vec::new();
        // Preserve every other snapshot field. If that cannot fit, return no
        // partial authority-bearing snapshot rather than silently dropping blockers.
        for key in ["messages", "turns"] {
            if let Some(old) = snapshot.as_object_mut().and_then(|m| m.remove(key)) {
                omitted.push(
                    json!({"path":format!("/snapshot/{key}"),"count":old.as_array().map(Vec::len)}),
                );
            }
        }
        if let Some(run) = snapshot.get_mut("run").and_then(Value::as_object_mut) {
            for key in ["partial_text", "live_text"] {
                if let Some(old) = run.remove(key) {
                    omitted.push(json!({"path":format!("/snapshot/run/{key}"),"bytes":old.as_str().map(str::len)}));
                }
            }
        }
        limited["registration"] = value["registration"].clone();
        limited["snapshot"] = snapshot;
        limited["omitted"] = json!(omitted);
        let mut next =
            json!({"action":"history","session_id":request["session_id"],"offset":0,"limit":1});
        if let Some(revision) = value["snapshot"].get("revision") {
            next["expected_revision"] = revision.clone();
        }
        if let Some(target) = request.get("target") {
            next["target"] = target.clone();
        }
        limited["next_read"] = next;
        limited["detail"] = json!(
            "Transcript/live text omitted. Read history for persisted messages; live text has no paged native-tool action. Retained cleanup fields are observations, not permission to resume work."
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
        limited["next_read"] = next;
        limited["detail"] = json!(
            "Request one entry. If a single entry still exceeds the budget, increase the configured output budget; do not repeat effects."
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
