use crate::{
    config::default_data_dir,
    model::{Message, Usage},
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::fs;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub workspace: PathBuf,
    pub model: String,
    pub messages: Vec<Message>,
    pub usage: Usage,
}

impl Session {
    pub fn new(workspace: PathBuf, model: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            created_at: now,
            updated_at: now,
            workspace,
            model,
            messages: Vec::new(),
            usage: Usage::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionStore {
    directory: PathBuf,
}
impl Default for SessionStore {
    fn default() -> Self {
        Self {
            directory: default_data_dir().join("sessions"),
        }
    }
}

impl SessionStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
    pub async fn save(&self, session: &mut Session) -> Result<()> {
        session.updated_at = Utc::now();
        fs::create_dir_all(&self.directory).await?;
        let destination = self.path(session.id);
        let temporary = self
            .directory
            .join(format!(".{}.{}.tmp", session.id, nonce()));
        fs::write(&temporary, serde_json::to_vec_pretty(session)?).await?;
        fs::rename(&temporary, &destination)
            .await
            .with_context(|| format!("failed to save session {}", session.id))?;
        Ok(())
    }
    pub async fn load(&self, id: Uuid) -> Result<Session> {
        load_path(&self.path(id)).await
    }
    pub async fn load_reference(&self, reference: &str) -> Result<Session> {
        let direct = Path::new(reference);
        if direct.exists() {
            return load_path(direct).await;
        }
        let id = Uuid::parse_str(reference)
            .with_context(|| format!("invalid session id or path: {reference}"))?;
        self.load(id).await
    }
    pub async fn list(&self) -> Result<Vec<Session>> {
        let mut result = Vec::new();
        let mut entries = match fs::read_dir(&self.directory).await {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(result),
            Err(e) => return Err(e.into()),
        };
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().and_then(|e| e.to_str()) == Some("json")
                && let Ok(session) = load_path(&entry.path()).await
            {
                result.push(session);
            }
        }
        result.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(result)
    }
    fn path(&self, id: Uuid) -> PathBuf {
        self.directory.join(format!("{id}.json"))
    }
}

async fn load_path(path: &Path) -> Result<Session> {
    let data = fs::read(path)
        .await
        .with_context(|| format!("failed to read session {}", path.display()))?;
    serde_json::from_slice(&data).with_context(|| format!("invalid session {}", path.display()))
}
fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "test".into());
        store.save(&mut session).await.unwrap();
        assert_eq!(store.load(session.id).await.unwrap().id, session.id);
    }
}
