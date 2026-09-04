use super::{AgentBudget, AgentId, AgentPolicy, SpawnRequest, SubagentRuntime};
use crate::{
    model::ToolDefinition,
    tools::{Tool, ToolContext, ToolError},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Model-facing control surface. Policy and budgets are supplied by Helm, never by the model.
#[derive(Clone)]
pub struct SubagentTool {
    runtime: Arc<SubagentRuntime>,
    policy: AgentPolicy,
    budget: AgentBudget,
}
impl SubagentTool {
    pub fn new(runtime: Arc<SubagentRuntime>, policy: AgentPolicy, budget: AgentBudget) -> Self {
        Self {
            runtime,
            policy,
            budget,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Args {
    Spawn {
        name: String,
        task: String,
        #[serde(default)]
        parent_id: Option<Uuid>,
    },
    Status {
        id: Uuid,
    },
    List {
        #[serde(default)]
        parent_id: Option<Uuid>,
    },
    Wait {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
    },
    Message {
        id: Uuid,
        message: String,
    },
    FollowUp {
        id: Uuid,
        message: String,
    },
}

#[async_trait]
impl Tool for SubagentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
        name:"subagent".into(), description:"Spawn and supervise bounded background Helm agents. Actions: spawn, status, list, wait, cancel, message, follow_up.".into(),
        input_schema:json!({"type":"object","required":["action"],"properties":{"action":{"enum":["spawn","status","list","wait","cancel","message","follow_up"]},"id":{"type":"string","format":"uuid"},"parent_id":{"type":"string","format":"uuid"},"name":{"type":"string"},"task":{"type":"string"},"message":{"type":"string"}},"additionalProperties":false}),
    }
    }
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let value = match args {
            Args::Spawn {
                name,
                task,
                parent_id,
            } => {
                let id = self
                    .runtime
                    .spawn(SpawnRequest {
                        parent_id: parent_id.map(AgentId),
                        name,
                        task,
                        policy: self.policy.clone(),
                        budget: self.budget.clone(),
                        worktree: None,
                        branch: None,
                    })
                    .await
                    .map_err(failed)?;
                json!({"id":id,"status":"queued"})
            }
            Args::Status { id } => {
                serde_json::to_value(self.runtime.get(AgentId(id)).await.map_err(failed)?)
                    .map_err(json_error)?
            }
            Args::List { parent_id } => serde_json::to_value(match parent_id {
                Some(id) => self.runtime.tree(Some(AgentId(id))).await,
                None => self.runtime.list().await,
            })
            .map_err(json_error)?,
            Args::Wait { id } => {
                let outcome = tokio::select! {_ = context.cancellation.cancelled()=>return Err(ToolError::Cancelled), value=self.runtime.wait(AgentId(id))=>value.map_err(failed)?};
                match outcome {
                    Ok(result) => json!({"status":"completed","result":result}),
                    Err(error) => json!({"status":"failed","error":error}),
                }
            }
            Args::Cancel { id } => {
                self.runtime.cancel(AgentId(id)).await.map_err(failed)?;
                json!({"id":id,"cancel_requested":true})
            }
            Args::Message { id, message } => {
                self.runtime
                    .send_message(AgentId(id), message)
                    .await
                    .map_err(failed)?;
                json!({"id":id,"queued":true})
            }
            Args::FollowUp { id, message } => {
                let follow_up_id = self
                    .runtime
                    .follow_up(AgentId(id), message)
                    .await
                    .map_err(failed)?;
                json!({"id":follow_up_id,"queued":true})
            }
        };
        serde_json::to_string(&value).map_err(json_error)
    }
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
fn json_error(error: serde_json::Error) -> ToolError {
    ToolError::Failed(error.to_string())
}
