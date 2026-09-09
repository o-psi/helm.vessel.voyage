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
    ClearCompleted {},
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
        ToolDefinition {
            output_schema: None,
            annotations: None,
            name: "todo".into(),
            description: "Manage durable workspace tasks. Each action accepts only its branch's keys. Use block with id and blockers to explain blocked work; [] clears reasons and reopens blocked work. Record verification via evidence, not title/progress text. Status changes do not record evidence. Run completion uses recorded outcomes automatically; no separate completion sign-off is required. Remove/archive/clear_completed never erase run-owned obligations. clear_completed archives all completed items and accepts no id. Examples show shapes; copy actual IDs and record only real evidence.".into(),
            input_schema: schema::input_schema(),
        }
    }
    async fn execute(&self, args: Value, context: &ToolContext) -> Result<String, ToolError> {
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
                    .create_registered(
                        NewTodo {
                            title,
                            description,
                            priority: priority.into(),
                            order,
                            assignees: clean_set(assignees),
                        },
                        context.completion.as_ref(),
                    )
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
            Args::ClearCompleted {} => {
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

mod schema;
