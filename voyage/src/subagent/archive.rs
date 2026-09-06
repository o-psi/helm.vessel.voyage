//! Immutable terminal records, separate from the bounded working tree.
use super::{AgentId, AgentRecord, AgentStatus};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchivedAgent {
    pub version: u32,
    pub archived_at: DateTime<Utc>,
    pub record: AgentRecord,
}
impl ArchivedAgent {
    pub(crate) fn new(record: AgentRecord) -> Self {
        Self {
            version: 1,
            archived_at: Utc::now(),
            record,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ArchiveSummary {
    pub id: AgentId,
    pub parent_id: Option<AgentId>,
    pub name: String,
    pub task_preview: String,
    pub status: AgentStatus,
    pub archived_at: DateTime<Utc>,
}
impl From<ArchivedAgent> for ArchiveSummary {
    fn from(value: ArchivedAgent) -> Self {
        Self {
            id: value.record.id,
            parent_id: value.record.parent_id,
            name: value.record.name.chars().take(120).collect(),
            task_preview: value.record.task.chars().take(240).collect(),
            status: value.record.status,
            archived_at: value.archived_at,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ArchivePage {
    pub agents: Vec<ArchiveSummary>,
    pub next_after: Option<AgentId>,
}
impl ArchivePage {
    pub(crate) fn validate_limit(limit: usize) -> Result<()> {
        ensure!(
            (1..=100).contains(&limit),
            "archive limit must be between 1 and 100"
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AgentArchive {
    directory: PathBuf,
}
impl AgentArchive {
    pub fn new(tree_path: &Path) -> Self {
        Self {
            directory: tree_path.with_extension("archive"),
        }
    }

    async fn exists(&self) -> Result<bool> {
        match tokio::fs::symlink_metadata(&self.directory).await {
            Ok(meta) => {
                ensure!(
                    meta.is_dir() && !meta.file_type().is_symlink(),
                    "invalid subagent archive directory"
                );
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn get(&self, id: AgentId) -> Result<Option<ArchivedAgent>> {
        if !self.exists().await? {
            return Ok(None);
        }
        let path = self.directory.join(format!("{id}.json"));
        match tokio::fs::symlink_metadata(&path).await {
            Ok(meta) => ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "invalid archived agent file"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        let mut options = tokio::fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        options.custom_flags(libc::O_NOFOLLOW);
        let file = options.open(path).await?;
        let mut bytes = Vec::new();
        file.take(MAX_RECORD_BYTES + 1)
            .read_to_end(&mut bytes)
            .await?;
        ensure!(
            bytes.len() as u64 <= MAX_RECORD_BYTES,
            "archived agent record exceeds size limit"
        );
        let value: ArchivedAgent =
            serde_json::from_slice(&bytes).context("invalid archived agent record")?;
        ensure!(
            value.version == 1 && value.record.id == id && value.record.status.is_terminal(),
            "invalid archived agent version, identity or status"
        );
        Ok(Some(value))
    }

    /// The caller serializes archive and working-tree mutations. Existing records
    /// are immutable; retries after a partial archive transaction are idempotent.
    pub async fn put(&self, record: &AgentRecord) -> Result<()> {
        ensure!(record.status.is_terminal(), "cannot archive a live agent");
        if let Some(existing) = self.get(record.id).await? {
            ensure!(
                &existing.record == record,
                "archived agent conflicts with retained record"
            );
            self.sync_directory().await?;
            return Ok(());
        }
        if !self.exists().await? {
            let mut builder = tokio::fs::DirBuilder::new();
            #[cfg(unix)]
            builder.mode(0o700);
            builder.create(&self.directory).await?;
        }
        let bytes = serde_json::to_vec(&ArchivedAgent::new(record.clone()))?;
        ensure!(
            bytes.len() as u64 <= MAX_RECORD_BYTES,
            "archived agent record exceeds size limit"
        );
        let temp = self.directory.join(format!(".tmp-{}", Uuid::new_v4()));
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temp).await?;
        let result = async {
            file.write_all(&bytes).await?;
            file.sync_all().await?;
            drop(file);
            tokio::fs::rename(&temp, self.directory.join(format!("{}.json", record.id))).await?;
            self.sync_directory().await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(temp).await;
        }
        result
    }

    async fn sync_directory(&self) -> Result<()> {
        #[cfg(unix)]
        {
            tokio::fs::File::open(&self.directory)
                .await?
                .sync_all()
                .await?;
            if let Some(parent) = self.directory.parent() {
                tokio::fs::File::open(parent).await?.sync_all().await?;
            }
        }
        Ok(())
    }

    pub async fn list(
        &self,
        after: Option<AgentId>,
        limit: usize,
        retained: &BTreeSet<AgentId>,
    ) -> Result<ArchivePage> {
        ArchivePage::validate_limit(limit)?;
        let mut ids = BTreeSet::new();
        if self.exists().await? {
            let mut entries = tokio::fs::read_dir(&self.directory).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name();
                let Some(name) = name.to_str().and_then(|name| name.strip_suffix(".json")) else {
                    continue;
                };
                let Ok(id) = Uuid::parse_str(name).map(AgentId) else {
                    continue;
                };
                if after.is_some_and(|cursor| id <= cursor) || retained.contains(&id) {
                    continue;
                }
                ids.insert(id);
                if ids.len() > limit + 1 {
                    ids.pop_last();
                }
            }
        }
        let has_more = ids.len() > limit;
        if has_more {
            ids.pop_last();
        }
        let next_after = if has_more { ids.last().copied() } else { None };
        let mut agents = Vec::new();
        for id in ids {
            agents.push(
                self.get(id)
                    .await?
                    .context("archived agent disappeared")?
                    .into(),
            );
        }
        Ok(ArchivePage { agents, next_after })
    }
}
