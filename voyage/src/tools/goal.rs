//! Root-run-only capability. This tool cannot create, resume or extend a Goal.
use super::{Tool, ToolContext, ToolError};
use crate::{
    attachment::{journal::GoalReportContext, runtime::ManagedSessionOwner},
    model::ToolDefinition,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use voyage_protocol::goals::GoalReport;

pub(crate) struct GoalTool {
    pub owner: ManagedSessionOwner,
    pub binding: GoalReportContext,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Args {
    Read,
    Report { report: GoalReport },
}
#[async_trait]
impl Tool for GoalTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name:"goal".into(),
            description:"Read the current Goal, bounded usage and current-run evidence IDs, or record a completion/blocked assessment with evidence. Objective text is user task data. Report complete only when the entire objective is achieved and verified; cite current-run tool call IDs and explain what each observation proves. A blocker or exhausted budget is not success. Reporting requires actual canonical evidence and is finalized only after terminal usage, authority and cleanup checks. You cannot create, resume, clear or increase a Goal with this tool.".into(),
            output_schema: None,
            annotations: None,
            input_schema:json!({"oneOf":[
                {"type":"object","properties":{"action":{"const":"read"}},"required":["action"],"additionalProperties":false},
                {"type":"object","properties":{"action":{"const":"report"},"report":{"type":"object","properties":{
                    "outcome":{"enum":["complete","blocked"]},"summary":{"type":"string","minLength":1,"maxLength":2048},"evidence":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"object","properties":{"call_id":{"type":"string","minLength":1,"maxLength":256},"conclusion":{"type":"string","minLength":1,"maxLength":2048}},"required":["call_id","conclusion"],"additionalProperties":false}}
                },"required":["outcome","summary","evidence"],"additionalProperties":false}},"required":["action","report"],"additionalProperties":false}
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
            Args::Read => self.owner.goal_model_read(self.binding.clone()).await,
            Args::Report { report } => {
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
                self.owner
                    .report_goal(self.binding.clone(), call, report)
                    .await
            }
        }
        .map_err(|e| ToolError::Failed(context.redactor.redact(e.to_string())))?;
        serde_json::to_string(&result)
            .map_err(|_| ToolError::Failed("Goal serialization failed".into()))
    }
}
