//! Explicit runtime adoption and evidence-bound accounting; no automatic status edits.
use super::{DispositionKind, Obligation};
use crate::{
    model::ToolDefinition,
    subagent::{AgentId, AgentTreeStore},
    todo::{TodoId, TodoStore},
    tools::{Tool, ToolContext, ToolError},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct CompletionTool {
    todos: Arc<TodoStore>,
    agents: AgentTreeStore,
}
impl CompletionTool {
    pub fn new(todos: Arc<TodoStore>, agents: AgentTreeStore) -> Self {
        Self { todos, agents }
    }
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Args {
    Snapshot {},
    Read {
        kind: Kind,
        id: Uuid,
    },
    Adopt {
        kind: Kind,
        id: Uuid,
        revision: u64,
    },
    Account {
        kind: Kind,
        id: Uuid,
        revision: u64,
        fingerprint: String,
        disposition: DispositionKind,
        reason: String,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Todo,
    Agent,
}
impl Kind {
    fn obligation(self, id: Uuid) -> Obligation {
        match self {
            Self::Todo => Obligation::Todo(TodoId(id)),
            Self::Agent => Obligation::Agent(AgentId(id)),
        }
    }
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
#[async_trait]
impl Tool for CompletionTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition { name: "completion".into(), description: "Review only this run's obligations. Snapshot returns unresolved IDs and revision. Read owned evidence/results; adopt older todos or complete terminal agent subtrees explicitly; wait or cancel active work before adoption. Account against fresh revision with evidence or truthful blocked/deferred/failure impact; this does not complete, delete, archive, or cancel work. Mechanical accounting does not prove semantic correctness.".into(), input_schema: json!({"type":"object","required":["action"],"properties":{"action":{"enum":["snapshot","read","adopt","account"]},"kind":{"enum":["todo","agent"]},"id":{"type":"string","format":"uuid"},"revision":{"type":"integer","minimum":0},"disposition":{"enum":["completed_with_evidence","cancelled_with_reason","blocked_with_impact","deferred_with_impact","incorporated","failure_with_impact","not_needed_with_reason"]},"reason":{"type":"string"},"fingerprint":{"type":"string"}},"additionalProperties":false}) }
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<String, ToolError> {
        tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => Err(ToolError::Cancelled),
            result = tokio::time::timeout(context.timeout, self.execute_operation(args, context)) => result.unwrap_or(Err(ToolError::Timeout(context.timeout))),
        }
    }
}

impl CompletionTool {
    async fn execute_operation(
        &self,
        args: Value,
        context: &ToolContext,
    ) -> Result<String, ToolError> {
        if context.cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let args: Args =
            serde_json::from_value(args).map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let run = context
            .completion
            .as_ref()
            .ok_or_else(|| failed("no completion run is attached"))?;
        let value = match args {
            Args::Snapshot {} => serde_json::to_value(
                run.snapshot(&self.todos, &self.agents, 64)
                    .await
                    .map_err(failed)?,
            )
            .map_err(failed)?,
            Args::Read { kind, id } => run
                .read_owned(&self.todos, &self.agents, kind.obligation(id))
                .await
                .map_err(failed)?,
            Args::Adopt { kind, id, revision } => {
                run.adopt_existing(&self.todos, &self.agents, kind.obligation(id), revision)
                    .await
                    .map_err(failed)?;
                json!({"adopted":id})
            }
            Args::Account {
                kind,
                id,
                revision,
                disposition,
                fingerprint,
                reason,
            } => {
                run.account(
                    &self.todos,
                    &self.agents,
                    kind.obligation(id),
                    super::runtime::Review {
                        revision,
                        fingerprint,
                        disposition,
                        reason: context.redactor.redact(reason),
                    },
                )
                .await
                .map_err(failed)?;
                json!({"accounted":id})
            }
        };
        let text = serde_json::to_string(&value).map_err(failed)?;
        Ok(crate::tools::truncate(
            text.into_bytes(),
            context.max_output_bytes.min(32768),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        completion::runtime::{Coordinator, RunHandle},
        config::Config,
        policy::Policy,
        todo::{NewTodo, Priority, TodoScope},
        tools::{InteractionMode, Redactor, ToolRegistry, UnattendedApprover},
    };
    #[tokio::test]
    async fn native_tool_rejects_unowned_reads_and_redacts_durable_reviews() {
        let root = tempfile::tempdir().unwrap();
        let coordinator = Coordinator::open(root.path().join("completion"), root.path()).unwrap();
        let run = RunHandle::create(coordinator.clone(), Uuid::new_v4(), Uuid::new_v4())
            .await
            .unwrap();
        let todos = Arc::new(
            TodoStore::new(
                root.path().join("todos/list.json"),
                TodoScope::workspace(root.path().canonicalize().unwrap()),
            )
            .with_coordinator(coordinator.clone()),
        );
        let agents =
            AgentTreeStore::new(root.path().join("agents/tree.json")).with_coordinator(coordinator);
        let item = todos
            .create(NewTodo {
                title: "legacy fixture".into(),
                description: String::new(),
                priority: Priority::Normal,
                order: None,
                assignees: Default::default(),
            })
            .await
            .unwrap();
        let mut registry = ToolRegistry::default();
        registry.register(CompletionTool::new(todos.clone(), agents.clone()));
        let context = ToolContext {
            completion: Some(run.clone()),
            policy: Arc::new(Policy::new(&Config::default(), root.path().to_owned()).unwrap()),
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: std::time::Duration::from_secs(3),
            max_output_bytes: 4096,
            environment: Default::default(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: run.run_id(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::new(["fixture-sensitive-secret".into()])),
        };
        assert!(
            registry
                .execute(
                    "completion",
                    json!({"action":"read","kind":"todo","id":item.id}),
                    &context
                )
                .await
                .is_err()
        );
        registry
            .execute(
                "completion",
                json!({"action":"adopt","kind":"todo","id":item.id,"revision":0}),
                &context,
            )
            .await
            .unwrap();
        let snapshot: Value = serde_json::from_str(
            &registry
                .execute("completion", json!({"action":"snapshot"}), &context)
                .await
                .unwrap(),
        )
        .unwrap();
        registry.execute("completion",json!({"action":"account","kind":"todo","id":item.id,"revision":snapshot["revision"],"fingerprint":snapshot["fingerprint"],"disposition":"deferred_with_impact","reason":"waiting for fixture-sensitive-secret"}),&context).await.unwrap();
        assert!(run.snapshot(&todos, &agents, 64).await.unwrap().ready());
        assert_eq!(
            todos.snapshot().await.unwrap().items[&item.id].status,
            crate::todo::TodoStatus::Pending
        );
        let reference = run.reference();
        let bytes = std::fs::read_to_string(root.path().join("completion/ledgers").join(format!(
            "{}-{}.json",
            reference.session_id, reference.run_id
        )))
        .unwrap();
        assert!(!bytes.contains("fixture-sensitive-secret"));
        assert!(bytes.contains("[REDACTED]"));
        assert!(
            registry
                .execute(
                    "completion",
                    json!({"action":"snapshot","unexpected":true}),
                    &context
                )
                .await
                .is_err()
        );
        context.cancellation.cancel();
        assert!(matches!(
            registry
                .execute("completion", json!({"action":"snapshot"}), &context)
                .await,
            Err(ToolError::Cancelled)
        ));
    }
}

#[cfg(test)]
mod schema_tests;
