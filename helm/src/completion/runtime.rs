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

/// Exclusive lifetime of one cooperating subagent writer for this workspace.
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
            "workspace subagent runtime is busy; another owner still holds its execution lease",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        completion::UnresolvedReason,
        subagent::{
            AgentBudget, AgentPolicy, ApprovalPolicy, ExecutionContext, RuntimeLimits,
            SpawnRequest, SubagentExecutor, SubagentResult, SubagentRuntime,
        },
        todo::{EntryKind, NewTodo, Priority, TodoScope, TodoStatus},
    };
    use std::collections::BTreeSet;
    struct Fixture {
        _root: tempfile::TempDir,
        coordinator: Coordinator,
        todos: TodoStore,
        agents: AgentTreeStore,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let coordinator =
                Coordinator::open(root.path().join("completion"), root.path()).unwrap();
            let todos = TodoStore::new(
                root.path().join("todos/list.json"),
                TodoScope::workspace(root.path().canonicalize().unwrap()),
            )
            .with_coordinator(coordinator.clone());
            let agents = AgentTreeStore::new(root.path().join("agents/tree.json"))
                .with_coordinator(coordinator.clone());
            Self {
                _root: root,
                coordinator,
                todos,
                agents,
            }
        }
        async fn run(&self) -> RunHandle {
            RunHandle::create(self.coordinator.clone(), Uuid::new_v4(), Uuid::new_v4())
                .await
                .unwrap()
        }
        async fn snapshot(&self, run: &RunHandle) -> Readiness {
            run.snapshot(&self.todos, &self.agents, 64).await.unwrap()
        }
    }
    fn todo(title: &str) -> NewTodo {
        NewTodo {
            title: title.into(),
            description: String::new(),
            priority: Priority::Normal,
            order: None,
            assignees: BTreeSet::new(),
        }
    }
    fn review(snapshot: Readiness, disposition: DispositionKind) -> Review {
        Review {
            revision: snapshot.revision,
            fingerprint: snapshot.fingerprint,
            disposition,
            reason: "verified fixture evidence".into(),
        }
    }
    #[tokio::test]
    async fn final_seal_survives_restart_and_rejects_late_registration() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let mut lease = run
            .readiness_lease(&fixture.todos, &fixture.agents, 0)
            .await
            .unwrap();
        // Public display data is not authority to alter the private observation.
        lease.readiness.fingerprint = "untrusted display edit".into();
        let writer = run.clone();
        let late = tokio::spawn(async move {
            writer
                .register(Obligation::Todo(crate::todo::TodoId::new()))
                .await
        });
        tokio::task::yield_now().await;
        assert!(!late.is_finished());
        let decision = lease.seal(FinalOutcome::Completed, None).await.unwrap();
        assert!(decision.readiness.ready());
        assert_ne!(decision.readiness.fingerprint, "untrusted display edit");
        assert!(
            late.await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("sealed")
        );
        let reference = run.reference();
        let resumed = RunHandle::resume(
            fixture.coordinator.clone(),
            reference.session_id,
            reference.run_id,
        )
        .await
        .unwrap();
        assert_eq!(resumed.decision().await.unwrap(), Some(decision.clone()));
        assert_eq!(
            resumed
                .validate_final_decision(&fixture.todos, &fixture.agents)
                .await
                .unwrap(),
            decision
        );
        assert!(
            resumed
                .readiness_lease(&fixture.todos, &fixture.agents, 10)
                .await
                .unwrap()
                .seal(FinalOutcome::Completed, None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn registered_before_seal_cannot_be_hidden_by_display_cap_or_stale_snapshot() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let old = fixture.snapshot(&run).await;
        let id = crate::todo::TodoId::new();
        run.register(Obligation::Todo(id)).await.unwrap();
        let lease = run
            .readiness_lease(&fixture.todos, &fixture.agents, 0)
            .await
            .unwrap();
        assert_ne!(lease.readiness.fingerprint, old.fingerprint);
        assert!(lease.readiness.unresolved.is_empty());
        assert!(!lease.readiness.ready());
        assert!(lease.seal(FinalOutcome::Completed, None).await.is_err());
        assert!(run.decision().await.unwrap().is_none());
        let decision = run
            .readiness_lease(&fixture.todos, &fixture.agents, 0)
            .await
            .unwrap()
            .seal(
                FinalOutcome::Interrupted,
                Some("owned record unavailable after shutdown".into()),
            )
            .await
            .unwrap();
        assert_eq!(decision.readiness.unresolved.len(), 1);
        assert_eq!(
            decision.readiness.unresolved[0].obligation,
            Obligation::Todo(id)
        );
        assert_eq!(decision.readiness.omitted_unresolved, 0);
    }

    #[tokio::test]
    async fn sealed_incomplete_is_historical_and_later_record_edits_fail_validation() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let item = fixture
            .todos
            .create_registered(todo("deferred"), Some(&run))
            .await
            .unwrap();
        run.account(
            &fixture.todos,
            &fixture.agents,
            Obligation::Todo(item.id),
            review(
                fixture.snapshot(&run).await,
                DispositionKind::DeferredWithImpact,
            ),
        )
        .await
        .unwrap();
        let lease = run
            .readiness_lease(&fixture.todos, &fixture.agents, 10)
            .await
            .unwrap();
        assert!(lease.readiness.ready());
        assert!(lease.seal(FinalOutcome::Completed, None).await.is_err());
        let decision = run
            .readiness_lease(&fixture.todos, &fixture.agents, 10)
            .await
            .unwrap()
            .seal(FinalOutcome::Incomplete, Some("waiting on operator".into()))
            .await
            .unwrap();
        assert_eq!(
            decision.readiness.incomplete_obligations,
            vec![Obligation::Todo(item.id)]
        );
        let fresh = fixture.snapshot(&run).await;
        assert!(
            run.account(
                &fixture.todos,
                &fixture.agents,
                Obligation::Todo(item.id),
                review(fresh.clone(), DispositionKind::DeferredWithImpact)
            )
            .await
            .is_err()
        );
        assert!(
            run.adopt_existing(
                &fixture.todos,
                &fixture.agents,
                Obligation::Todo(item.id),
                fresh.revision
            )
            .await
            .is_err()
        );
        assert!(run.register(Obligation::Todo(item.id)).await.is_err());
        assert!(
            run.validate_final_decision(&fixture.todos, &fixture.agents)
                .await
                .is_ok()
        );
        fixture
            .todos
            .append_note(
                item.id,
                EntryKind::Progress,
                "later independent edit".into(),
                None,
            )
            .await
            .unwrap();
        assert!(
            run.validate_final_decision(&fixture.todos, &fixture.agents)
                .await
                .is_err()
        );
        assert_eq!(run.decision().await.unwrap(), Some(decision));
    }

    #[tokio::test]
    async fn record_mutation_waits_until_seal_and_cannot_rewrite_decision() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let item = fixture
            .todos
            .create_registered(todo("deferred"), Some(&run))
            .await
            .unwrap();
        run.account(
            &fixture.todos,
            &fixture.agents,
            Obligation::Todo(item.id),
            review(
                fixture.snapshot(&run).await,
                DispositionKind::DeferredWithImpact,
            ),
        )
        .await
        .unwrap();
        let lease = run
            .readiness_lease(&fixture.todos, &fixture.agents, 10)
            .await
            .unwrap();
        let store = fixture.todos.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let edit = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            store
                .append_note(
                    item.id,
                    EntryKind::Progress,
                    "independent later edit".into(),
                    None,
                )
                .await
        });
        started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert!(!edit.is_finished());
        let decision = lease
            .seal(FinalOutcome::Incomplete, Some("explicitly deferred".into()))
            .await
            .unwrap();
        edit.await.unwrap().unwrap();
        assert_eq!(run.decision().await.unwrap(), Some(decision));
        assert!(
            run.validate_final_decision(&fixture.todos, &fixture.agents)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn uncommitted_lease_and_failed_seal_never_become_completed() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        drop(
            run.readiness_lease(&fixture.todos, &fixture.agents, 10)
                .await
                .unwrap(),
        );
        assert!(run.decision().await.unwrap().is_none());
        let lease = run
            .readiness_lease(&fixture.todos, &fixture.agents, 10)
            .await
            .unwrap();
        let path = fixture
            ._root
            .path()
            .join("completion/ledgers")
            .join(format!("{}-{}.json", run.session_id, run.run_id.0));
        let backup = path.with_extension("backup");
        let original = std::fs::read(&path).unwrap();
        std::fs::rename(&path, &backup).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(lease.seal(FinalOutcome::Completed, None).await.is_err());
        assert_eq!(std::fs::read(&backup).unwrap(), original);
        std::fs::remove_dir(&path).unwrap();
        std::fs::rename(&backup, &path).unwrap();
        let resumed = RunHandle::resume(fixture.coordinator.clone(), run.session_id, run.run_id.0)
            .await
            .unwrap();
        assert!(resumed.decision().await.unwrap().is_none());
        // A failed attempt does not poison the coordinator or forbid a fresh decision.
        resumed
            .readiness_lease(&fixture.todos, &fixture.agents, 10)
            .await
            .unwrap()
            .seal(FinalOutcome::Completed, None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn owned_todos_are_isolated_and_review_rejects_concurrent_edits() {
        let fixture = Fixture::new();
        let first = fixture.run().await;
        let second = fixture.run().await;
        let legacy = fixture
            .todos
            .create(todo("legacy unrelated"))
            .await
            .unwrap();
        let item = fixture
            .todos
            .create_registered(todo("owned"), Some(&first))
            .await
            .unwrap();
        assert_eq!(fixture.snapshot(&first).await.total, 1);
        assert!(fixture.snapshot(&second).await.ready());
        assert!(
            first
                .read_owned(&fixture.todos, &fixture.agents, Obligation::Todo(legacy.id))
                .await
                .is_err()
        );
        let stale = fixture.snapshot(&first).await;
        fixture
            .todos
            .set_blockers(item.id, vec!["waiting on fixture".into()])
            .await
            .unwrap();
        assert!(
            first
                .account(
                    &fixture.todos,
                    &fixture.agents,
                    Obligation::Todo(item.id),
                    review(stale, DispositionKind::BlockedWithImpact)
                )
                .await
                .is_err()
        );
        first
            .account(
                &fixture.todos,
                &fixture.agents,
                Obligation::Todo(item.id),
                review(
                    fixture.snapshot(&first).await,
                    DispositionKind::BlockedWithImpact,
                ),
            )
            .await
            .unwrap();
        let ready = fixture.snapshot(&first).await;
        assert!(ready.ready());
        assert_eq!(ready.incomplete, 1);
        assert_eq!(ready.completed, 0);
        assert_eq!(
            fixture.todos.snapshot().await.unwrap().items[&item.id].status,
            TodoStatus::Blocked
        );
        assert_eq!(
            fixture.todos.snapshot().await.unwrap().items[&legacy.id].status,
            TodoStatus::Pending
        );
    }
    #[tokio::test]
    async fn explicit_adoption_and_archival_preserve_obligations_and_evidence() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let item = fixture.todos.create(todo("older work")).await.unwrap();
        run.adopt_existing(
            &fixture.todos,
            &fixture.agents,
            Obligation::Todo(item.id),
            0,
        )
        .await
        .unwrap();
        fixture
            .todos
            .append_note(
                item.id,
                EntryKind::Evidence,
                "actual test evidence".into(),
                None,
            )
            .await
            .unwrap();
        fixture
            .todos
            .set_status(item.id, TodoStatus::Completed)
            .await
            .unwrap();
        run.account(
            &fixture.todos,
            &fixture.agents,
            Obligation::Todo(item.id),
            review(
                fixture.snapshot(&run).await,
                DispositionKind::CompletedWithEvidence,
            ),
        )
        .await
        .unwrap();
        assert!(fixture.snapshot(&run).await.ready());
        fixture.todos.archive(item.id).await.unwrap();
        assert_eq!(
            fixture.snapshot(&run).await.unresolved[0].reason,
            UnresolvedReason::RecordChanged
        );
        run.account(
            &fixture.todos,
            &fixture.agents,
            Obligation::Todo(item.id),
            review(
                fixture.snapshot(&run).await,
                DispositionKind::CompletedWithEvidence,
            ),
        )
        .await
        .unwrap();
        fixture.todos.remove(item.id).await.unwrap();
        assert_eq!(
            fixture.snapshot(&run).await.unresolved[0].reason,
            UnresolvedReason::MissingRecord
        );
    }
    #[tokio::test]
    async fn registration_failure_never_publishes_todo_and_missing_run_never_becomes_empty() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let reference = run.reference();
        let path = fixture.coordinator.directory.join("ledgers").join(format!(
            "{}-{}.json",
            reference.session_id, reference.run_id
        ));
        std::fs::remove_file(&path).unwrap();
        assert!(
            fixture
                .todos
                .create_registered(todo("must not publish"), Some(&run))
                .await
                .is_err()
        );
        assert!(fixture.todos.snapshot().await.unwrap().items.is_empty());
        assert!(
            RunHandle::resume(
                fixture.coordinator.clone(),
                reference.session_id,
                reference.run_id
            )
            .await
            .is_err()
        );
        assert!(
            run.snapshot(&fixture.todos, &fixture.agents, 64)
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn failed_publication_leaves_missing_obligation_and_reopen_retains_it() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let bad = TodoStore::new(
            fixture._root.path().join("not-a-directory/todos.json"),
            TodoScope::workspace(fixture._root.path().canonicalize().unwrap()),
        )
        .with_coordinator(fixture.coordinator.clone());
        std::fs::write(fixture._root.path().join("not-a-directory"), "occupied").unwrap();
        assert!(
            bad.create_registered(todo("tracked before failure"), Some(&run))
                .await
                .is_err()
        );
        let reference = run.reference();
        let reopened = RunHandle::resume(
            fixture.coordinator.clone(),
            reference.session_id,
            reference.run_id,
        )
        .await
        .unwrap();
        let snapshot = fixture.snapshot(&reopened).await;
        assert_eq!(snapshot.total, 1);
        assert_eq!(
            snapshot.unresolved[0].reason,
            UnresolvedReason::MissingRecord
        );
    }
    #[tokio::test]
    async fn coordinated_writer_waits_for_readiness_boundary_without_reentrant_deadlock() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let item = fixture
            .todos
            .create_registered(todo("original"), Some(&run))
            .await
            .unwrap();
        let guard = fixture.coordinator.lock().await.unwrap();
        let todos = fixture.todos.clone();
        let writer =
            tokio::spawn(
                async move { todos.edit(item.id, Some("edited".into()), None, None).await },
            );
        tokio::task::yield_now().await;
        assert!(!writer.is_finished());
        let ledger = run.ledger().await.unwrap();
        let (todos, agents) = run
            .records(&ledger, &fixture.todos, &fixture.agents)
            .await
            .unwrap();
        assert_eq!(todos.items[&item.id].title, "original");
        assert_eq!(ledger.snapshot(&todos, &agents, 0).unwrap().total, 1);
        drop(guard);
        tokio::time::timeout(std::time::Duration::from_secs(3), writer)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            fixture.todos.snapshot().await.unwrap().items[&item.id].title,
            "edited"
        );
    }
    struct OwnedExecutor;
    #[async_trait::async_trait]
    impl SubagentExecutor for OwnedExecutor {
        async fn execute(
            &self,
            mut context: ExecutionContext,
        ) -> std::result::Result<SubagentResult, String> {
            if context.task == "hold" {
                context.recv().await;
            }
            Ok(SubagentResult {
                summary: context
                    .completion
                    .map(|run| run.run_id().to_string())
                    .unwrap_or_else(|| "legacy".into()),
            })
        }
    }
    fn request(parent_id: Option<crate::subagent::AgentId>) -> SpawnRequest {
        let budget = AgentBudget {
            max_tokens: 100,
            max_terminals: 1,
        };
        SpawnRequest {
            parent_id,
            name: "owned-child".into(),
            task: "test ownership".into(),
            policy: AgentPolicy {
                readable_roots: vec![],
                writable_roots: vec![],
                allowed_tools: BTreeSet::new(),
                approval: ApprovalPolicy::Deny,
                budget: budget.clone(),
            },
            budget,
            worktree: None,
            branch: None,
        }
    }
    #[tokio::test]
    async fn subagent_creation_keeps_archived_obligations() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let runtime = SubagentRuntime::new(
            Arc::new(OwnedExecutor),
            RuntimeLimits::default(),
            Some(fixture.agents.clone()),
        )
        .unwrap();
        let parent = runtime
            .spawn_for_run(request(None), Some(run.clone()))
            .await
            .unwrap();
        let result = runtime.wait(parent).await.unwrap().unwrap();
        assert_eq!(result.summary, run.run_id().to_string());
        let snapshot = fixture.snapshot(&run).await;
        assert_eq!(snapshot.total, 1);
        assert!(!snapshot.ready());
        assert_eq!(
            run.read_owned(&fixture.todos, &fixture.agents, Obligation::Agent(parent))
                .await
                .unwrap()["completion"]["run_id"],
            run.run_id().to_string()
        );
        // Archived records cannot be resumed; a fresh owned child must reference
        // earlier results explicitly. Durable obligations survive auto-archive.
        assert!(runtime.is_archived(parent).await.unwrap());
        assert_eq!(
            snapshot.unresolved[0].reason,
            UnresolvedReason::NeedsDisposition
        );
        run.account(
            &fixture.todos,
            &fixture.agents,
            Obligation::Agent(parent),
            review(snapshot, DispositionKind::Incorporated),
        )
        .await
        .unwrap();
        assert!(fixture.snapshot(&run).await.ready());
    }
    #[tokio::test]
    async fn nested_children_and_terminal_followups_inherit_and_reject_unadopted_runs() {
        let fixture = Fixture::new();
        let run = fixture.run().await;
        let unrelated = fixture.run().await;
        let runtime = SubagentRuntime::new(
            Arc::new(OwnedExecutor),
            // This ownership fixture deliberately holds an ancestor active while
            // independently driving its child. Do not inherit the CPU-based
            // production default, which is one slot on two-core CI runners.
            RuntimeLimits {
                max_concurrency: 2,
                ..RuntimeLimits::default()
            },
            Some(fixture.agents.clone()),
        )
        .unwrap();
        let mut initial = request(None);
        // Keep an ancestor active so terminal children with worktrees remain
        // retained for the existing terminal-followup contract.
        initial.task = "hold".into();
        let ancestor = runtime
            .spawn_for_run(initial, Some(run.clone()))
            .await
            .unwrap();
        let mut child_request = request(Some(ancestor));
        child_request.worktree = Some(fixture._root.path().to_owned());
        let parent = runtime.spawn(child_request).await.unwrap();
        let child = parent;
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), runtime.wait(child))
                .await
                .expect("owned child did not finish")
                .unwrap()
                .unwrap()
                .summary,
            run.run_id().to_string()
        );
        let followup = runtime
            .follow_up_in_run(None, parent, "followup", Some(run.clone()))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), runtime.wait(followup))
                .await
                .expect("followup did not finish")
                .unwrap()
                .unwrap()
                .summary,
            run.run_id().to_string()
        );
        assert_eq!(fixture.snapshot(&run).await.total, 3);
        assert!(
            runtime
                .follow_up_in_run(None, parent, "unrelated", Some(unrelated.clone()))
                .await
                .is_err()
        );
        assert!(fixture.snapshot(&unrelated).await.ready());
        unrelated
            .adopt_existing(
                &fixture.todos,
                &fixture.agents,
                Obligation::Agent(parent),
                0,
            )
            .await
            .unwrap();
        let adopted = runtime
            .follow_up_in_run(None, parent, "explicitly adopted", Some(unrelated.clone()))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), runtime.wait(adopted))
                .await
                .expect("adopted followup did not finish")
                .unwrap()
                .unwrap()
                .summary,
            unrelated.run_id().to_string()
        );
        assert_eq!(fixture.snapshot(&unrelated).await.total, 3);
        runtime
            .send_message(ancestor, "finish fixture")
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), runtime.wait(ancestor))
            .await
            .expect("ancestor did not finish")
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn coordinator_excludes_a_fresh_process() {
        const ENV: &str = "HELM_COMPLETION_COORDINATOR_CHILD";
        if let Ok(root) = std::env::var(ENV) {
            let root = PathBuf::from(root);
            let coordinator = Coordinator::open(root.join("completion"), &root).unwrap();
            assert!(
                coordinator.lock().await.is_err(),
                "child acquired parent's workspace lease"
            );
            return;
        }
        let fixture = Fixture::new();
        let _guard = fixture.coordinator.lock().await.unwrap();
        let root = fixture._root.path().to_owned();
        tokio::task::spawn_blocking(move || {
            let mut child = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "completion::runtime::tests::coordinator_excludes_a_fresh_process",
                ])
                .env(ENV, root)
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("coordinator contention child timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn runtime_owner_excludes_second_process_and_recovers_after_owner_exit() {
        const ENV: &str = "HELM_COMPLETION_RUNTIME_OWNER_CHILD";
        if let Ok(root) = std::env::var(ENV) {
            let root = PathBuf::from(root);
            let coordinator = Coordinator::open(root.join("completion"), &root).unwrap();
            let store = AgentTreeStore::new(root.join("agents/tree.json"))
                .with_coordinator(coordinator.clone());
            let run = RunHandle::create(coordinator, Uuid::new_v4(), Uuid::new_v4())
                .await
                .unwrap();
            let runtime = SubagentRuntime::new_persistent(
                Arc::new(OwnedExecutor),
                RuntimeLimits::default(),
                store,
            )
            .await
            .unwrap();
            let mut task = request(None);
            task.task = "hold".into();
            let id = runtime
                .spawn_for_run(task, Some(run.clone()))
                .await
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while runtime.get(id).await.unwrap().status != crate::subagent::AgentStatus::Running {
                assert!(std::time::Instant::now() < deadline);
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            std::fs::write(
                root.join("owner-ready.tmp"),
                serde_json::to_vec(&(id, run.reference())).unwrap(),
            )
            .unwrap();
            std::fs::rename(root.join("owner-ready.tmp"), root.join("owner-ready.json")).unwrap();
            std::future::pending::<()>().await;
            return;
        }
        struct Child(std::process::Child);
        impl Drop for Child {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let fixture = Fixture::new();
        let mut child=Child(std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact","completion::runtime::tests::runtime_owner_excludes_second_process_and_recovers_after_owner_exit"])
            .env(ENV,fixture._root.path()).stdout(std::process::Stdio::null()).spawn().unwrap());
        let marker = fixture._root.path().join("owner-ready.json");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "owner child failed to become ready"
            );
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "owner child exited early"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let (id, reference): (crate::subagent::AgentId, RunReference) =
            serde_json::from_slice(&std::fs::read(marker).unwrap()).unwrap();
        let before = fixture.agents.get(id).await.unwrap().unwrap();
        assert_eq!(before.status, crate::subagent::AgentStatus::Running);
        let second = SubagentRuntime::new_persistent(
            Arc::new(OwnedExecutor),
            RuntimeLimits::default(),
            fixture.agents.clone(),
        )
        .await;
        assert!(matches!(second,Err(error) if error.to_string().contains("busy")));
        assert_eq!(
            fixture.agents.get(id).await.unwrap().unwrap(),
            before,
            "second startup rewrote live owner state"
        );
        assert!(
            fixture.agents.update(before).await.is_err(),
            "unleased direct store overwrote the owner"
        );
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        let recovered = SubagentRuntime::new_persistent(
            Arc::new(OwnedExecutor),
            RuntimeLimits::default(),
            fixture.agents.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            recovered.get(id).await.unwrap().status,
            crate::subagent::AgentStatus::Interrupted
        );
        let run = RunHandle::resume(
            fixture.coordinator.clone(),
            reference.session_id,
            reference.run_id,
        )
        .await
        .unwrap();
        let snapshot = fixture.snapshot(&run).await;
        assert_eq!(snapshot.total, 1);
        assert!(!snapshot.ready());
        assert_eq!(
            snapshot.unresolved[0].status,
            Some(super::super::WorkStatus::Agent(
                crate::subagent::AgentStatus::Interrupted
            ))
        );
    }
    #[tokio::test]
    async fn adoption_rejects_active_descendants_then_atomically_includes_archived_subtree() {
        let fixture = Fixture::new();
        let original = fixture.run().await;
        let adopter = fixture.run().await;
        let runtime = SubagentRuntime::new(
            Arc::new(OwnedExecutor),
            RuntimeLimits {
                max_concurrency: 3,
                event_history: 64,
            },
            Some(fixture.agents.clone()),
        )
        .unwrap();
        let mut initial = request(None);
        initial.task = "hold".into();
        let ancestor = runtime
            .spawn_for_run(initial, Some(original.clone()))
            .await
            .unwrap();
        let mut parent_task = request(Some(ancestor));
        parent_task.task = "hold".into();
        let parent = runtime.spawn(parent_task).await.unwrap();
        let mut child_task = request(Some(parent));
        child_task.task = "hold".into();
        let child = runtime.spawn(child_task).await.unwrap();
        runtime
            .send_message(parent, "finish while descendant remains active")
            .await
            .unwrap();
        runtime.wait(parent).await.unwrap().unwrap();
        let rejected = adopter
            .adopt_existing(
                &fixture.todos,
                &fixture.agents,
                Obligation::Agent(parent),
                0,
            )
            .await
            .unwrap_err();
        assert!(
            rejected
                .to_string()
                .contains("wait or cancel active work before adoption")
        );
        assert_eq!(
            fixture.snapshot(&adopter).await.total,
            0,
            "rejected adoption exposed partial membership"
        );
        runtime.cancel(child).await.unwrap();
        runtime.wait(child).await.unwrap().unwrap_err();
        assert!(runtime.is_archived(parent).await.unwrap());
        assert!(runtime.is_archived(child).await.unwrap());
        adopter
            .adopt_existing(
                &fixture.todos,
                &fixture.agents,
                Obligation::Agent(parent),
                0,
            )
            .await
            .unwrap();
        assert!(adopter.owns(Obligation::Agent(parent)).await.unwrap());
        assert!(adopter.owns(Obligation::Agent(child)).await.unwrap());
        assert!(!adopter.owns(Obligation::Agent(ancestor)).await.unwrap());
        let snapshot = fixture.snapshot(&adopter).await;
        assert_eq!(snapshot.total, 2);
        assert_eq!(snapshot.accounted, 0);
        let full = fixture.run().await;
        full.store
            .update(full.run_id, 0, |ledger| {
                for _ in 0..super::super::MAX_OBLIGATIONS - 1 {
                    ledger.adopt(
                        Obligation::Todo(crate::todo::TodoId::new()),
                        ledger.revision(),
                    )?;
                }
                Ok(())
            })
            .unwrap();
        let before = fixture.snapshot(&full).await;
        assert!(
            full.adopt_existing(
                &fixture.todos,
                &fixture.agents,
                Obligation::Agent(parent),
                before.revision
            )
            .await
            .is_err()
        );
        assert_eq!(
            fixture.snapshot(&full).await,
            before,
            "capacity failure partially adopted a subtree"
        );
        assert!(!full.owns(Obligation::Agent(parent)).await.unwrap());
        runtime
            .send_message(ancestor, "finish fixture")
            .await
            .unwrap();
        runtime.wait(ancestor).await.unwrap().unwrap();
    }
}
