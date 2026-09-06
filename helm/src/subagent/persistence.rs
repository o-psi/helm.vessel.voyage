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
    pub(crate) fn has_history_evidence(&self) -> bool {
        self.path.try_exists().unwrap_or(true)
            || self
                .path
                .with_extension("archive")
                .try_exists()
                .unwrap_or(true)
    }
    pub(crate) fn history_path(&self) -> PathBuf {
        self.path.with_extension("events.json")
    }
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
