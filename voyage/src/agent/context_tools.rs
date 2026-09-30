//! Fulfilled only by the active agent; registry declarations convey no execution owner.
use super::*;
use crate::tools::{ContextToolKind, ToolError, ToolReport};
use serde_json::json;

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn fulfill_context_tool(
        &self,
        kind: ContextToolKind,
        call: &crate::model::ToolCall,
        context: &ToolContext,
        history: &[Message],
        working: &mut crate::context::WorkingContext,
        last_observation: Option<&voyage_protocol::context_accounting::ContextObservation>,
        checkpoint: Option<&dyn RunCheckpoint>,
        cancel: &CancellationToken,
    ) -> Result<Result<ToolReport, ToolError>, AgentError> {
        if let Err(error) = self
            .tools
            .validate_context_call(&call.name, &call.arguments, context)
        {
            return Ok(Err(error));
        }
        let report = match kind {
            ContextToolKind::Status => {
                if call.arguments["action"] == "read_receipt" {
                    let Some(anchor) = call.arguments["message_index"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                    else {
                        return Ok(Err(ToolError::InvalidArguments(
                            "read_receipt needs a message_index".into(),
                        )));
                    };
                    let Some(id) = call.arguments["call_id"].as_str() else {
                        return Ok(Err(ToolError::InvalidArguments(
                            "read_receipt needs a call_id".into(),
                        )));
                    };
                    let receipt = working
                        .model_receipt(history, anchor, id)
                        .map_err(|_| CheckpointError)?;
                    ToolReport::text(
                        json!({"scope":"durable_internal_context_receipt","receipt":receipt})
                            .to_string(),
                    )
                } else if call.arguments["action"] == "read_history" {
                    let index = match call.arguments["message_index"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                    {
                        Some(index) => index,
                        None => {
                            return Ok(Err(ToolError::InvalidArguments(
                                "read_history needs a message_index".into(),
                            )));
                        }
                    };
                    let message = match history.get(index) {
                        Some(message) if message.role != crate::model::Role::System => message,
                        _ => {
                            return Ok(Err(ToolError::InvalidArguments(
                                "canonical message unavailable".into(),
                            )));
                        }
                    };
                    let text = if call.arguments["part"] == "tool_calls" {
                        serde_json::to_string(&message.tool_calls).map_err(|_| CheckpointError)?
                    } else {
                        message.content.clone()
                    };
                    let text = context.redactor.redact(text);
                    let offset = call.arguments["offset"]
                        .as_u64()
                        .unwrap_or(0)
                        .min(usize::MAX as u64) as usize;
                    let limit = call.arguments["limit"].as_u64().unwrap_or(1024).min(4096) as usize;
                    let limit = limit.min(context.max_output_bytes.saturating_sub(768) / 6);
                    let total = text.chars().count();
                    let chunk = text.chars().skip(offset).take(limit).collect::<String>();
                    ToolReport::text(json!({"scope":"canonical_authored_history","message_index":index,"role":message.role,"tool_call_id":message.tool_call_id,"part":call.arguments["part"].as_str().unwrap_or("text"),"offset":offset,"text":chunk,"total_characters":total,"next_offset":(offset+limit<total).then_some(offset+limit)}).to_string())
                } else {
                    ToolReport::text(json!({"projection_generation":working.generation,"projection_reason":working.reason,"last_prepared_request":last_observation.map(|o|json!({"model":o.count.scope.model,"precision":o.count.precision,"method":o.count.method,"input_tokens":o.count.reliable_input_tokens(),"enabled_capacity":o.capacity.as_ref().and_then(|c|c.enabled_window_tokens),"output_reserve":o.output_reserve,"observed_at_ms":o.count.observed_at_ms,"projection_generation":o.projection_generation})),"current_input_tokens":null,"current_remaining_tokens":null,"accounting_scope":"current history changed after the last prepared request; occupancy unknown","canonical_messages":history.len(),"compaction_possible":checkpoint.is_some(),"model_notes":working.model_note_count(),"continuity":"Use read_history to recover exact canonical evidence; completed tools are never replayed"}).to_string())
                }
            }
            ContextToolKind::Compact => {
                if context.max_output_bytes < 512 {
                    return Ok(Err(ToolError::Denied(
                        "output budget cannot retain an exact context receipt".into(),
                    )));
                }
                let Some(checkpoint) = checkpoint else {
                    return Ok(Ok(ToolReport::text(json!({"state":"refused","reason":"durable_projection_checkpoint_unavailable","generation":working.generation}).to_string())));
                };
                let retain = call.arguments["retain_recent"]
                    .as_u64()
                    .unwrap_or(4)
                    .min(1024) as usize;
                let notes = call.arguments["carry_forward"]
                    .as_str()
                    .map(|s| context.redactor.redact(s));
                if notes
                    .as_ref()
                    .is_some_and(|s| s.len() > 8192 || s.contains('\0'))
                {
                    return Ok(Err(ToolError::InvalidArguments(
                        "carry_forward notes exceed bounds or contain invalid data".into(),
                    )));
                }
                let (candidate,receipt)=match working.request_model_compaction(history,call,context.execution_id,retain,notes) {
                    Ok(value)=>value,Err(_)=>return Ok(Ok(ToolReport::text(json!({"state":"refused","reason":"context_receipt_or_note_bounds","generation":working.generation}).to_string()))),
                };
                if cancel.is_cancelled() {
                    return Err(AgentError::Cancelled);
                }
                gate::guarded(
                    tokio::time::timeout(
                        context.timeout,
                        checkpoint.save_working_context(&candidate),
                    ),
                    cancel,
                )
                .await?
                .map_err(|_| CheckpointError)??;
                *working = candidate;
                ToolReport::text(receipt.to_string())
            }
        };
        Ok(Ok(self.tools.bound_context_report(report, context)))
    }
}
