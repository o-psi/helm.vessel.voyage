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
        anyhow::ensure!(
            !matches!(
                record.status,
                AgentStatus::Completed | AgentStatus::Cancelled
            ),
            "agent is terminal"
        );
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
        let temp = self.path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        let mut file = tokio::fs::File::create(&temp).await?;
        file.write_all(&serde_json::to_vec_pretty(tree)?).await?;
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
