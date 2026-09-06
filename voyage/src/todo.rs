//! Durable workspace/session-scoped work planning.
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex, OnceLock, Weak},
};
use tokio::{io::AsyncWriteExt, sync::Mutex};
use uuid::Uuid;

pub const STORE_VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TodoId(pub Uuid);
impl TodoId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for TodoId {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoScope {
    pub workspace: PathBuf,
    pub session_id: Option<Uuid>,
}
impl TodoScope {
    pub fn workspace(workspace: PathBuf) -> Self {
        Self {
            workspace,
            session_id: None,
        }
    }
    pub fn session(workspace: PathBuf, session_id: Uuid) -> Self {
        Self {
            workspace,
            session_id: Some(session_id),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Blocked,
    Completed,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimelineEntry {
    pub at: DateTime<Utc>,
    pub author: Option<String>,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoItem {
    pub id: TodoId,
    pub title: String,
    pub description: String,
    pub status: TodoStatus,
    pub priority: Priority,
    pub order: i64,
    pub dependencies: BTreeSet<TodoId>,
    pub blockers: Vec<String>,
    pub assignees: BTreeSet<String>,
    pub notes: Vec<TimelineEntry>,
    pub progress: Vec<TimelineEntry>,
    pub evidence: Vec<TimelineEntry>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
}
impl TodoItem {
    pub fn archived(&self) -> bool {
        self.archived_at.is_some()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TodoList {
    pub version: u32,
    pub revision: u64,
    pub scope: TodoScope,
    pub items: BTreeMap<TodoId, TodoItem>,
}
impl TodoList {
    fn empty(scope: TodoScope) -> Self {
        Self {
            version: STORE_VERSION,
            revision: 0,
            scope,
            items: BTreeMap::new(),
        }
    }
    pub fn ordered(&self) -> Vec<&TodoItem> {
        let mut values: Vec<_> = self.items.values().filter(|i| !i.archived()).collect();
        values.sort_by_key(|i| (i.order, std::cmp::Reverse(i.priority), i.created_at));
        values
    }
}
#[derive(Clone, Debug)]
pub struct NewTodo {
    pub title: String,
    pub description: String,
    pub priority: Priority,
    pub order: Option<i64>,
    pub assignees: BTreeSet<String>,
}
#[derive(Clone)]
pub struct TodoStore {
    path: PathBuf,
    scope: TodoScope,
    gate: Arc<Mutex<()>>,
    coordinator: Option<crate::completion::runtime::Coordinator>,
}

impl TodoStore {
    pub fn new(path: PathBuf, scope: TodoScope) -> Self {
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        Self {
            gate: shared_gate(&path),
            path,
            scope,
            coordinator: None,
        }
    }
    pub fn with_coordinator(
        mut self,
        coordinator: crate::completion::runtime::Coordinator,
    ) -> Self {
        self.coordinator = Some(coordinator);
        self
    }
    pub fn coordinator(&self) -> Option<&crate::completion::runtime::Coordinator> {
        self.coordinator.as_ref()
    }
    pub async fn snapshot(&self) -> Result<TodoList> {
        let _guard = self.gate.lock().await;
        self.load_raw().await
    }
    pub async fn create(&self, new: NewTodo) -> Result<TodoItem> {
        self.create_registered(new, None).await
    }
    /// Import selected remote feedback as ordinary open work, preserving repeat imports.
    pub async fn import_github_feedback(
        &self,
        feedback: crate::github::operator::Feedback,
        policy: std::sync::Arc<crate::policy::Policy>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<TodoItem> {
        feedback.validate()?;
        anyhow::ensure!(
            self.scope.workspace == policy.workspace(),
            "GitHub feedback belongs to another workspace"
        );
        self.mutate(move |list| {
            policy.check_current()?;
            anyhow::ensure!(
                policy.access_mode() != crate::config::AccessMode::ReadOnly,
                "GitHub feedback import is denied in read-only mode"
            );
            anyhow::ensure!(
                !cancellation.is_cancelled(),
                "GitHub feedback import cancelled"
            );
            if let Some(item) = list.items.values().find(|item| {
                item.notes.iter().any(|note| {
                    note.author.as_deref() == Some("helm.github") && note.text == feedback.source
                })
            }) {
                return Ok(item.clone());
            }
            let now = Utc::now();
            let order = list
                .items
                .values()
                .map(|item| item.order)
                .max()
                .unwrap_or(-1)
                .checked_add(1)
                .context("todo order exhausted")?;
            let item = TodoItem {
                id: TodoId::new(),
                title: feedback.title,
                description: feedback.description,
                status: TodoStatus::Pending,
                priority: Priority::Normal,
                order,
                dependencies: BTreeSet::new(),
                blockers: Vec::new(),
                assignees: BTreeSet::new(),
                notes: vec![TimelineEntry {
                    at: now,
                    author: Some("helm.github".into()),
                    text: feedback.source,
                }],
                progress: Vec::new(),
                evidence: Vec::new(),
                created_at: now,
                updated_at: now,
                completed_at: None,
                archived_at: None,
            };
            list.items.insert(item.id, item.clone());
            Ok(item)
        })
        .await
    }
    pub async fn create_registered(
        &self,
        new: NewTodo,
        run: Option<&crate::completion::runtime::RunHandle>,
    ) -> Result<TodoItem> {
        let id = TodoId::new();
        anyhow::ensure!(!new.title.trim().is_empty(), "todo title cannot be empty");
        if let Some(run) = run {
            anyhow::ensure!(
                self.coordinator
                    .as_ref()
                    .is_some_and(|c| c.same(run.coordinator())),
                "todo run coordinator mismatch"
            );
            run.register(crate::completion::Obligation::Todo(id))
                .await?;
        }
        self.mutate(move |list| {
            let title = new.title.trim();
            anyhow::ensure!(!title.is_empty(), "todo title cannot be empty");
            let now = Utc::now();
            let item = TodoItem {
                id,
                title: title.into(),
                description: new.description,
                status: TodoStatus::Pending,
                priority: new.priority,
                order: new.order.unwrap_or_else(|| {
                    list.items.values().map(|i| i.order).max().unwrap_or(-1) + 1
                }),
                dependencies: BTreeSet::new(),
                blockers: vec![],
                assignees: new.assignees,
                notes: vec![],
                progress: vec![],
                evidence: vec![],
                created_at: now,
                updated_at: now,
                completed_at: None,
                archived_at: None,
            };
            list.items.insert(item.id, item.clone());
            Ok(item)
        })
        .await
    }
    pub async fn set_status(&self, id: TodoId, status: TodoStatus) -> Result<TodoItem> {
        self.mutate(move |list| {
            let now = Utc::now();
            let item = list.items.get(&id).context("unknown todo")?;
            anyhow::ensure!(!item.archived(), "archived todo is immutable");
            anyhow::ensure!(
                valid_transition(item.status, status),
                "invalid todo status transition"
            );
            if matches!(status, TodoStatus::InProgress | TodoStatus::Completed) {
                anyhow::ensure!(item.blockers.is_empty(), "blocked todo cannot start");
                for dependency in item.dependencies.clone() {
                    let dep = list.items.get(&dependency).context("missing dependency")?;
                    anyhow::ensure!(
                        dep.status == TodoStatus::Completed,
                        "dependency {} is not completed",
                        dependency.0
                    );
                }
            }
            if status == TodoStatus::Blocked {
                anyhow::ensure!(
                    !item.blockers.is_empty(),
                    "use todo action=block with id and nonempty blockers to explain why the todo is blocked"
                );
            }
            let item = list.items.get_mut(&id).unwrap();
            item.status = status;
            item.completed_at = (status == TodoStatus::Completed).then_some(now);
            item.updated_at = now;
            Ok(item.clone())
        })
        .await
    }
    pub async fn add_dependency(&self, id: TodoId, dependency: TodoId) -> Result<TodoItem> {
        self.update_dependencies(id, BTreeSet::from([dependency]), BTreeSet::new())
            .await
    }
    pub async fn remove_dependency(&self, id: TodoId, dependency: TodoId) -> Result<TodoItem> {
        self.update_dependencies(id, BTreeSet::new(), BTreeSet::from([dependency]))
            .await
    }
    /// Atomically applies a dependency delta, validating the final graph before persistence.
    pub async fn update_dependencies(
        &self,
        id: TodoId,
        add: BTreeSet<TodoId>,
        remove: BTreeSet<TodoId>,
    ) -> Result<TodoItem> {
        self.mutate(move |list| {
            anyhow::ensure!(
                add.is_disjoint(&remove),
                "a dependency cannot be both added and removed"
            );
            anyhow::ensure!(
                !list.items.get(&id).context("unknown todo")?.archived(),
                "archived todo is immutable"
            );
            anyhow::ensure!(
                !matches!(
                    list.items.get(&id).unwrap().status,
                    TodoStatus::Completed | TodoStatus::Cancelled
                ),
                "terminal todo dependencies are immutable"
            );
            for dependency in &add {
                anyhow::ensure!(id != *dependency, "todo cannot depend on itself");
                anyhow::ensure!(list.items.contains_key(dependency), "unknown dependency");
            }
            let item = list.items.get_mut(&id).unwrap();
            for dependency in &remove {
                item.dependencies.remove(dependency);
            }
            item.dependencies.extend(add);
            if has_cycle(list, id, &mut BTreeSet::new(), &mut BTreeSet::new()) {
                anyhow::bail!("dependency would create a cycle")
            }
            let item = list.items.get_mut(&id).unwrap();
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn set_blockers(&self, id: TodoId, blockers: Vec<String>) -> Result<TodoItem> {
        self.mutate(move |list| {
            let item = list.items.get_mut(&id).context("unknown todo")?;
            anyhow::ensure!(!item.archived(), "archived todo is immutable");
            anyhow::ensure!(
                !matches!(item.status, TodoStatus::Completed | TodoStatus::Cancelled),
                "terminal todo cannot be blocked"
            );
            item.blockers = blockers
                .into_iter()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
                .collect();
            item.status = if item.blockers.is_empty() && item.status == TodoStatus::Blocked {
                TodoStatus::Pending
            } else if !item.blockers.is_empty() {
                TodoStatus::Blocked
            } else {
                item.status
            };
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn assign(&self, id: TodoId, assignees: BTreeSet<String>) -> Result<TodoItem> {
        self.mutate(move |list| {
            anyhow::ensure!(
                assignees.iter().all(|a| !a.trim().is_empty()),
                "assignee cannot be empty"
            );
            let item = list.items.get_mut(&id).context("unknown todo")?;
            anyhow::ensure!(!item.archived(), "archived todo is immutable");
            item.assignees = assignees;
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn edit(
        &self,
        id: TodoId,
        title: Option<String>,
        description: Option<String>,
        priority: Option<Priority>,
    ) -> Result<TodoItem> {
        self.mutate(move |list| {
            let item = list.items.get_mut(&id).context("unknown todo")?;
            anyhow::ensure!(!item.archived(), "archived todo is immutable");
            if let Some(title) = title {
                anyhow::ensure!(!title.trim().is_empty(), "todo title cannot be empty");
                item.title = title.trim().into();
            }
            if let Some(description) = description {
                item.description = description;
            }
            if let Some(priority) = priority {
                item.priority = priority;
            }
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn reorder(&self, id: TodoId, order: i64) -> Result<TodoItem> {
        self.mutate(move |list| {
            let item = list.items.get_mut(&id).context("unknown todo")?;
            anyhow::ensure!(!item.archived(), "archived todo is immutable");
            item.order = order;
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn append_note(
        &self,
        id: TodoId,
        kind: EntryKind,
        text: String,
        author: Option<String>,
    ) -> Result<TodoItem> {
        self.mutate(move |list| {
            anyhow::ensure!(!text.trim().is_empty(), "entry cannot be empty");
            let item = list.items.get_mut(&id).context("unknown todo")?;
            anyhow::ensure!(!item.archived(), "archived todo is immutable");
            let entry = TimelineEntry {
                at: Utc::now(),
                author,
                text,
            };
            match kind {
                EntryKind::Note => item.notes.push(entry),
                EntryKind::Progress => item.progress.push(entry),
                EntryKind::Evidence => item.evidence.push(entry),
            }
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn archive(&self, id: TodoId) -> Result<TodoItem> {
        self.mutate(move |list| {
            let item = list.items.get_mut(&id).context("unknown todo")?;
            anyhow::ensure!(
                matches!(item.status, TodoStatus::Completed | TodoStatus::Cancelled),
                "only completed or cancelled todos can be archived"
            );
            item.archived_at = Some(Utc::now());
            item.updated_at = Utc::now();
            Ok(item.clone())
        })
        .await
    }
    pub async fn remove(&self, id: TodoId) -> Result<()> {
        self.mutate(move |list| {
            let item = list.items.get(&id).context("unknown todo")?;
            anyhow::ensure!(item.archived(), "todo must be archived before removal");
            anyhow::ensure!(
                !list
                    .items
                    .values()
                    .any(|other| other.id != id && other.dependencies.contains(&id)),
                "todo is still referenced as a dependency"
            );
            list.items.remove(&id);
            Ok(())
        })
        .await
    }
    pub async fn clear_completed(&self) -> Result<usize> {
        self.mutate(move |list| {
            let now = Utc::now();
            let mut count = 0;
            for item in list.items.values_mut() {
                if item.status == TodoStatus::Completed && item.archived_at.is_none() {
                    item.archived_at = Some(now);
                    item.updated_at = now;
                    count += 1;
                }
            }
            Ok(count)
        })
        .await
    }
    async fn mutate<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut TodoList) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        // Cancellation of a tool waiter must not release the coordination lease
        // while an already-started filesystem commit continues in the background.
        let store = self.clone();
        tokio::spawn(async move {
            let _coordination = match &store.coordinator {
                Some(c) => Some(c.lock().await?),
                None => None,
            };
            let _guard = store.gate.lock().await;
            let mut list = store.load_raw().await?;
            let result = operation(&mut list)?;
            list.revision = list
                .revision
                .checked_add(1)
                .context("todo revision overflow")?;
            store.save_raw(&list).await?;
            Ok(result)
        })
        .await
        .context("todo writer task failed")?
    }
    async fn load_raw(&self) -> Result<TodoList> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                let list: TodoList =
                    serde_json::from_slice(&bytes).context("malformed todo store")?;
                anyhow::ensure!(
                    list.version == STORE_VERSION,
                    "unsupported todo store version {}",
                    list.version
                );
                anyhow::ensure!(list.scope == self.scope, "todo store scope mismatch");
                validate_graph(&list)?;
                Ok(list)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(TodoList::empty(self.scope.clone()))
            }
            Err(e) => Err(e.into()),
        }
    }
    async fn save_raw(&self, list: &TodoList) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("todo store path has no parent")?;
        tokio::fs::create_dir_all(parent).await?;
        secure(parent, 0o700).await?;
        let temp = self.path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&temp).await?;
        secure(&temp, 0o600).await?;
        file.write_all(&serde_json::to_vec_pretty(list)?).await?;
        file.sync_all().await?;
        if let Err(error) = tokio::fs::rename(&temp, &self.path).await {
            let _ = tokio::fs::remove_file(&temp).await;
            return Err(error.into());
        }
        sync_directory(parent)?;
        Ok(())
    }
}
fn valid_transition(from: TodoStatus, to: TodoStatus) -> bool {
    from == to
        || matches!(
            (from, to),
            (
                TodoStatus::Pending,
                TodoStatus::InProgress
                    | TodoStatus::Blocked
                    | TodoStatus::Completed
                    | TodoStatus::Cancelled
            ) | (
                TodoStatus::InProgress,
                TodoStatus::Pending
                    | TodoStatus::Blocked
                    | TodoStatus::Completed
                    | TodoStatus::Cancelled
            ) | (
                TodoStatus::Blocked,
                TodoStatus::Pending
                    | TodoStatus::InProgress
                    | TodoStatus::Completed
                    | TodoStatus::Cancelled
            ) | (
                TodoStatus::Completed | TodoStatus::Cancelled,
                TodoStatus::Pending
            )
        )
}
fn shared_gate(path: &std::path::Path) -> Arc<Mutex<()>> {
    static GATES: OnceLock<StdMutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .expect("todo gate registry poisoned");
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(path).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(Mutex::new(()));
    gates.insert(path.to_owned(), Arc::downgrade(&gate));
    gate
}
#[derive(Clone, Copy, Debug)]
pub enum EntryKind {
    Note,
    Progress,
    Evidence,
}
fn validate_graph(list: &TodoList) -> Result<()> {
    for item in list.items.values() {
        for dependency in &item.dependencies {
            anyhow::ensure!(
                list.items.contains_key(dependency),
                "todo {} has missing dependency {}",
                item.id.0,
                dependency.0
            );
        }
    }
    for id in list.items.keys() {
        anyhow::ensure!(
            !has_cycle(list, *id, &mut BTreeSet::new(), &mut BTreeSet::new()),
            "todo dependency cycle detected"
        );
    }
    Ok(())
}
fn has_cycle(
    list: &TodoList,
    id: TodoId,
    visiting: &mut BTreeSet<TodoId>,
    visited: &mut BTreeSet<TodoId>,
) -> bool {
    if visited.contains(&id) {
        return false;
    }
    if !visiting.insert(id) {
        return true;
    }
    if let Some(item) = list.items.get(&id) {
        for dependency in &item.dependencies {
            if has_cycle(list, *dependency, visiting, visited) {
                return true;
            }
        }
    }
    visiting.remove(&id);
    visited.insert(id);
    false
}
#[cfg(unix)]
async fn secure(path: &std::path::Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).await?;
    Ok(())
}
#[cfg(not(unix))]
async fn secure(_: &std::path::Path, _: u32) -> Result<()> {
    Ok(())
}
#[cfg(unix)]
fn sync_directory(path: &std::path::Path) -> Result<()> {
    std::fs::File::open(path)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn sync_directory(_: &std::path::Path) -> Result<()> {
    Ok(())
}
