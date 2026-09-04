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
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// Shrink a conversation without breaking tool-call groups. The system message and
/// most recent messages are retained; removed history is represented by a durable
/// summary marker so providers are told that context was intentionally compacted.
pub fn compact_messages(messages: &mut Vec<Message>, retain: usize) -> usize {
    if messages.len() <= retain.max(2) {
        return 0;
    }
    let system = messages
        .first()
        .filter(|message| message.role == crate::model::Role::System)
        .cloned();
    let keep = retain.max(2).saturating_sub(usize::from(system.is_some()));
    let nominal_split = messages.len().saturating_sub(keep);
    // Never retain a tool result without the user turn which led to its call.
    let mut split = (nominal_split..messages.len())
        .find(|index| messages[*index].role == crate::model::Role::User)
        .unwrap_or(nominal_split);
    // A long tool loop may have no later user turn. Keep the assistant
    // call together with all of its results at the fallback boundary.
    while split > 0 && messages[split].role == crate::model::Role::Tool {
        split -= 1;
    }
    let removed = split.saturating_sub(usize::from(system.is_some()));
    let mut recent = messages.split_off(split);
    messages.clear();
    if let Some(system) = system {
        messages.push(system);
    }
    messages.push(Message::new(
        crate::model::Role::System,
        format!("[Earlier conversation compacted: {removed} messages omitted.]"),
    ));
    messages.append(&mut recent);
    removed
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub workspace: PathBuf,
    pub model: String,
    #[serde(default)]
    pub model_history: Vec<ModelChange>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    pub messages: Vec<Message>,
    pub usage: Usage,
    #[serde(default)]
    pub terminals: Vec<crate::terminal::TerminalSummary>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelChange {
    pub from: String,
    pub to: String,
    pub changed_at: DateTime<Utc>,
}

impl Session {
    pub fn new(workspace: PathBuf, model: String) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4();
        Self {
            id,
            created_at: now,
            updated_at: now,
            workspace,
            model,
            model_history: Vec::new(),
            name: Some(generated_name(id)),
            parent_id: None,
            messages: Vec::new(),
            usage: Usage::default(),
            terminals: Vec::new(),
        }
    }

    /// Return the durable name, generating the same stable fallback used to migrate old sessions.
    pub fn display_name(&self) -> String {
        self.name
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| generated_name(self.id))
    }

    fn ensure_name(&mut self) {
        if self
            .name
            .as_deref()
            .is_none_or(|name| name.trim().is_empty())
        {
            self.name = Some(generated_name(self.id));
        }
    }

    pub fn switch_model(&mut self, model: impl Into<String>) -> Result<bool> {
        let model = model.into();
        let model = model.trim();
        anyhow::ensure!(!model.is_empty(), "model cannot be empty");
        if self.model == model {
            return Ok(false);
        }
        self.model_history.push(ModelChange {
            from: self.model.clone(),
            to: model.to_owned(),
            changed_at: Utc::now(),
        });
        for message in &mut self.messages {
            message.provider_state = None;
        }
        self.model = model.to_owned();
        Ok(true)
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
        session.ensure_name();
        session.updated_at = Utc::now();
        fs::create_dir_all(&self.directory).await?;
        secure_directory(&self.directory).await?;
        let destination = self.path(session.id);
        let temporary = self
            .directory
            .join(format!(".{}.{}.tmp", session.id, nonce()));
        let mut file = fs::File::create(&temporary).await?;
        secure_file(&temporary).await?;
        // System guidance belongs to the current runtime, not durable user history. Keeping it
        // on disk makes resumed sessions inherit stale tools, policy, or provider identity.
        let mut persisted = session.clone();
        persisted
            .messages
            .retain(|message| message.role != crate::model::Role::System);
        file.write_all(&serde_json::to_vec_pretty(&persisted)?)
            .await?;
        file.sync_all().await?;
        if let Err(error) = fs::rename(&temporary, &destination).await {
            let _ = fs::remove_file(&temporary).await;
            return Err(error).with_context(|| format!("failed to save session {}", session.id));
        }
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
        if let Ok(id) = Uuid::parse_str(reference) {
            return self.load(id).await;
        }
        let matches: Vec<_> = self
            .list()
            .await?
            .into_iter()
            .filter(|session| session.name.as_deref() == Some(reference))
            .collect();
        match matches.as_slice() {
            [session] => Ok(session.clone()),
            [] => anyhow::bail!("unknown session id, name, or path: {reference}"),
            _ => anyhow::bail!("session name is ambiguous: {reference}"),
        }
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
    pub async fn delete(&self, id: Uuid) -> Result<()> {
        match fs::remove_file(self.path(id)).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn branch(&self, source: &Session, name: Option<String>) -> Result<Session> {
        let now = Utc::now();
        let mut branch = source.clone();
        branch.id = Uuid::new_v4();
        branch.created_at = now;
        branch.updated_at = now;
        branch.parent_id = Some(source.id);
        branch.name = name
            .filter(|name| !name.trim().is_empty())
            .or_else(|| Some(generated_name(branch.id)));
        self.save(&mut branch).await?;
        Ok(branch)
    }

    pub async fn export_markdown(&self, session: &Session, path: &Path) -> Result<()> {
        let mut output = format!(
            "# {}\n\n- Session: `{}`\n- Model: `{}`\n- Workspace: `{}`\n- Updated: {}\n\n",
            session.display_name(),
            session.id,
            session.model,
            session.workspace.display(),
            session.updated_at.to_rfc3339()
        );
        for message in &session.messages {
            use std::fmt::Write as _;
            let _ = writeln!(output, "## {:?}\n\n{}\n", message.role, message.content);
        }
        fs::write(path, output)
            .await
            .with_context(|| format!("failed to export session to {}", path.display()))
    }
    fn path(&self, id: Uuid) -> PathBuf {
        self.directory.join(format!("{id}.json"))
    }
}

#[cfg(unix)]
async fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).await?;
    Ok(())
}
#[cfg(not(unix))]
async fn secure_directory(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn secure_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    Ok(())
}
#[cfg(not(unix))]
async fn secure_file(_: &Path) -> Result<()> {
    Ok(())
}

async fn load_path(path: &Path) -> Result<Session> {
    let data = fs::read(path)
        .await
        .with_context(|| format!("failed to read session {}", path.display()))?;
    let mut session: Session = serde_json::from_slice(&data)
        .with_context(|| format!("invalid session {}", path.display()))?;
    session.ensure_name();
    // Migrate older session files that embedded the then-current runtime prompt.
    session
        .messages
        .retain(|message| message.role != crate::model::Role::System);
    for terminal in &mut session.terminals {
        terminal.state = crate::terminal::TerminalState::Disconnected;
    }
    Ok(session)
}
fn generated_name(id: Uuid) -> String {
    format!("session-{}", &id.simple().to_string()[..8])
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                std::fs::metadata(store.path(session.id))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn new_sessions_have_stable_generated_names() {
        let session = Session::new(PathBuf::from("/tmp"), "model".into());
        let expected = format!("session-{}", &session.id.simple().to_string()[..8]);
        assert_eq!(session.name.as_deref(), Some(expected.as_str()));
        assert_eq!(session.display_name(), expected);
    }

    #[tokio::test]
    async fn old_unnamed_sessions_receive_a_name_when_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "model".into());
        session.name = None;
        std::fs::write(
            store.path(session.id),
            serde_json::to_vec_pretty(&session).unwrap(),
        )
        .unwrap();

        let restored = store.load(session.id).await.unwrap();
        assert!(
            restored
                .name
                .as_deref()
                .is_some_and(|name| !name.is_empty())
        );
        assert_eq!(
            restored.name.as_deref(),
            Some(restored.display_name().as_str())
        );
    }

    #[tokio::test]
    async fn runtime_system_guidance_is_never_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "model".into());
        session.messages = vec![
            Message::new(crate::model::Role::System, "stale runtime tools"),
            Message::new(crate::model::Role::User, "hello"),
        ];
        store.save(&mut session).await.unwrap();
        let restored = store.load(session.id).await.unwrap();
        assert_eq!(restored.messages.len(), 1);
        assert_eq!(restored.messages[0].role, crate::model::Role::User);
    }

    #[tokio::test]
    async fn assistant_markdown_survives_save_and_resume_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "model".into());
        let source = "# Result\n\n```rust\nfn main() {}\n```\n\n| A | B |\n|---|---|\n| 1 | 2 |";
        session
            .messages
            .push(Message::new(crate::model::Role::Assistant, source));

        store.save(&mut session).await.unwrap();
        let restored = store.load(session.id).await.unwrap();

        assert_eq!(restored.messages[0].content, source);
    }

    #[tokio::test]
    async fn model_switch_history_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "first".into());
        let mut message = Message::new(crate::model::Role::Assistant, "answer");
        message.provider_state = Some(serde_json::json!({"kind":"provider-state"}));
        session.messages.push(message);
        assert!(session.switch_model("second").unwrap());
        assert!(session.messages[0].provider_state.is_none());
        assert!(!session.switch_model("second").unwrap());
        assert_eq!(session.model_history.len(), 1);
        assert_eq!(session.model_history[0].from, "first");
        assert_eq!(session.model_history[0].to, "second");
        store.save(&mut session).await.unwrap();
        let restored = store.load(session.id).await.unwrap();
        assert_eq!(restored.model, "second");
        assert_eq!(restored.model_history, session.model_history);
    }

    #[test]
    fn compaction_keeps_parallel_results_with_their_assistant() {
        use crate::model::Role;
        let mut messages = vec![
            Message::new(Role::User, "work"),
            Message::new(Role::Assistant, "calling tools"),
            Message::tool("a", "first"),
            Message::tool("b", "second"),
            Message::tool("c", "third"),
        ];
        assert_eq!(compact_messages(&mut messages, 2), 1);
        assert_eq!(messages[1].role, Role::Assistant);
        assert_eq!(messages.len(), 5);
    }

    #[test]
    fn compaction_retains_system_and_tail() {
        let mut messages = vec![Message::new(crate::model::Role::System, "rules")];
        for i in 0..10 {
            messages.push(Message::new(crate::model::Role::User, i.to_string()));
        }
        assert_eq!(compact_messages(&mut messages, 5), 6);
        assert_eq!(messages.first().unwrap().content, "rules");
        assert!(messages.iter().any(|message| message.content == "9"));
    }

    #[tokio::test]
    async fn branch_and_named_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut original = Session::new(dir.path().into(), "test".into());
        store.save(&mut original).await.unwrap();
        let branch = store
            .branch(&original, Some("experiment".into()))
            .await
            .unwrap();
        assert_eq!(branch.parent_id, Some(original.id));
        assert_eq!(
            store.load_reference("experiment").await.unwrap().id,
            branch.id
        );
        let unnamed_branch = store.branch(&original, None).await.unwrap();
        assert_eq!(
            unnamed_branch.name.as_deref(),
            Some(unnamed_branch.display_name().as_str())
        );
        assert_ne!(unnamed_branch.name, original.name);
    }

    #[tokio::test]
    async fn restored_terminal_metadata_is_disconnected() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "test".into());
        session.terminals.push(crate::terminal::TerminalSummary {
            id: crate::terminal::TerminalId(Uuid::new_v4()),
            title: "build".into(),
            state: crate::terminal::TerminalState::Running,
        });
        store.save(&mut session).await.unwrap();
        let restored = store.load(session.id).await.unwrap();
        assert_eq!(
            restored.terminals[0].state,
            crate::terminal::TerminalState::Disconnected
        );
    }
}
