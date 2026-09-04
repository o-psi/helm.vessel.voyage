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
            max_turns: 2,
            max_tokens: 10,
            max_runtime_secs: 30,
            max_children: 1,
            max_terminals: 1,
        };
        AgentRecord {
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
                        && !self.has_active_ancestor(record)
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
}
impl AgentTreeStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            gate: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    pub async fn load(&self) -> Result<AgentTree> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).context("invalid agent tree"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(AgentTree::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub async fn save(&self, tree: &AgentTree) -> Result<()> {
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
        let _guard = self.gate.lock().await;
        let mut tree = self.load().await?;
        tree.insert(record)?;
        self.save(&tree).await
    }
    pub async fn update(&self, record: AgentRecord) -> Result<()> {
        let _guard = self.gate.lock().await;
        let mut tree = self.load().await?;
        anyhow::ensure!(
            tree.agents.contains_key(&record.id),
            "unknown agent {}",
            record.id
        );
        tree.agents.insert(record.id, record);
        self.save(&tree).await
    }
    pub async fn get(&self, id: AgentId) -> Result<Option<AgentRecord>> {
        Ok(self.load().await?.agents.remove(&id))
    }
    pub async fn list(&self) -> Result<Vec<AgentRecord>> {
        Ok(self.load().await?.agents.into_values().collect())
    }
    pub async fn prune_terminal_leaves(
        &self,
        max_records: usize,
        protected: Option<AgentId>,
    ) -> Result<Vec<AgentId>> {
        let _guard = self.gate.lock().await;
        let mut tree = self.load().await?;
        let removed = tree.prune_terminal_leaves(max_records, protected);
        if !removed.is_empty() {
            self.save(&tree).await?;
        }
        Ok(removed)
    }
    pub async fn recover_after_restart(&self) -> Result<usize> {
        let _guard = self.gate.lock().await;
        let mut tree = self.load().await?;
        let count = tree.recover_after_restart();
        if count > 0 {
            self.save(&tree).await?;
        }
        Ok(count)
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
