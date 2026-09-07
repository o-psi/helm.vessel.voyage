//! Run-owned obligations coordinated with durable workspace records.
use super::{
    DispositionKind, FinalDecision, FinalOutcome, Obligation, Readiness, RunId, RunLedger,
    store::{RunLedgerStore, RunScope},
};
use crate::{
    subagent::{AgentTree, AgentTreeStore},
    todo::TodoStore,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct Coordinator {
    directory: PathBuf,
    workspace: PathBuf,
    gate: Arc<AsyncMutex<()>>,
}

pub(crate) struct CoordinationGuard {
    file: File,
    _local: OwnedMutexGuard<()>,
}
impl Drop for CoordinationGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// Exclusive lifetime of one writer for this coordinator's persistent agent tree.
#[derive(Debug)]
pub(crate) struct AgentWriterLease(File);
impl Drop for AgentWriterLease {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl Coordinator {
    /// Dedicated private directory beneath an existing trusted data directory.
    pub fn open(directory: PathBuf, workspace: &Path) -> Result<Self> {
        let workspace = workspace.canonicalize()?;
        ensure!(
            workspace.is_dir(),
            "completion workspace is not a directory"
        );
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&directory) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let metadata = std::fs::symlink_metadata(&directory)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "invalid completion coordinator directory"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                metadata.permissions().mode() & 0o077 == 0,
                "completion coordinator directory is not private"
            );
        }
        let directory = directory.canonicalize()?;
        static GATES: OnceLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> = OnceLock::new();
        let mut gates = GATES
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| anyhow::anyhow!("coordinator registry poisoned"))?;
        let gate = gates
            .get(&directory)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let gate = Arc::new(AsyncMutex::new(()));
                gates.insert(directory.clone(), Arc::downgrade(&gate));
                gate
            });
        Ok(Self {
            directory,
            workspace,
            gate,
        })
    }
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) fn transfer_identity(&self) -> (&Path, &Path) {
        (&self.directory, &self.workspace)
    }
    pub(crate) fn acquire_agent_writer(&self) -> Result<AgentWriterLease> {
        let path = self.directory.join("agents.execution.lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(&path)?;
        ensure!(
            file.metadata()?.is_file()
                && !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
            "invalid subagent writer lease"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            ensure!(
                file.metadata()?.permissions().mode() & 0o077 == 0,
                "subagent writer lease is not private"
            );
        }
        file.try_lock().context(
            "session subagent runtime is busy; another owner still holds its execution lease",
        )?;
        Ok(AgentWriterLease(file))
    }
    pub(crate) async fn lock(&self) -> Result<CoordinationGuard> {
        let local = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.gate.clone().lock_owned(),
        )
        .await
        .context("workspace completion coordinator timed out")?;
        let path = self.directory.join("workspace.lock");
        tokio::task::spawn_blocking(move || {
            let mut options = OpenOptions::new();
            options.read(true).write(true).create(true).truncate(false);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = options.open(&path)?;
            ensure!(
                file.metadata()?.is_file()
                    && !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
                "invalid coordinator lock"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                ensure!(
                    file.metadata()?.permissions().mode() & 0o077 == 0,
                    "coordinator lock is not private"
                );
            }
            file.try_lock()
                .context("workspace completion state is busy")?;
            Ok(CoordinationGuard {
                file,
                _local: local,
            })
        })
        .await?
    }
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.directory == other.directory && self.workspace == other.workspace
    }
}

/// Holds cooperating todo/agent/ledger writers out until the caller records its
/// acceptance decision. Never retain this across provider calls, tools or waits.
pub struct ReadinessLease {
    pub readiness: Readiness,
    observed: Readiness,
    handle: RunHandle,
    _guard: CoordinationGuard,
}

impl ReadinessLease {
    /// Checkpoint canonical proposal text before calling this, while this lease
    /// still excludes cooperating writers. Publish acceptance only after success.
    /// No provider, tool, approval or wait belongs inside this boundary.
    /// A cancelled waiter does not release the guard until the write finishes;
    /// its result is uncertain and must be inspected, never blindly repeated.
    pub async fn seal(
        self,
        outcome: FinalOutcome,
        reason: Option<String>,
    ) -> Result<FinalDecision> {
        tokio::task::spawn_blocking(move || {
            let _guard = self._guard;
            let ledger =
                self.handle
                    .store
                    .update(self.handle.run_id, self.observed.revision, |ledger| {
                        ledger.seal(self.observed, outcome, reason)
                    })?;
            Ok(ledger.decision().expect("seal creates decision").clone())
        })
        .await?
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunReference {
    pub session_id: Uuid,
    pub run_id: Uuid,
}

#[derive(Clone, Debug)]
pub struct Review {
    pub revision: u64,
    pub fingerprint: String,
    pub disposition: DispositionKind,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct RunHandle {
    coordinator: Coordinator,
    store: RunLedgerStore,
    run_id: RunId,
    session_id: Uuid,
}
impl RunHandle {
    pub async fn create(coordinator: Coordinator, session_id: Uuid, run_id: Uuid) -> Result<Self> {
        let _guard = coordinator.lock().await?;
        let worker = coordinator.clone();
        let store = tokio::task::spawn_blocking(move || {
            let _guard = _guard;
            let scope = RunScope::new(&worker.workspace, session_id)?;
            let store = RunLedgerStore::open(worker.directory.join("ledgers"), scope)?;
            store.create(&RunLedger::with_id(RunId(run_id)))?;
            Ok::<_, anyhow::Error>(store)
        })
        .await??;
        Ok(Self {
            coordinator,
            store,
            run_id: RunId(run_id),
            session_id,
        })
    }
    /// Known runs fail closed if their durable ledger is missing or corrupt.
    pub async fn resume(coordinator: Coordinator, session_id: Uuid, run_id: Uuid) -> Result<Self> {
        let _guard = coordinator.lock().await?;
        let worker = coordinator.clone();
        let store = tokio::task::spawn_blocking(move || {
            let _guard = _guard;
            let store = RunLedgerStore::open(
                worker.directory.join("ledgers"),
                RunScope::new(&worker.workspace, session_id)?,
            )?;
            store.load(RunId(run_id))?;
            Ok::<_, anyhow::Error>(store)
        })
        .await??;
        Ok(Self {
            coordinator,
            store,
            run_id: RunId(run_id),
            session_id,
        })
    }
    pub fn reference(&self) -> RunReference {
        RunReference {
            session_id: self.session_id,
            run_id: self.run_id.0,
        }
    }
    pub async fn owns(&self, obligation: Obligation) -> Result<bool> {
        let _guard = self.coordinator.lock().await?;
        Ok(self
            .ledger()
            .await?
            .obligations()
            .any(|entry| entry == obligation))
    }
    pub fn run_id(&self) -> Uuid {
        self.run_id.0
    }
    pub fn coordinator(&self) -> &Coordinator {
        &self.coordinator
    }
    async fn ledger(&self) -> Result<RunLedger> {
        let handle = self.clone();
        tokio::task::spawn_blocking(move || handle.store.load(handle.run_id)).await?
    }
    /// Register before publishing a todo or starting a child. Failure after this
    /// commit leaves an unresolved missing obligation, never untracked work.
    pub async fn register(&self, obligation: Obligation) -> Result<()> {
        let _guard = self.coordinator.lock().await?;
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = _guard;
            let ledger = handle.store.load(handle.run_id)?;
            handle.store.update(handle.run_id, ledger.revision(), |l| {
                l.adopt(obligation, l.revision())
            })?;
            Ok(())
        })
        .await?
    }
    async fn records(
        &self,
        ledger: &RunLedger,
        todos: &TodoStore,
        agents: &AgentTreeStore,
    ) -> Result<(crate::todo::TodoList, AgentTree)> {
        ensure!(
            todos
                .coordinator()
                .is_some_and(|c| c.same(&self.coordinator))
                && agents
                    .coordinator()
                    .is_some_and(|c| c.same(&self.coordinator)),
            "completion stores use a different coordinator"
        );
        let todos = todos.snapshot().await?;
        let mut tree = agents.load().await?;
        for obligation in ledger.obligations() {
            if let Obligation::Agent(id) = obligation
                && let std::collections::btree_map::Entry::Vacant(entry) = tree.agents.entry(id)
                && let Some(archived) = agents.get_archived(id).await?
            {
                entry.insert(archived.record);
            }
        }
        Ok((todos, tree))
    }
    pub async fn snapshot(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
        limit: usize,
    ) -> Result<Readiness> {
        Ok(self.readiness_lease(todos, agents, limit).await?.readiness)
    }
    pub async fn readiness_lease(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
        limit: usize,
    ) -> Result<ReadinessLease> {
        let guard = self.coordinator.lock().await?;
        let ledger = self.ledger().await?;
        let (todos, agents) = self.records(&ledger, todos, agents).await?;
        Ok(ReadinessLease {
            readiness: ledger.snapshot(&todos, &agents, limit)?,
            observed: ledger.snapshot(&todos, &agents, super::MAX_OBLIGATIONS)?,
            handle: self.clone(),
            _guard: guard,
        })
    }
    async fn record(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
        obligation: Obligation,
    ) -> Result<serde_json::Value> {
        ensure!(
            todos
                .coordinator()
                .is_some_and(|c| c.same(&self.coordinator))
                && agents
                    .coordinator()
                    .is_some_and(|c| c.same(&self.coordinator)),
            "completion stores use a different coordinator"
        );
        match obligation {
            Obligation::Todo(id) => serde_json::to_value(
                todos
                    .snapshot()
                    .await?
                    .items
                    .get(&id)
                    .context("todo missing")?,
            )
            .map_err(Into::into),
            Obligation::Agent(id) => {
                let record = match agents.get(id).await? {
                    Some(record) => record,
                    None => {
                        agents
                            .get_archived(id)
                            .await?
                            .context("agent missing")?
                            .record
                    }
                };
                Ok(serde_json::to_value(record)?)
            }
        }
    }
    /// Historical decision only; shared records may have changed afterwards.
    pub async fn decision(&self) -> Result<Option<FinalDecision>> {
        let _guard = self.coordinator.lock().await?;
        Ok(self.ledger().await?.decision().cloned())
    }
    /// Recheck that the exact accepted records still match, under the writer
    /// coordinator. A later shared-record edit never silently reopens the run.
    pub async fn validate_final_decision(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
    ) -> Result<FinalDecision> {
        let _guard = self.coordinator.lock().await?;
        let mut ledger = self.ledger().await?;
        let decision = ledger
            .decision()
            .context("completion run is not sealed")?
            .clone();
        ledger.state = super::LedgerState::Open;
        ledger.revision = decision.readiness.revision;
        let (todos, agents) = self.records(&ledger, todos, agents).await?;
        ensure!(
            ledger.snapshot(&todos, &agents, super::MAX_OBLIGATIONS)? == decision.readiness,
            "owned completion records changed after final decision"
        );
        Ok(decision)
    }
    pub async fn read_owned(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
        obligation: Obligation,
    ) -> Result<serde_json::Value> {
        let _guard = self.coordinator.lock().await?;
        ensure!(
            self.ledger()
                .await?
                .obligations()
                .any(|entry| entry == obligation),
            "record is not owned by this run"
        );
        self.record(todos, agents, obligation).await
    }
    pub async fn adopt_existing(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
        obligation: Obligation,
        expected_revision: u64,
    ) -> Result<()> {
        let _guard = self.coordinator.lock().await?;
        let ledger = self.ledger().await?;
        ledger.ensure_open()?;
        ensure!(
            ledger.revision() == expected_revision,
            "stale completion ledger revision"
        );
        if ledger.obligations().any(|entry| entry == obligation) {
            return Ok(());
        }
        self.record(todos, agents, obligation).await?;
        let obligations = match obligation {
            Obligation::Todo(_) => vec![obligation],
            Obligation::Agent(id) => agents
                .adoption_subtree(id, super::MAX_OBLIGATIONS)
                .await?
                .into_iter()
                .map(Obligation::Agent)
                .collect(),
        };
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = _guard;
            handle
                .store
                .update(handle.run_id, expected_revision, |ledger| {
                    for obligation in obligations {
                        ledger.adopt(obligation, ledger.revision())?;
                    }
                    Ok(())
                })?;
            Ok(())
        })
        .await?
    }
    pub async fn account(
        &self,
        todos: &TodoStore,
        agents: &AgentTreeStore,
        obligation: Obligation,
        review: Review,
    ) -> Result<()> {
        let _guard = self.coordinator.lock().await?;
        let ledger = self.ledger().await?;
        let (todos, agents) = self.records(&ledger, todos, agents).await?;
        ensure!(
            ledger.snapshot(&todos, &agents, 0)?.fingerprint == review.fingerprint,
            "completion records changed since review"
        );
        let Review {
            revision: expected_revision,
            disposition: kind,
            reason,
            ..
        } = review;
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = _guard;
            handle.store.update(
                handle.run_id,
                expected_revision,
                |ledger| match obligation {
                    Obligation::Todo(id) => ledger.account_todo(
                        todos.items.get(&id).context("owned todo missing")?,
                        kind,
                        reason,
                        expected_revision,
                    ),
                    Obligation::Agent(id) => ledger.account_agent(
                        agents.agents.get(&id).context("owned agent missing")?,
                        kind,
                        reason,
                        expected_revision,
                    ),
                },
            )?;
            Ok(())
        })
        .await?
    }
}
