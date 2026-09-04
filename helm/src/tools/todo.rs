use super::{Tool, ToolContext, ToolError};
use crate::{
    model::ToolDefinition,
    todo::{EntryKind, NewTodo, Priority, TodoId, TodoStatus, TodoStore},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Arc};
use uuid::Uuid;

#[derive(Clone)]
pub struct TodoTool {
    store: Arc<TodoStore>,
}
impl TodoTool {
    pub fn new(store: Arc<TodoStore>) -> Self {
        Self { store }
    }
    pub fn store(&self) -> Arc<TodoStore> {
        self.store.clone()
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Args {
    Create {
        title: String,
        #[serde(default)]
        description: String,
        #[serde(default)]
        priority: PriorityArg,
        #[serde(default)]
        order: Option<i64>,
        #[serde(default)]
        assignees: Vec<String>,
    },
    List {
        #[serde(default)]
        include_archived: bool,
        #[serde(default)]
        status: Option<StatusArg>,
    },
    Edit {
        id: Uuid,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        priority: Option<PriorityArg>,
    },
    Status {
        id: Uuid,
        status: StatusArg,
    },
    Block {
        id: Uuid,
        #[serde(default)]
        blockers: Vec<String>,
    },
    Dependencies {
        id: Uuid,
        #[serde(default)]
        add: Vec<Uuid>,
        #[serde(default)]
        remove: Vec<Uuid>,
    },
    Assign {
        id: Uuid,
        #[serde(default)]
        assignees: Vec<String>,
    },
    Note {
        id: Uuid,
        text: String,
        #[serde(default)]
        author: Option<String>,
    },
    Progress {
        id: Uuid,
        text: String,
        #[serde(default)]
        author: Option<String>,
    },
    Evidence {
        id: Uuid,
        text: String,
        #[serde(default)]
        author: Option<String>,
    },
    Reorder {
        id: Uuid,
        order: i64,
    },
    Remove {
        id: Uuid,
    },
    Archive {
        id: Uuid,
    },
    ClearCompleted,
}
#[derive(Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum PriorityArg {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StatusArg {
    Pending,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}
impl From<PriorityArg> for Priority {
    fn from(v: PriorityArg) -> Self {
        match v {
            PriorityArg::Low => Self::Low,
            PriorityArg::Normal => Self::Normal,
            PriorityArg::High => Self::High,
            PriorityArg::Critical => Self::Critical,
        }
    }
}
impl From<StatusArg> for TodoStatus {
    fn from(v: StatusArg) -> Self {
        match v {
            StatusArg::Pending => Self::Pending,
            StatusArg::InProgress => Self::InProgress,
            StatusArg::Blocked => Self::Blocked,
            StatusArg::Completed => Self::Completed,
            StatusArg::Cancelled => Self::Cancelled,
        }
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition{name:"todo".into(),description:"Manage Helm's durable workspace task list: create/list/edit/status/block/dependencies/assign/note/progress/evidence/reorder/remove/archive/clear_completed. Set status to pending to reopen work.".into(),input_schema:json!({"type":"object","required":["action"],"properties":{"action":{"enum":["create","list","edit","status","block","dependencies","assign","note","progress","evidence","reorder","remove","archive","clear_completed"]},"id":{"type":"string","format":"uuid"},"title":{"type":"string"},"description":{"type":"string"},"priority":{"enum":["low","normal","high","critical"]},"status":{"enum":["pending","in_progress","blocked","completed","cancelled"]},"order":{"type":"integer"},"assignees":{"type":"array","items":{"type":"string"}},"blockers":{"type":"array","items":{"type":"string"}},"add":{"type":"array","items":{"type":"string","format":"uuid"}},"remove":{"type":"array","items":{"type":"string","format":"uuid"}},"text":{"type":"string"},"author":{"type":"string"},"include_archived":{"type":"boolean"}},"additionalProperties":false})}
    }
    async fn execute(&self, args: Value, _: &ToolContext) -> Result<String, ToolError> {
        let args: Args = serde_json::from_value(args).map_err(invalid)?;
        let value = match args {
            Args::Create {
                title,
                description,
                priority,
                order,
                assignees,
            } => serde_json::to_value(
                self.store
                    .create(NewTodo {
                        title,
                        description,
                        priority: priority.into(),
                        order,
                        assignees: clean_set(assignees),
                    })
                    .await
                    .map_err(failed)?,
            ),
            Args::List {
                include_archived,
                status,
            } => {
                let list = self.store.snapshot().await.map_err(failed)?;
                let wanted = status.map(TodoStatus::from);
                let mut items: Vec<_> = list
                    .items
                    .into_values()
                    .filter(|item| {
                        (include_archived || !item.archived())
                            && wanted.is_none_or(|s| item.status == s)
                    })
                    .collect();
                items.sort_by_key(|item| {
                    (
                        item.order,
                        std::cmp::Reverse(item.priority),
                        item.created_at,
                    )
                });
                serde_json::to_value(json!({"revision":list.revision,"items":items}))
            }
            Args::Edit {
                id,
                title,
                description,
                priority,
            } => serde_json::to_value(
                self.store
                    .edit(TodoId(id), title, description, priority.map(Into::into))
                    .await
                    .map_err(failed)?,
            ),
            Args::Status { id, status } => serde_json::to_value(
                self.store
                    .set_status(TodoId(id), status.into())
                    .await
                    .map_err(failed)?,
            ),
            Args::Block { id, blockers } => serde_json::to_value(
                self.store
                    .set_blockers(TodoId(id), blockers)
                    .await
                    .map_err(failed)?,
            ),
            Args::Dependencies { id, add, remove } => {
                let add = add.into_iter().map(TodoId).collect();
                let remove = remove.into_iter().map(TodoId).collect();
                serde_json::to_value(
                    self.store
                        .update_dependencies(TodoId(id), add, remove)
                        .await
                        .map_err(failed)?,
                )
            }
            Args::Assign { id, assignees } => serde_json::to_value(
                self.store
                    .assign(TodoId(id), clean_set(assignees))
                    .await
                    .map_err(failed)?,
            ),
            Args::Progress { id, text, author } => serde_json::to_value(
                self.store
                    .append_note(TodoId(id), EntryKind::Progress, text, author)
                    .await
                    .map_err(failed)?,
            ),
            Args::Note { id, text, author } => serde_json::to_value(
                self.store
                    .append_note(TodoId(id), EntryKind::Note, text, author)
                    .await
                    .map_err(failed)?,
            ),
            Args::Evidence { id, text, author } => serde_json::to_value(
                self.store
                    .append_note(TodoId(id), EntryKind::Evidence, text, author)
                    .await
                    .map_err(failed)?,
            ),
            Args::Reorder { id, order } => serde_json::to_value(
                self.store
                    .reorder(TodoId(id), order)
                    .await
                    .map_err(failed)?,
            ),
            Args::Remove { id } => {
                self.store.remove(TodoId(id)).await.map_err(failed)?;
                Ok(json!({"removed":id}))
            }
            Args::Archive { id } => {
                serde_json::to_value(self.store.archive(TodoId(id)).await.map_err(failed)?)
            }
            Args::ClearCompleted => {
                Ok(json!({"archived":self.store.clear_completed().await.map_err(failed)?}))
            }
        }
        .map_err(|e| ToolError::Failed(e.to_string()))?;
        serde_json::to_string(&value).map_err(|e| ToolError::Failed(e.to_string()))
    }
}
fn clean_set(values: Vec<String>) -> BTreeSet<String> {
    values
        .into_iter()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect()
}
fn invalid(error: serde_json::Error) -> ToolError {
    ToolError::InvalidArguments(error.to_string())
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        policy::Policy,
        tools::{InteractionMode, Redactor, UnattendedApprover},
    };
    use std::{collections::BTreeMap, time::Duration};

    fn context(directory: &tempfile::TempDir) -> ToolContext {
        ToolContext {
            policy: Arc::new(
                Policy::new(&Config::default(), directory.path().to_path_buf()).unwrap(),
            ),
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: Duration::from_secs(1),
            max_output_bytes: 4096,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::default()),
        }
    }
    async fn call(tool: &TodoTool, context: &ToolContext, value: Value) -> Value {
        serde_json::from_str(&tool.execute(value, context).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn structured_actions_mutate_one_durable_store() {
        let directory = tempfile::tempdir().unwrap();
        let scope = crate::todo::TodoScope {
            workspace: directory.path().into(),
            session_id: None,
        };
        let tool = TodoTool::new(Arc::new(TodoStore::new(
            directory.path().join("todos.json"),
            scope,
        )));
        let context = context(&directory);
        let first = call(
            &tool,
            &context,
            json!({"action":"create","title":"inspect","priority":"high"}),
        )
        .await;
        let first_id = first["id"].as_str().unwrap();
        let second = call(&tool, &context, json!({"action":"create","title":"report"})).await;
        let second_id = second["id"].as_str().unwrap();
        call(&tool,&context,json!({"action":"edit","id":second_id,"description":"write results","priority":"critical"})).await;
        call(
            &tool,
            &context,
            json!({"action":"dependencies","id":second_id,"add":[first_id]}),
        )
        .await;
        let before = tool.store.snapshot().await.unwrap();
        assert!(tool
            .execute(
                json!({"action":"dependencies","id":second_id,"remove":[first_id],"add":[second_id]}),
                &context,
            )
            .await
            .is_err());
        let after = tool.store.snapshot().await.unwrap();
        assert_eq!(after.revision, before.revision);
        let second_uuid = TodoId(Uuid::parse_str(second_id).unwrap());
        let first_uuid = TodoId(Uuid::parse_str(first_id).unwrap());
        assert!(after.items[&second_uuid].dependencies.contains(&first_uuid));
        assert!(
            tool.execute(
                json!({"action":"status","id":second_id,"status":"in_progress"}),
                &context
            )
            .await
            .is_err()
        );
        call(
            &tool,
            &context,
            json!({"action":"note","id":first_id,"text":"operator context","author":"helm"}),
        )
        .await;
        call(
            &tool,
            &context,
            json!({"action":"progress","id":first_id,"text":"inspected inventory"}),
        )
        .await;
        call(
            &tool,
            &context,
            json!({"action":"evidence","id":first_id,"text":"inventory.txt:1"}),
        )
        .await;
        call(
            &tool,
            &context,
            json!({"action":"status","id":first_id,"status":"completed"}),
        )
        .await;
        call(
            &tool,
            &context,
            json!({"action":"status","id":second_id,"status":"in_progress"}),
        )
        .await;
        call(
            &tool,
            &context,
            json!({"action":"assign","id":second_id,"assignees":["agent-1"]}),
        )
        .await;
        call(
            &tool,
            &context,
            json!({"action":"block","id":second_id,"blockers":["approval"]}),
        )
        .await;
        let listed = call(&tool, &context, json!({"action":"list"})).await;
        assert_eq!(listed["items"].as_array().unwrap().len(), 2);
        assert_eq!(listed["items"][0]["notes"][0]["text"], "operator context");
        let cleared = call(&tool, &context, json!({"action":"clear_completed"})).await;
        assert_eq!(cleared["archived"], 1);
    }

    #[tokio::test]
    async fn rejects_fields_that_do_not_belong_to_the_selected_action() {
        let directory = tempfile::tempdir().unwrap();
        let tool = TodoTool::new(Arc::new(TodoStore::new(
            directory.path().join("todos.json"),
            crate::todo::TodoScope::workspace(directory.path().into()),
        )));
        let context = context(&directory);
        let error = tool
            .execute(
                json!({
                    "action": "dependencies",
                    "id": Uuid::new_v4(),
                    "blockers": ["wrong action field"]
                }),
                &context,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(error, ToolError::InvalidArguments(message) if message.contains("blockers"))
        );
        assert!(tool.store.snapshot().await.unwrap().items.is_empty());
    }
}
