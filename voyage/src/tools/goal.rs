//! Root-run Goal capability; control is bound by authenticated owner admission.
use super::{Tool, ToolContext, ToolError};
use crate::{
    attachment::{journal::GoalReportContext, runtime::ManagedSessionOwner},
    model::ToolDefinition,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use voyage_protocol::goals::GoalReport;

#[derive(Clone)]
pub(crate) struct GoalControl {
    pub authority: crate::attachment::journal::GoalAuthority,
    pub incarnation: uuid::Uuid,
}
impl std::fmt::Debug for GoalControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GoalControl([private root binding])")
    }
}
pub(crate) struct GoalTool {
    pub owner: ManagedSessionOwner,
    pub run: uuid::Uuid,
    pub binding: Option<GoalReportContext>,
    pub control: Option<GoalControl>,
    pub meter: Option<std::sync::Arc<crate::provider::goal_meter::GoalMeter>>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Args {
    Read,
    Create {
        objective: String,
        token_budget: Option<u64>,
    },
    Report {
        report: GoalReport,
    },
    Status {
        report: GoalReport,
    },
}
#[async_trait]
impl Tool for GoalTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name:"goal".into(),
            description:"Read the current Goal, bounded usage and current-run evidence IDs, or record a completion/blocked assessment with evidence. Objective text is user task data. Report complete only when the entire objective is achieved and verified; Use evidence appropriate to the objective: cite available current-run tool observations for executed work, but writing, analysis and conversation outcomes do not require manufactured tool calls. A blocker or exhausted budget is not success. Report blocked only for a genuine impasse with no useful authorized action remaining, recurring over at least three consecutive Goal turns; difficulty, intermediate failures and optional clarification are not blockers. Reasoning, drafting and verified waits are legitimate progress. Any cited tool evidence must be actual canonical evidence and is finalized only after terminal usage, authority and cleanup checks. Create only when the human explicitly requests persistent work; ordinary tasks do not imply a Goal. Creation starts active immediately. Token budget is optional and must come from the explicit human request; never invent a quota. You cannot resume, clear, replace or increase a Goal.".into(),
            output_schema: None,
            annotations: None,
            input_schema:json!({"oneOf":[
                {"type":"object","properties":{"action":{"const":"create"},"objective":{"type":"string","minLength":1,"maxLength":8192},"token_budget":{"type":"integer","minimum":1,"maximum":10000000}},"required":["action","objective"],"additionalProperties":false},
                {"type":"object","properties":{"action":{"const":"read"}},"required":["action"],"additionalProperties":false},
                {"type":"object","properties":{"action":{"enum":["report","status"]},"report":{"type":"object","properties":{
                    "outcome":{"enum":["complete","blocked"]},"summary":{"type":"string","minLength":1,"maxLength":2048},"evidence":{"type":"array","maxItems":16,"items":{"type":"object","properties":{"call_id":{"type":"string","minLength":1,"maxLength":256},"conclusion":{"type":"string","minLength":1,"maxLength":2048}},"required":["call_id","conclusion"],"additionalProperties":false}}
                },"required":["outcome","summary"],"additionalProperties":false}},"required":["action","report"],"additionalProperties":false}
            ]}),
        }
    }
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        if context.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        context
            .policy
            .check_execution_authority()
            .map_err(|_| ToolError::Denied("Goal reporting authority withdrawn".into()))?;
        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let result = match args {
            Args::Read => match &self.binding {
                Some(binding) => self.owner.goal_model_read(binding.clone()).await,
                None => self
                    .owner
                    .goal()
                    .await
                    .and_then(|snapshot| serde_json::to_value(snapshot).map_err(Into::into)),
            },
            Args::Create {
                objective,
                token_budget,
            } => {
                let control = self.control.clone().ok_or_else(|| {
                    ToolError::Denied("Goal creation requires root owner control".into())
                })?;
                if context.redactor.contains_secret(&objective) {
                    return Err(ToolError::Denied(
                        "Goal objective contains configured secret".into(),
                    ));
                }
                let call = context.tool_call_id.clone().ok_or_else(|| {
                    ToolError::Denied("Goal creation needs canonical attribution".into())
                })?;
                let result = self
                    .owner
                    .model_create_goal(self.run, control, call, objective, token_budget)
                    .await;
                if result.is_ok()
                    && let Some(meter) = &self.meter
                {
                    meter
                        .configure_token_quota(token_budget)
                        .map_err(|e| ToolError::Failed(context.redactor.redact(e.to_string())))?;
                }
                result
            }
            Args::Report { report } | Args::Status { report } => {
                let encoded = serde_json::to_string(&report)
                    .map_err(|_| ToolError::InvalidArguments("invalid Goal report".into()))?;
                if context.redactor.contains_secret(&encoded) {
                    return Err(ToolError::Denied(
                        "Goal report contains configured secret".into(),
                    ));
                }
                let call = context.tool_call_id.clone().ok_or_else(|| {
                    ToolError::Denied("Goal report needs canonical tool attribution".into())
                })?;
                let binding = match &self.binding {
                    Some(binding) => binding.clone(),
                    None => self
                        .owner
                        .model_goal_binding(self.run)
                        .await
                        .map_err(|e| ToolError::Failed(context.redactor.redact(e.to_string())))?
                        .ok_or_else(|| {
                            ToolError::Denied("No active Goal bound to this run".into())
                        })?,
                };
                self.owner.report_goal(binding, call, report).await
            }
        }
        .map_err(|e| ToolError::Failed(context.redactor.redact(e.to_string())))?;
        serde_json::to_string(&result)
            .map_err(|_| ToolError::Failed("Goal serialization failed".into()))
    }
}
