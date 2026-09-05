use super::{AgentId, AgentRecord, AgentStatus};
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AgentTree {
    pub agents: BTreeMap<AgentId, AgentRecord>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent::*;
    use std::collections::BTreeSet;
    fn record(status: AgentStatus) -> AgentRecord {
        let now = Utc::now();
        let budget = AgentBudget {
            max_tokens: 10,

            max_terminals: 1,
        };
        AgentRecord {
            completion: None,
            id: AgentId::new(),
            parent_id: None,
            name: "child".into(),
            task: "work".into(),
            status,
            policy: AgentPolicy {
                readable_roots: vec!["/tmp".into()],
                writable_roots: vec![],
                allowed_tools: BTreeSet::new(),
                approval: ApprovalPolicy::Deny,
                budget: budget.clone(),
            },
            budget,
            worktree: None,
            branch: None,
            created_at: now,
            started_at: Some(now),
            finished_at: None,
            updated_at: now,
            recent_progress: vec![],
            result: None,
            error: None,
        }
    }
    #[tokio::test]
    async fn restart_marks_live_agents_interrupted() {
        let d = tempfile::tempdir().unwrap();
        let store = AgentTreeStore::new(d.path().join("tree.json"));
        let item = record(AgentStatus::Running);
        let id = item.id;
        store.create(item).await.unwrap();
        assert_eq!(store.recover_after_restart().await.unwrap(), 1);
        let restored = store.get(id).await.unwrap().unwrap();
        assert_eq!(restored.status, AgentStatus::Interrupted);
        assert!(restored.finished_at.is_some());
    }
    #[tokio::test]
    async fn legacy_turn_budgets_load_and_are_removed_on_save() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tree.json");
        let item = record(AgentStatus::Running);
        let id = item.id;
        let mut tree = AgentTree::default();
        tree.insert(item.clone()).unwrap();
        let mut legacy = serde_json::to_value(tree).unwrap();
        for agent in legacy["agents"].as_object_mut().unwrap().values_mut() {
            agent["budget"]["max_turns"] = serde_json::json!(1);
            for field in ["max_runtime_secs", "max_children"] {
                agent["budget"][field] = serde_json::json!(0);
                agent["policy"]["budget"][field] = serde_json::json!(0);
            }
            agent["policy"]["budget"]["max_turns"] = serde_json::json!(64);
        }
        std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let store = AgentTreeStore::new(path.clone());
        assert_eq!(store.get(id).await.unwrap().unwrap().budget, item.budget);
        assert_eq!(store.recover_after_restart().await.unwrap(), 1);
        let restored = store.get(id).await.unwrap().unwrap();
        assert_eq!(restored.status, AgentStatus::Interrupted);
        assert_eq!(restored.policy, item.policy);
        let saved = std::fs::read_to_string(path).unwrap();
        for field in ["max_turns", "max_runtime_secs", "max_children"] {
            assert!(!saved.contains(field));
        }
    }

    #[test]
    fn terminal_states_cannot_transition() {
        for status in [
            AgentStatus::Completed,
            AgentStatus::Failed,
            AgentStatus::Cancelled,
            AgentStatus::Interrupted,
            AgentStatus::TimedOut,
        ] {
            let item = record(status);
            let id = item.id;
            let mut tree = AgentTree::default();
            tree.insert(item).unwrap();
            assert!(tree.transition(id, AgentStatus::Running, None).is_err());
        }
    }

    #[test]
    fn pruning_terminal_history_preserves_active_agents_and_tree_integrity() {
        let mut tree = AgentTree::default();
        let mut root = record(AgentStatus::Running);
        root.finished_at = None;
        let root_id = root.id;
        tree.insert(root).unwrap();

        let mut child = record(AgentStatus::Completed);
        child.parent_id = Some(root_id);
        child.worktree = Some("/worktree-for-integration".into());
        let child_id = child.id;
        tree.insert(child).unwrap();

        let old_terminal = record(AgentStatus::TimedOut);
        let old_terminal_id = old_terminal.id;
        tree.insert(old_terminal).unwrap();

        let removed = tree.prune_terminal_leaves(1, None);
        assert_eq!(removed, vec![old_terminal_id]);
        assert!(!removed.contains(&child_id));
        assert!(removed.contains(&old_terminal_id));
        assert_eq!(tree.agents.len(), 2);
        assert!(tree.agents.contains_key(&root_id));
        assert!(tree.agents.contains_key(&child_id));
        assert!(tree.agents.values().all(|agent| {
            agent
                .parent_id
                .is_none_or(|parent| tree.agents.contains_key(&parent))
        }));
    }

    #[test]
    fn pruning_does_not_remove_a_protected_terminal_parent() {
        let mut tree = AgentTree::default();
        let parent = record(AgentStatus::Completed);
        let parent_id = parent.id;
        tree.insert(parent).unwrap();
        let removed = tree.prune_terminal_leaves(0, Some(parent_id));
        assert!(removed.is_empty());
        assert!(tree.agents.contains_key(&parent_id));
    }

    #[tokio::test]
    async fn store_pruning_is_durable() {
        let directory = tempfile::tempdir().unwrap();
        let store = AgentTreeStore::new(directory.path().join("tree.json"));
        for _ in 0..3 {
            store.create(record(AgentStatus::Completed)).await.unwrap();
        }

        assert_eq!(store.prune_terminal_leaves(2, None).await.unwrap().len(), 1);
        assert_eq!(store.list().await.unwrap().len(), 2);
        assert_eq!(
            AgentTreeStore::new(directory.path().join("tree.json"))
                .list()
                .await
                .unwrap()
                .len(),
            2
        );
    }
    #[tokio::test]
    async fn archive_preserves_every_terminal_outcome_and_original_links() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tree.json");
        let store = AgentTreeStore::new(path.clone());
        let mut parent = None;
        let mut expected = Vec::new();
        for status in [
            AgentStatus::Completed,
            AgentStatus::Failed,
            AgentStatus::Cancelled,
            AgentStatus::TimedOut,
            AgentStatus::Interrupted,
        ] {
            let mut item = record(status);
            item.parent_id = parent;
            item.result = Some("retained result α".into());
            item.error = Some("retained error".into());
            item.recent_progress = vec!["checkpoint".into()];
            item.worktree = Some(directory.path().join("untouched"));
            std::fs::write(item.worktree.as_ref().unwrap(), "user data").unwrap();
            parent = Some(item.id);
            expected.push(item.clone());
            store.create(item).await.unwrap();
        }
        assert_eq!(store.prune_terminal_leaves(0, None).await.unwrap().len(), 5);
        assert!(store.list().await.unwrap().is_empty());
        let reopened = AgentTreeStore::new(path);
        for item in expected {
            assert_eq!(
                reopened
                    .get_archived(item.id)
                    .await
                    .unwrap()
                    .unwrap()
                    .record,
                item
            );
            assert_eq!(
                std::fs::read_to_string(item.worktree.unwrap()).unwrap(),
                "user data"
            );
        }
        assert_eq!(
            reopened
                .list_archived(None, 100)
                .await
                .unwrap()
                .agents
                .len(),
            5
        );
    }

    #[tokio::test]
    async fn archive_failure_retains_original_and_retry_deduplicates() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tree.json");
        let store = AgentTreeStore::new(path.clone());
        let item = record(AgentStatus::Completed);
        store.create(item.clone()).await.unwrap();
        std::fs::write(path.with_extension("archive"), "blocked archive directory").unwrap();
        assert!(store.prune_terminal_leaves(0, None).await.is_err());
        assert_eq!(store.get(item.id).await.unwrap(), Some(item.clone()));
        std::fs::remove_file(path.with_extension("archive")).unwrap();
        // Simulate crash after durable archive write, before working-tree retirement.
        super::super::archive::AgentArchive::new(&path)
            .put(&item)
            .await
            .unwrap();
        assert!(
            store
                .list_archived(None, 20)
                .await
                .unwrap()
                .agents
                .is_empty()
        );
        assert_eq!(
            store.prune_terminal_leaves(0, None).await.unwrap(),
            vec![item.id]
        );
        assert_eq!(store.list_archived(None, 20).await.unwrap().agents.len(), 1);
        assert!(
            store
                .prune_terminal_leaves(0, None)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn archive_pagination_is_bounded_and_workspace_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let store = AgentTreeStore::new(directory.path().join("tree.json"));
        let other = AgentTreeStore::new(directory.path().join("other.json"));
        let mut expected = Vec::new();
        for _ in 0..5 {
            let mut item = record(AgentStatus::Completed);
            item.name = "α".repeat(500);
            item.task = "β".repeat(1000);
            expected.push(item.id);
            store.create(item).await.unwrap();
        }
        store.prune_terminal_leaves(0, None).await.unwrap();
        expected.sort();
        let mut after = None;
        let mut actual = Vec::new();
        loop {
            let page = store.list_archived(after, 2).await.unwrap();
            assert!(page.agents.len() <= 2);
            for agent in page.agents {
                assert!(agent.name.chars().count() <= 120);
                assert!(agent.task_preview.chars().count() <= 240);
                actual.push(agent.id);
            }
            after = page.next_after;
            if after.is_none() {
                break;
            }
        }
        assert_eq!(actual, expected);
        assert!(store.list_archived(None, 0).await.is_err());
        assert!(store.list_archived(None, 101).await.is_err());
        assert!(
            other
                .list_archived(None, 20)
                .await
                .unwrap()
                .agents
                .is_empty()
        );
        assert!(other.get_archived(expected[0]).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn malformed_or_conflicting_archive_never_retires_original() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tree.json");
        let store = AgentTreeStore::new(path.clone());
        let item = record(AgentStatus::Completed);
        store.create(item.clone()).await.unwrap();
        let archive = super::super::archive::AgentArchive::new(&path);
        archive.put(&item).await.unwrap();
        let archive_path = path
            .with_extension("archive")
            .join(format!("{}.json", item.id));
        let valid = std::fs::read(&archive_path).unwrap();
        for field in ["version", "id", "status", "result", "json"] {
            let mut value: serde_json::Value = serde_json::from_slice(&valid).unwrap();
            match field {
                "version" => value["version"] = serde_json::json!(999),
                "id" => value["record"]["id"] = serde_json::json!(AgentId::new()),
                "status" => value["record"]["status"] = serde_json::json!("running"),
                "result" => value["record"]["result"] = serde_json::json!("different evidence"),
                _ => value = serde_json::json!("invalid envelope"),
            }
            std::fs::write(&archive_path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(
                store.prune_terminal_leaves(0, None).await.is_err(),
                "{field}"
            );
            assert_eq!(store.get(item.id).await.unwrap(), Some(item.clone()));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn archive_rejects_symlinks_and_uses_private_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tree.json");
        let store = AgentTreeStore::new(path.clone());
        let item = record(AgentStatus::Completed);
        store.create(item.clone()).await.unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), path.with_extension("archive")).unwrap();
        assert!(store.prune_terminal_leaves(0, None).await.is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
        std::fs::remove_file(path.with_extension("archive")).unwrap();
        store.prune_terminal_leaves(0, None).await.unwrap();
        let file = path
            .with_extension("archive")
            .join(format!("{}.json", item.id));
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(path.with_extension("archive"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        std::fs::remove_file(&file).unwrap();
        let outside_file = outside.path().join("secret");
        std::fs::write(&outside_file, "not an agent").unwrap();
        symlink(outside_file, file).unwrap();
        assert!(store.get_archived(item.id).await.is_err());
    }
}
impl AgentTree {
    pub fn insert(&mut self, record: AgentRecord) -> Result<()> {
        if let Some(parent) = record.parent_id {
            anyhow::ensure!(
                self.agents.contains_key(&parent),
                "unknown parent agent {parent}"
            );
        }
        anyhow::ensure!(
            !self.agents.contains_key(&record.id),
            "duplicate agent {}",
            record.id
        );
        self.agents.insert(record.id, record);
        Ok(())
    }
    pub fn transition(
        &mut self,
        id: AgentId,
        status: AgentStatus,
        error: Option<String>,
    ) -> Result<()> {
        let record = self
            .agents
            .get_mut(&id)
            .with_context(|| format!("unknown agent {id}"))?;
        anyhow::ensure!(!record.status.is_terminal(), "agent is terminal");
        record.status = status;
        record.error = error;
        record.updated_at = Utc::now();
        Ok(())
    }
    pub fn recover_after_restart(&mut self) -> usize {
        let mut count = 0;
        for record in self.agents.values_mut() {
            if matches!(
                record.status,
                AgentStatus::Queued | AgentStatus::Running | AgentStatus::Waiting
            ) {
                record.status = AgentStatus::Interrupted;
                record.error =
                    Some("Helm restarted; in-process agent execution did not survive".into());
                record.finished_at = Some(Utc::now());
                record.updated_at = Utc::now();
                count += 1;
            }
        }
        count
    }
    pub fn children(&self, parent: AgentId) -> Vec<&AgentRecord> {
        self.agents
            .values()
            .filter(|r| r.parent_id == Some(parent))
            .collect()
    }

    /// Removes the oldest terminal leaves until the retained tree fits the limit.
    ///
    /// Only leaves are removed so every retained child continues to have its
    /// parent available for tree rendering and follow-up policy checks. A
    /// protected record (normally the parent of a pending spawn) is retained.
    pub fn prune_terminal_leaves(
        &mut self,
        max_records: usize,
        protected: Option<AgentId>,
    ) -> Vec<AgentId> {
        let mut removed = Vec::new();
        while self.agents.len() > max_records {
            let candidate = self
                .agents
                .values()
                .filter(|record| {
                    record.status.is_terminal()
                        && Some(record.id) != protected
                        && !(record.worktree.is_some() && self.has_active_ancestor(record))
                        && !self
                            .agents
                            .values()
                            .any(|child| child.parent_id == Some(record.id))
                })
                .min_by_key(|record| {
                    (
                        record.finished_at.unwrap_or(record.updated_at),
                        record.created_at,
                        record.id,
                    )
                })
                .map(|record| record.id);
            let Some(id) = candidate else {
                break;
            };
            self.agents.remove(&id);
            removed.push(id);
        }
        removed
    }

    fn has_active_ancestor(&self, record: &AgentRecord) -> bool {
        let mut parent = record.parent_id;
        while let Some(id) = parent {
            let Some(ancestor) = self.agents.get(&id) else {
                break;
            };
            if !ancestor.status.is_terminal() {
                return true;
            }
            parent = ancestor.parent_id;
        }
        false
    }
}

#[derive(Clone, Debug)]
pub struct AgentTreeStore {
    path: PathBuf,
    gate: std::sync::Arc<tokio::sync::Mutex<()>>,
    coordinator: Option<crate::completion::runtime::Coordinator>,
    execution_lease: Option<std::sync::Arc<crate::completion::runtime::AgentWriterLease>>,
}
impl AgentTreeStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            coordinator: None,
            execution_lease: None,
        }
    }
    pub fn with_coordinator(
        mut self,
        coordinator: crate::completion::runtime::Coordinator,
    ) -> Self {
        self.coordinator = Some(coordinator);
        self.execution_lease = None;
        self
    }
    pub(crate) fn acquire_runtime_owner(&mut self) -> Result<()> {
        if self.execution_lease.is_none()
            && let Some(coordinator) = &self.coordinator
        {
            self.execution_lease = Some(std::sync::Arc::new(coordinator.acquire_agent_writer()?));
        }
        Ok(())
    }
    fn require_runtime_owner(&self) -> Result<()> {
        anyhow::ensure!(
            self.coordinator.is_none() || self.execution_lease.is_some(),
            "coordinated agent writes require the runtime execution lease"
        );
        Ok(())
    }
    pub fn coordinator(&self) -> Option<&crate::completion::runtime::Coordinator> {
        self.coordinator.as_ref()
    }
    pub async fn load(&self) -> Result<AgentTree> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => {
                let tree: AgentTree =
                    serde_json::from_slice(&bytes).context("invalid agent tree")?;
                for (id, record) in &tree.agents {
                    anyhow::ensure!(*id == record.id, "agent tree identity mismatch");
                    let mut visited = std::collections::BTreeSet::from([*id]);
                    let mut parent = record.parent_id;
                    while let Some(id) = parent {
                        anyhow::ensure!(visited.insert(id), "cycle in agent tree");
                        parent = tree
                            .agents
                            .get(&id)
                            .context("missing parent in agent tree")?
                            .parent_id;
                    }
                }
                Ok(tree)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AgentTree::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub async fn save(&self, tree: &AgentTree) -> Result<()> {
        self.require_runtime_owner()?;
        let tree = tree.clone();
        let store = self.clone();
        tokio::spawn(async move {
            let _coordination = match &store.coordinator {
                Some(c) => Some(c.lock().await?),
                None => None,
            };
            let _guard = store.gate.lock().await;
            store.save_raw(&tree).await
        })
        .await
        .context("agent writer task failed")?
    }
    async fn save_raw(&self, tree: &AgentTree) -> Result<()> {
        let parent = self.path.parent().context("invalid agent tree path")?;
        tokio::fs::create_dir_all(parent).await?;
        secure(parent, 0o700).await?;
        let temp = self.path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&temp).await?;
        file.write_all(&serde_json::to_vec_pretty(tree)?).await?;
        secure(&temp, 0o600).await?;
        file.sync_all().await?;
        tokio::fs::rename(&temp, &self.path).await?;
        Ok(())
    }
    pub async fn create(&self, record: AgentRecord) -> Result<()> {
        self.require_runtime_owner()?;
        let store = self.clone();
        tokio::spawn(async move {
            let _coordination = match &store.coordinator {
                Some(c) => Some(c.lock().await?),
                None => None,
            };
            let _guard = store.gate.lock().await;
            let mut tree = store.load().await?;
            tree.insert(record)?;
            store.save_raw(&tree).await
        })
        .await
        .context("agent writer task failed")?
    }
    pub async fn update(&self, record: AgentRecord) -> Result<()> {
        self.require_runtime_owner()?;
        let store = self.clone();
        tokio::spawn(async move {
            let _coordination = match &store.coordinator {
                Some(c) => Some(c.lock().await?),
                None => None,
            };
            let _guard = store.gate.lock().await;
            let mut tree = store.load().await?;
            anyhow::ensure!(
                tree.agents.contains_key(&record.id),
                "unknown agent {}",
                record.id
            );
            tree.agents.insert(record.id, record);
            store.save_raw(&tree).await
        })
        .await
        .context("agent writer task failed")?
    }
    pub async fn get(&self, id: AgentId) -> Result<Option<AgentRecord>> {
        Ok(self.load().await?.agents.remove(&id))
    }
    pub async fn get_archived(&self, id: AgentId) -> Result<Option<super::ArchivedAgent>> {
        super::archive::AgentArchive::new(&self.path).get(id).await
    }
    pub async fn list_archived(
        &self,
        after: Option<AgentId>,
        limit: usize,
    ) -> Result<super::ArchivePage> {
        let _guard = self.gate.lock().await;
        let retained = self.load().await?.agents.into_keys().collect();
        super::archive::AgentArchive::new(&self.path)
            .list(after, limit, &retained)
            .await
    }
    pub async fn list(&self) -> Result<Vec<AgentRecord>> {
        Ok(self.load().await?.agents.into_values().collect())
    }
    /// Called under the workspace mutation boundary. Only ancestry/status
    /// metadata is collected; unrelated result text is never returned.
    pub(crate) async fn adoption_subtree(&self, root: AgentId, max: usize) -> Result<Vec<AgentId>> {
        let mut records: std::collections::BTreeMap<_, _> = self
            .load()
            .await?
            .agents
            .into_values()
            .map(|record| (record.id, (record.parent_id, record.status)))
            .collect();
        let mut after = None;
        loop {
            let page = self.list_archived(after, 100).await?;
            for record in page.agents {
                records
                    .entry(record.id)
                    .or_insert((record.parent_id, record.status));
            }
            match page.next_after {
                Some(next) => {
                    anyhow::ensure!(
                        after.is_none_or(|previous| previous < next),
                        "archive cursor did not advance"
                    );
                    after = Some(next);
                }
                None => break,
            }
        }
        anyhow::ensure!(records.contains_key(&root), "agent missing");
        let mut selected = std::collections::BTreeSet::from([root]);
        loop {
            let before = selected.len();
            for (id, (parent, _)) in &records {
                if parent.is_some_and(|parent| selected.contains(&parent)) {
                    selected.insert(*id);
                }
            }
            anyhow::ensure!(
                selected.len() <= max,
                "agent subtree exceeds completion obligation limit"
            );
            if selected.len() == before {
                break;
            }
        }
        for id in &selected {
            anyhow::ensure!(
                records[id].1.is_terminal(),
                "wait or cancel active work before adoption; agent {} is active",
                id
            );
            let mut ancestors = std::collections::BTreeSet::from([*id]);
            let mut parent = records[id].0;
            while let Some(id) = parent {
                anyhow::ensure!(ancestors.insert(id), "cycle in adopted agent ancestry");
                parent = records.get(&id).and_then(|record| record.0);
            }
        }
        Ok(selected.into_iter().collect())
    }
    pub async fn prune_terminal_leaves(
        &self,
        max_records: usize,
        protected: Option<AgentId>,
    ) -> Result<Vec<AgentId>> {
        self.require_runtime_owner()?;
        let store = self.clone();
        tokio::spawn(async move {
            let _coordination = match &store.coordinator {
                Some(c) => Some(c.lock().await?),
                None => None,
            };
            let _guard = store.gate.lock().await;
            let mut tree = store.load().await?;
            let original = tree.clone();
            let removed = tree.prune_terminal_leaves(max_records, protected);
            if !removed.is_empty() {
                let archive = super::archive::AgentArchive::new(&store.path);
                for id in &removed {
                    archive.put(&original.agents[id]).await?;
                }
                store.save_raw(&tree).await?;
            }
            Ok(removed)
        })
        .await
        .context("agent writer task failed")?
    }
    pub async fn recover_after_restart(&self) -> Result<usize> {
        self.require_runtime_owner()?;
        let store = self.clone();
        tokio::spawn(async move {
            let _coordination = match &store.coordinator {
                Some(c) => Some(c.lock().await?),
                None => None,
            };
            let _guard = store.gate.lock().await;
            let mut tree = store.load().await?;
            let count = tree.recover_after_restart();
            if count > 0 {
                store.save_raw(&tree).await?;
            }
            Ok(count)
        })
        .await
        .context("agent writer task failed")?
    }
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
