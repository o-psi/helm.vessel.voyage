mod checkpoint;
pub use checkpoint::SessionCheckpoint;

mod outcomes;
pub use outcomes::RunSummary;

use crate::{
    config::default_data_dir,
    model::{Message, Usage},
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::fs;

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
    /// Optimistic concurrency version; legacy files start at zero.
    #[serde(default)]
    pub revision: u64,
    #[serde(skip)]
    loaded_from: Option<PathBuf>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub workspace: PathBuf,
    pub model: String,
    #[serde(default)]
    pub model_history: Vec<ModelChange>,
    #[serde(default)]
    pub completion_runs: Vec<crate::completion::runtime::RunReference>,
    #[serde(default)]
    pub run_summaries: Vec<RunSummary>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title_state: Option<TitleState>,
    #[serde(default)]
    pub parent_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workflow_runs: Vec<crate::workflow::Invocation>,
    pub messages: Vec<Message>,
    pub usage: Usage,
    #[serde(default)]
    pub terminals: Vec<crate::terminal::TerminalSummary>,
}

/// Automatic-title lifecycle is independent of compacted conversation history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TitleState {
    pub completed_runs: u64,
    pub automatic: bool,
    /// Last generated name also detects direct caller edits of the public name field.
    pub generated: String,
    #[serde(default)]
    pub usage: Usage,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelChange {
    pub from: String,
    pub to: String,
    pub changed_at: DateTime<Utc>,
}

impl Session {
    /// Replace provisional frontend history with rejected-run canonical state.
    /// Inputs accepted after that snapshot remain visible and are not duplicated.
    pub fn recover_context_failure(
        &mut self,
        recovery: &crate::agent::CanonicalRecovery,
    ) -> Result<()> {
        let input_tokens = self
            .usage
            .input_tokens
            .checked_add(recovery.usage.input_tokens)
            .context("session input usage overflow during recovery")?;
        let output_tokens = self
            .usage
            .output_tokens
            .checked_add(recovery.usage.output_tokens)
            .context("session output usage overflow during recovery")?;
        let mut messages = recovery.messages.clone();
        for message in &self.messages {
            if let Some(receipt) = &message.steering
                && receipt.status == crate::model::SteeringStatus::Queued
                && !messages.iter().any(|other| {
                    other
                        .steering
                        .as_ref()
                        .is_some_and(|other| other.id == receipt.id)
                })
            {
                messages.push(message.clone());
            }
        }
        self.replace_messages(messages);
        self.usage.input_tokens = input_tokens;
        self.usage.output_tokens = output_tokens;
        Ok(())
    }

    pub fn new(workspace: PathBuf, model: String) -> Self {
        let now = Utc::now();
        let id = Uuid::new_v4();
        Self {
            id,
            revision: 0,
            loaded_from: None,
            created_at: now,
            updated_at: now,
            workspace,
            model,
            model_history: Vec::new(),
            completion_runs: Vec::new(),
            run_summaries: Vec::new(),
            name: Some(generated_name(id)),
            title_state: Some(TitleState {
                completed_runs: 0,
                automatic: true,
                generated: generated_name(id),
                usage: Usage::default(),
            }),
            parent_id: None,
            workflow_runs: Vec::new(),
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

    fn title_state_mut(&mut self) -> &mut TitleState {
        let fallback = generated_name(self.id);
        let name = self.display_name();
        self.title_state.get_or_insert_with(|| TitleState {
            completed_runs: 0,
            automatic: name == fallback,
            generated: fallback,
            usage: Usage::default(),
        })
    }

    pub fn set_name(&mut self, name: String) {
        self.title_state_mut().automatic = false;
        self.name = Some(name);
    }

    pub fn title_due_after_turn(&self) -> bool {
        let (automatic, count, generated) = self.title_state.as_ref().map_or_else(
            || (true, 0, generated_name(self.id)),
            |state| {
                (
                    state.automatic,
                    state.completed_runs,
                    state.generated.clone(),
                )
            },
        );
        automatic
            && self.display_name() == generated
            && count.checked_add(1).is_some_and(is_title_checkpoint)
    }

    /// Called once after a successful top-level run, never for tool/model iterations.
    pub fn record_completed_turn(&mut self) {
        let state = self.title_state_mut();
        state.completed_runs = state.completed_runs.saturating_add(1);
    }

    pub fn apply_generated_title(&mut self, result: crate::titles::TitleResult) {
        // Auxiliary usage remains separate from conversation usage so it cannot
        // overflow or distort the main run's accounting.
        let name = self.display_name();
        let state = self.title_state_mut();
        match (
            state
                .usage
                .input_tokens
                .checked_add(result.usage.input_tokens),
            state
                .usage
                .output_tokens
                .checked_add(result.usage.output_tokens),
        ) {
            (Some(input), Some(output)) => {
                state.usage.input_tokens = input;
                state.usage.output_tokens = output;
            }
            _ => tracing::warn!("title usage accounting overflowed"),
        }
        if state.automatic
            && state.generated == name
            && let Some(title) = result.title
        {
            state.generated = title.clone();
            self.name = Some(title);
        }
    }

    pub fn clear_conversation(&mut self) {
        self.messages.clear();
        self.run_summaries.clear();
        let automatic = self.title_state_mut().automatic
            && self
                .title_state
                .as_ref()
                .is_some_and(|state| state.generated == self.display_name());
        let fallback = generated_name(self.id);
        let state = self.title_state_mut();
        state.completed_runs = 0;
        if automatic {
            state.generated = fallback.clone();
            self.name = Some(fallback);
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
    execution: Option<Arc<SessionLease>>,
}
impl Default for SessionStore {
    fn default() -> Self {
        Self {
            directory: default_data_dir().join("sessions"),
            execution: None,
        }
    }
}

impl SessionStore {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            execution: None,
        }
    }
    /// Acquire before loading a session, and retain through execution and cleanup.
    /// Non-reentrant: while holding this lease use the `_with_lease` operations.
    /// Waiting is cancellation-safe and never blocks a Tokio worker on an OS lock.
    pub async fn acquire_execution(&self, id: Uuid) -> Result<SessionLease> {
        let directory = prepare_directory(&self.directory)?;
        let path = directory.join(format!(".{id}.lock"));
        let file = open_regular(&path, true)?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            match file.try_lock() {
                Ok(()) => {
                    return Ok(SessionLease {
                        directory,
                        id,
                        _file: file,
                    });
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    anyhow::ensure!(
                        tokio::time::Instant::now() < deadline,
                        "session busy; another frontend owns execution"
                    );
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
    }

    /// Cloneable execution owner. Every clone retains the stable sidecar lock.
    /// A different ID creates a separate owner; existing clones are never rebound.
    pub async fn with_execution(&self, id: Uuid) -> Result<Self> {
        if self.execution.as_ref().is_some_and(|lease| lease.id == id) {
            return Ok(self.clone());
        }
        let lease = self.acquire_execution(id).await?;
        Ok(Self {
            directory: self.directory.clone(),
            execution: Some(Arc::new(lease)),
        })
    }

    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn transfer_source_path(&self, id: Uuid) -> Result<PathBuf> {
        let lease = self
            .execution
            .as_ref()
            .context("transfer requires session execution ownership")?;
        self.check_lease(id, lease)?;
        Ok(lease.directory.join(format!("{id}.json")))
    }

    pub fn owned_session_id(&self) -> Option<Uuid> {
        self.execution.as_ref().map(|lease| lease.id)
    }

    /// Resolve only the identity without ownership, then reload authoritative
    /// content under the lease. Never return the pre-lock snapshot for execution.
    pub async fn load_owned(&self, reference: &str) -> Result<(Self, Session)> {
        let initial = self.load_reference(reference).await?;
        let owner = self.with_execution(initial.id).await?;
        let current = owner.load(initial.id).await?;
        let resolved = owner.load_reference(reference).await?;
        anyhow::ensure!(
            resolved.id == current.id,
            "session reference changed while acquiring ownership"
        );
        Ok((owner, current))
    }

    pub async fn save(&self, session: &mut Session) -> Result<()> {
        if let Some(lease) = &self.execution
            && lease.id == session.id
        {
            return self.save_with_lease(session, lease).await;
        }
        let lease = self.acquire_execution(session.id).await?;
        self.save_with_lease(session, &lease).await
    }

    /// Compare and replace under an existing execution lease. Caller state is only
    /// changed after commit. The bounded synchronous commit has no cancellation points.
    pub async fn save_with_lease(&self, session: &mut Session, lease: &SessionLease) -> Result<()> {
        self.check_lease(session.id, lease)?;
        let destination = lease.directory.join(format!("{}.json", session.id));
        if let Some(source) = &session.loaded_from {
            anyhow::ensure!(
                source == &destination,
                "session was loaded from another path; branch it instead"
            );
        }
        match read_session(&destination) {
            Ok(current) => {
                anyhow::ensure!(session.loaded_from.is_some(), "session ID already exists");
                anyhow::ensure!(
                    current.revision == session.revision,
                    "stale session revision"
                );
            }
            Err(error) if is_not_found(&error) => {
                anyhow::ensure!(
                    session.loaded_from.is_none() && session.revision == 0,
                    "session was deleted; refusing resurrection"
                );
            }
            Err(error) => return Err(error),
        }
        let mut next = session.clone();
        next.revision = session
            .revision
            .checked_add(1)
            .context("session revision exhausted")?;
        next.ensure_name();
        next.updated_at = Utc::now();
        next.loaded_from = Some(destination.clone());
        let mut persisted = next.clone();
        // Runtime system guidance must never become durable conversation history.
        persisted
            .messages
            .retain(|message| message.role != crate::model::Role::System);
        persisted.reanchor_run_summaries();
        let bytes = serde_json::to_vec_pretty(&persisted)?;
        anyhow::ensure!(
            bytes.len() as u64 <= MAX_SESSION_BYTES,
            "session exceeds size limit"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(&lease.directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        // Validate again before replacement; never follow a destination symlink.
        reject_symlinks(&destination)?;
        // Retain a private rollback image until the directory entry is durable.
        #[cfg(unix)]
        let mut rollback = match open_regular(&destination, false) {
            Ok(mut previous) => {
                let mut backup = tempfile::NamedTempFile::new_in(&lease.directory)?;
                std::io::copy(&mut previous, &mut backup)?;
                backup.as_file().sync_all()?;
                Some(backup)
            }
            Err(error) if is_not_found(&error) => None,
            Err(error) => return Err(error),
        };
        // Unix supports syncing directory entries. Windows cannot open a
        // directory with File::open; the file itself is synced before replacement.
        // Non-Unix platforms do not promise directory-entry crash durability.
        #[cfg(unix)]
        let directory = std::fs::File::open(&lease.directory)?;
        temporary
            .persist(&destination)
            .map_err(|error| error.error)?;
        #[cfg(unix)]
        if let Err(error) = directory.sync_all() {
            if let Some(backup) = rollback.take() {
                backup
                    .persist(&destination)
                    .map_err(|error| error.error)
                    .context("directory sync failed and session rollback failed")?;
            } else {
                std::fs::remove_file(&destination)
                    .context("directory sync failed and new-session rollback failed")?;
            }
            // Failing storage cannot promise crash durability of the rollback.
            let _ = directory.sync_all();
            return Err(error).context("session directory sync failed; snapshot rolled back");
        }
        *session = next;
        Ok(())
    }

    fn check_lease(&self, id: Uuid, lease: &SessionLease) -> Result<()> {
        reject_symlinks(&self.directory)?;
        anyhow::ensure!(
            id == lease.id() && std::fs::canonicalize(&self.directory)? == lease.directory(),
            "session lease belongs to another directory or ID"
        );
        Ok(())
    }
    pub async fn load(&self, id: Uuid) -> Result<Session> {
        load_path(&self.path(id)).await
    }
    pub async fn load_reference(&self, reference: &str) -> Result<Session> {
        let direct = Path::new(reference);
        if std::fs::symlink_metadata(direct).is_ok() {
            // Reject foreign snapshots before the caller can execute tools. They
            // cannot be saved through this store's revision/ownership boundary.
            // Validate the path before canonicalizing so symlinks stay forbidden.
            reject_symlinks(direct)?;
            reject_symlinks(&self.directory)?;
            let source = std::fs::canonicalize(direct)?;
            let directory = std::fs::canonicalize(&self.directory).ok();
            anyhow::ensure!(
                directory.as_deref() == source.parent(),
                "session path belongs to another store; resume it using its original session store"
            );
            let session = load_path(direct).await?;
            anyhow::ensure!(
                source.file_name() == Some(std::ffi::OsStr::new(&format!("{}.json", session.id))),
                "session path does not match its canonical ID filename"
            );
            return Ok(session);
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
        reject_symlinks(&self.directory)?;
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
        if let Some(lease) = &self.execution
            && lease.id == id
        {
            return self.delete_with_lease(id, lease).await;
        }
        let lease = self.acquire_execution(id).await?;
        self.delete_with_lease(id, &lease).await
    }

    pub async fn delete_with_lease(&self, id: Uuid, lease: &SessionLease) -> Result<()> {
        self.check_lease(id, lease)?;
        let path = lease.directory.join(format!("{id}.json"));
        match read_session(&path) {
            Ok(_) => std::fs::remove_file(&path)?,
            Err(error) if is_not_found(&error) => return Ok(()),
            Err(error) => return Err(error),
        }
        #[cfg(unix)]
        if let Err(error) = std::fs::File::open(&lease.directory).and_then(|dir| dir.sync_all()) {
            tracing::warn!(%error, "session deleted but directory sync failed");
        }
        // The sidecar is deliberately NEVER unlinked, even on deletion.
        Ok(())
    }

    pub async fn branch(&self, source: &Session, name: Option<String>) -> Result<Session> {
        Ok(self.branch_owned(source, name).await?.1)
    }

    /// Publish the new branch only after acquiring its execution owner.
    pub async fn branch_owned(
        &self,
        source: &Session,
        name: Option<String>,
    ) -> Result<(Self, Session)> {
        let now = Utc::now();
        let mut branch = source.clone();
        branch.id = Uuid::new_v4();
        branch.revision = 0;
        branch.loaded_from = None;
        branch.created_at = now;
        branch.updated_at = now;
        branch.parent_id = Some(source.id);
        branch.completion_runs.clear();
        branch.title_state = None;
        branch.name = name
            .filter(|name| !name.trim().is_empty())
            .or_else(|| Some(generated_name(branch.id)));
        let manual = branch.name.as_deref() != Some(&generated_name(branch.id));
        branch.title_state_mut().automatic = !manual;
        let owner = self.with_execution(branch.id).await?;
        owner.save(&mut branch).await?;
        Ok((owner, branch))
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
        for (index, message) in session.messages.iter().enumerate() {
            use std::fmt::Write as _;
            let _ = writeln!(output, "## {:?}\n\n{}\n", message.role, message.content);
            if let Some(classification) = session.assistant_classification(index) {
                let _ = writeln!(output, "Run output classification: {classification}\n");
            }
            if let Some(receipt) = &message.steering {
                let _ = writeln!(output, "Steering delivery: {:?}\n", receipt.status);
            }
        }
        fs::write(path, output)
            .await
            .with_context(|| format!("failed to export session to {}", path.display()))
    }
    fn path(&self, id: Uuid) -> PathBuf {
        self.directory.join(format!("{id}.json"))
    }
}

/// Exclusive OS ownership of one session ID in one canonical store directory.
/// Send + Sync, with no runtime/thread affinity; dropping releases the lock.
/// Sidecar files are permanent so independent processes always lock the same inode.
#[derive(Debug)]
pub struct SessionLease {
    directory: PathBuf,
    id: Uuid,
    _file: std::fs::File,
}

impl SessionLease {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn id(&self) -> Uuid {
        self.id
    }
}

const MAX_SESSION_BYTES: u64 = 64 * 1024 * 1024;

fn is_not_found(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<std::io::Error>()
        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
}

pub(crate) fn reject_symlinks(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component);
        // A Windows drive/UNC prefix is not a complete filesystem ancestor.
        // In particular, canonicalize returns verbatim paths whose bare
        // `\\?\C:` prefix cannot be queried until RootDir has been appended.
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        match std::fs::symlink_metadata(&prefix) {
            Ok(meta) => anyhow::ensure!(
                !meta.file_type().is_symlink() || trusted_system_alias(&prefix, &meta),
                "symlink path rejected: {}",
                prefix.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

// macOS exposes /var, /tmp and /etc through root-managed aliases. These are
// part of the OS namespace, not operator-selected storage redirects. Do not
// generalize this exception to arbitrary root-owned or canonicalizable links.
fn trusted_system_alias(path: &Path, metadata: &std::fs::Metadata) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::MetadataExt;
        let target = match path.to_str() {
            Some("/var") => "/private/var",
            Some("/tmp") => "/private/tmp",
            Some("/etc") => "/private/etc",
            _ => return false,
        };
        metadata.uid() == 0
            && std::fs::canonicalize(path).is_ok_and(|actual| actual == Path::new(target))
            && std::fs::symlink_metadata("/")
                .is_ok_and(|root| root.uid() == 0 && root.mode() & 0o022 == 0)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (path, metadata);
        false
    }
}

fn prepare_directory(path: &Path) -> Result<PathBuf> {
    reject_symlinks(path)?;
    std::fs::create_dir_all(path)?;
    reject_symlinks(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        anyhow::ensure!(
            std::fs::metadata(path)?.uid() == unsafe { libc::geteuid() },
            "session directory is not owned by current user"
        );
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(std::fs::canonicalize(path)?)
}

fn open_regular(path: &Path, lock: bool) -> Result<std::fs::File> {
    reject_symlinks(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(lock).create(lock);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let meta = file.metadata()?;
    anyhow::ensure!(meta.is_file(), "not a regular file: {}", path.display());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(meta.nlink() == 1, "hard-linked session file rejected");
        if lock {
            anyhow::ensure!(
                meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o600,
                "session lock must be owner-only"
            );
        }
    }
    Ok(file)
}

async fn load_path(path: &Path) -> Result<Session> {
    read_session(path)
}

fn read_session(path: &Path) -> Result<Session> {
    let file = open_regular(path, false)?;
    anyhow::ensure!(
        file.metadata()?.len() <= MAX_SESSION_BYTES,
        "session exceeds size limit"
    );
    let mut data = Vec::new();
    file.take(MAX_SESSION_BYTES + 1).read_to_end(&mut data)?;
    anyhow::ensure!(
        data.len() as u64 <= MAX_SESSION_BYTES,
        "session exceeds size limit"
    );
    let mut session: Session = serde_json::from_slice(&data)
        .with_context(|| format!("invalid session {}", path.display()))?;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("invalid session filename")?;
    let id = Uuid::parse_str(stem).context("session filename must be a UUID")?;
    anyhow::ensure!(session.id == id, "session ID does not match filename");
    let mut run_ids = std::collections::BTreeSet::new();
    anyhow::ensure!(
        session
            .completion_runs
            .iter()
            .all(|reference| reference.session_id == session.id
                && !reference.run_id.is_nil()
                && run_ids.insert(reference.run_id)),
        "invalid or duplicate completion run reference"
    );
    session.loaded_from = Some(std::fs::canonicalize(path)?);
    session.ensure_name();
    for message in &mut session.messages {
        if let Some(receipt) = &mut message.steering
            && receipt.status == crate::model::SteeringStatus::Queued
        {
            receipt.status = crate::model::SteeringStatus::UnknownAfterRestart;
        }
    }
    // Migrate older session files that embedded the then-current runtime prompt.
    session
        .messages
        .retain(|message| message.role != crate::model::Role::System);
    session.recover_run_summaries();
    for terminal in &mut session.terminals {
        terminal.state = crate::terminal::TerminalState::Disconnected;
    }
    Ok(session)
}
fn is_title_checkpoint(count: u64) -> bool {
    let (mut previous, mut current) = (0_u64, 1_u64);
    while current < count {
        let Some(next) = previous.checked_add(current) else {
            return false;
        };
        previous = current;
        current = next;
    }
    count != 0 && current == count
}

fn generated_name(id: Uuid) -> String {
    format!("session-{}", &id.simple().to_string()[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    fn title_result(title: Option<&str>) -> crate::titles::TitleResult {
        crate::titles::TitleResult {
            title: title.map(str::to_owned),
            usage: Usage {
                input_tokens: 4,
                output_tokens: 2,
            },
        }
    }

    #[test]
    fn title_checkpoints_are_fibonacci_and_overflow_safe() {
        let mut session = Session::new(PathBuf::from("."), "main-model".into());
        let mut checkpoints = Vec::new();
        for turn in 1..=100 {
            if session.title_due_after_turn() {
                checkpoints.push(turn);
            }
            session.record_completed_turn();
            session.apply_generated_title(title_result(Some("Useful title")));
            compact_messages(&mut session.messages, 2);
        }
        assert_eq!(checkpoints, [1, 2, 3, 5, 8, 13, 21, 34, 55, 89]);
        assert!(!is_title_checkpoint(0));
        assert!(!is_title_checkpoint(u64::MAX));
        session.title_state_mut().completed_runs = u64::MAX;
        assert!(!session.title_due_after_turn());
        session.record_completed_turn();
        assert_eq!(session.title_state_mut().completed_runs, u64::MAX);
        assert_eq!(session.model, "main-model");
        assert!(session.model_history.is_empty());
        assert_eq!(session.usage.input_tokens, 0);
        assert_eq!(session.title_state_mut().usage.input_tokens, 400);
    }

    #[tokio::test]
    async fn title_state_survives_resume_and_branch_clear_reset_schedule() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().into());
        let mut session = Session::new(dir.path().into(), "main".into());
        for _ in 0..3 {
            session.record_completed_turn();
        }
        session.apply_generated_title(title_result(Some("Generated")));
        store.save(&mut session).await.unwrap();
        let mut loaded = store.load(session.id).await.unwrap();
        assert_eq!(loaded.display_name(), "Generated");
        assert!(!loaded.title_due_after_turn());
        loaded.record_completed_turn();
        assert!(loaded.title_due_after_turn());
        let mut branch = store.branch(&loaded, None).await.unwrap();
        assert!(branch.title_due_after_turn());
        branch.record_completed_turn();
        branch.apply_generated_title(title_result(Some("Branch title")));
        branch.clear_conversation();
        assert!(branch.title_due_after_turn());
        assert_eq!(branch.display_name(), generated_name(branch.id));
        let mut manual = store.branch(&loaded, Some("Chosen".into())).await.unwrap();
        manual.clear_conversation();
        assert!(!manual.title_due_after_turn());
        assert_eq!(manual.display_name(), "Chosen");
    }

    #[test]
    fn title_manual_names_and_legacy_sessions_are_preserved() {
        let mut session = Session::new(PathBuf::from("."), "main".into());
        // Even explicitly naming it exactly the generated fallback disables updates.
        session.set_name(session.display_name());
        assert!(!session.title_due_after_turn());
        let original = session.display_name();
        session.apply_generated_title(title_result(Some("ignored")));
        assert_eq!(session.display_name(), original);
        session.title_state = None;
        session.name = Some("Legacy custom".into());
        assert!(!session.title_due_after_turn());
        session.record_completed_turn();
        session.apply_generated_title(title_result(Some("ignored")));
        session.clear_conversation();
        assert_eq!(session.display_name(), "Legacy custom");
        session.title_state = None;
        session.name = Some(generated_name(session.id));
        assert!(session.title_due_after_turn());
        session.record_completed_turn();
        session.apply_generated_title(title_result(None));
        assert_eq!(session.display_name(), generated_name(session.id));
        assert_eq!(session.title_state_mut().usage.output_tokens, 2);
        // Public-field edits are also treated as manual.
        session.name = Some("Caller name".into());
        session.apply_generated_title(title_result(Some("ignored")));
        assert!(!session.title_due_after_turn());
        assert_eq!(session.display_name(), "Caller name");
    }

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
    #[tokio::test]
    async fn steering_recovery_preserves_text_and_reports_unknown_delivery() {
        use crate::model::SteeringStatus;
        let directory = tempfile::tempdir().unwrap();
        let store = SessionStore::new(directory.path().join("sessions"));
        let mut session = Session::new(directory.path().into(), "test".into());
        session
            .messages
            .push(Message::steering("pending before crash"));
        let mut applied = Message::steering("already applied");
        applied.steering.as_mut().unwrap().status = SteeringStatus::Applied;
        session.messages.push(applied);
        store.save(&mut session).await.unwrap();
        let saved = store.load(session.id).await.unwrap();
        assert_eq!(saved.messages.len(), 2);
        assert_eq!(saved.messages[0].content, "pending before crash");
        assert_eq!(
            saved.messages[0].steering.as_ref().unwrap().status,
            SteeringStatus::UnknownAfterRestart
        );
        assert_eq!(
            saved.messages[1].steering.as_ref().unwrap().status,
            SteeringStatus::Applied
        );
        assert_eq!(
            saved.messages[0].steering.as_ref().unwrap().id,
            session.messages[0].steering.as_ref().unwrap().id
        );
    }
}

#[cfg(test)]
#[path = "session/concurrency_tests.rs"]
mod concurrency_tests;
