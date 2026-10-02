//! Durable ordinary ownership and recovery. No model, native identity, provider,
//! terminal, subprocess or Git effects are executed by the owned worker.
use super::*;
use crate::{
    completion::{
        Obligation,
        runtime::{Coordinator, RunHandle},
    },
    todo::{TodoScope, TodoStore},
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use uuid::Uuid;

#[derive(Default)]
struct Worker {
    entered: AtomicUsize,
    retired: Arc<AtomicUsize>,
}
struct Retirement(Arc<AtomicUsize>);
impl Drop for Retirement {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[async_trait]
impl SubagentExecutor for Worker {
    async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let _retired = Retirement(self.retired.clone());
        context
            .progress(format!("owned-ready:{}", context.task))
            .await;
        if context.task == "complete" {
            return Ok(SubagentResult {
                summary: "owned completed result".into(),
            });
        }
        let cancel = context.cancellation.clone();
        tokio::select! {_=cancel.cancelled()=>Err("owned cancellation".into()),message=context.recv()=>match message {
            Some(InboxMessage::Message(text)) if text=="finish"=>Ok(SubagentResult {summary:"owned held result".into()}),
            _=>Err("unexpected owned fixture inbox".into()),
        }}
    }
}
fn request(root: &Path, task: &str, parent: Option<AgentId>) -> SpawnRequest {
    let budget = AgentBudget {
        max_tokens: 100,
        max_terminals: 0,
    };
    SpawnRequest {
        parent_id: parent,
        name: "owned fixture".into(),
        task: task.into(),
        policy: AgentPolicy {
            access: crate::config::AccessMode::ReadOnly,
            readable_roots: vec![root.into()],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: super::super::ApprovalPolicy::Deny,
            budget: budget.clone(),
        },
        budget,
        worktree: Some(root.into()),
        branch: None,
    }
}
struct Fixture {
    root: tempfile::TempDir,
    path: PathBuf,
    todos: TodoStore,
    a: RunHandle,
    b: RunHandle,
    worker: Arc<Worker>,
    runtime: SubagentRuntime,
}
impl Fixture {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agents.json");
        let coordinator = Coordinator::open(root.path().join("completion"), root.path()).unwrap();
        let session = Uuid::new_v4();
        let a = RunHandle::create(coordinator.clone(), session, Uuid::new_v4())
            .await
            .unwrap();
        let b = RunHandle::create(coordinator.clone(), session, Uuid::new_v4())
            .await
            .unwrap();
        let todos = TodoStore::new(
            root.path().join("todos.json"),
            TodoScope::session(root.path().into(), session),
        )
        .with_coordinator(coordinator.clone());
        let worker = Arc::new(Worker::default());
        let runtime = SubagentRuntime::new_persistent(
            worker.clone(),
            RuntimeLimits {
                max_concurrency: 4,
                event_history: 64,
            },
            AgentTreeStore::new(path.clone()).with_coordinator(coordinator.clone()),
        )
        .await
        .unwrap();
        Self {
            root,
            path,
            todos,
            a,
            b,
            worker,
            runtime,
        }
    }
    fn request(&self, task: &str, parent: Option<AgentId>) -> SpawnRequest {
        request(self.root.path(), task, parent)
    }
    fn store(&self) -> AgentTreeStore {
        self.runtime.store().unwrap()
    }
    async fn ready(&self, id: AgentId) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if self.runtime.get(id).await.unwrap().status == AgentStatus::Running
                    && self
                        .runtime
                        .get(id)
                        .await
                        .unwrap()
                        .recent_progress
                        .iter()
                        .any(|line| line.starts_with("owned-ready:"))
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
    }
    async fn finish(&self, id: AgentId) {
        self.runtime.send_message(id, "finish").await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), self.runtime.wait(id))
                .await
                .unwrap()
                .unwrap()
                .unwrap()
                .summary,
            "owned held result"
        );
    }
    async fn adopt(&self, run: &RunHandle, id: AgentId) {
        let revision = run
            .snapshot(&self.todos, &self.store(), 64)
            .await
            .unwrap()
            .revision;
        run.adopt_existing(&self.todos, &self.store(), Obligation::Agent(id), revision)
            .await
            .unwrap();
    }
    async fn refuse_active_adoption(&self, run: &RunHandle, id: AgentId) {
        let before = run.snapshot(&self.todos, &self.store(), 64).await.unwrap();
        let error = run
            .adopt_existing(
                &self.todos,
                &self.store(),
                Obligation::Agent(id),
                before.revision,
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("wait or cancel active work before adoption")
        );
        assert!(!run.owns(Obligation::Agent(id)).await.unwrap());
        assert_eq!(
            serde_json::to_value(run.snapshot(&self.todos, &self.store(), 64).await.unwrap())
                .unwrap(),
            serde_json::to_value(before).unwrap()
        );
    }
    async fn shutdown(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.runtime.shutdown())
            .await
            .unwrap();
        assert_eq!(
            self.worker.entered.load(Ordering::SeqCst),
            self.worker.retired.load(Ordering::SeqCst)
        );
    }
}

#[tokio::test]
async fn owned_spawn_records_exact_ledger_and_inherited_children_before_execution_and_scopes_shutdown()
 {
    let f = Fixture::new().await;
    let parent = f
        .runtime
        .spawn_for_run(f.request("held", None), Some(f.a.clone()))
        .await
        .unwrap();
    f.ready(parent).await;
    assert!(f.a.owns(Obligation::Agent(parent)).await.unwrap());
    assert!(!f.b.owns(Obligation::Agent(parent)).await.unwrap());
    assert_eq!(
        f.store().get(parent).await.unwrap().unwrap().completion,
        Some(f.a.reference())
    );
    let child = f
        .runtime
        .spawn(f.request("child", Some(parent)))
        .await
        .unwrap();
    f.ready(child).await;
    assert_eq!(
        f.runtime.get(child).await.unwrap().completion,
        Some(f.a.reference())
    );
    assert!(f.a.owns(Obligation::Agent(child)).await.unwrap());
    let foreign = f
        .runtime
        .spawn_for_run(f.request("foreign", None), Some(f.b.clone()))
        .await
        .unwrap();
    f.ready(foreign).await;
    let mut pending = f
        .runtime
        .pending_owned_shutdown(&f.a.reference())
        .await
        .unwrap();
    pending.sort();
    let mut expected = vec![parent, child];
    expected.sort();
    assert_eq!(pending, expected);
    assert_eq!(
        f.runtime
            .tree(Some(parent))
            .await
            .iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec![child]
    );
    f.finish(child).await;
    assert_eq!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap(),
        vec![parent]
    );
    f.finish(parent).await;
    assert!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.runtime
            .pending_owned_shutdown(&f.b.reference())
            .await
            .unwrap(),
        vec![foreign]
    );
    f.shutdown().await;
}

#[tokio::test]
async fn cross_run_child_requires_explicit_adoption_and_terminal_parent_then_uses_fresh_cancellation()
 {
    let f = Fixture::new().await;
    let parent = f
        .runtime
        .spawn_for_run(f.request("old", None), Some(f.a.clone()))
        .await
        .unwrap();
    f.ready(parent).await;
    assert!(
        matches!(f.runtime.spawn_for_run(f.request("new",Some(parent)),Some(f.b.clone())).await,Err(RuntimeError::Invalid(message)) if message.contains("explicitly adopt parent"))
    );
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 1);
    assert_eq!(f.runtime.list().await.len(), 1);
    f.refuse_active_adoption(&f.b, parent).await;
    f.runtime.cancel(parent).await.unwrap();
    assert!(f.runtime.wait(parent).await.unwrap().is_err());
    let original = f.runtime.get(parent).await.unwrap();
    f.adopt(&f.b, parent).await;
    let child = f
        .runtime
        .spawn_for_run(f.request("new", Some(parent)), Some(f.b.clone()))
        .await
        .unwrap();
    f.ready(child).await;
    assert_eq!(
        f.runtime.get(child).await.unwrap().completion,
        Some(f.b.reference())
    );
    assert!(f.b.owns(Obligation::Agent(child)).await.unwrap());
    assert!(
        !f.runtime
            .control(child)
            .await
            .unwrap()
            .cancel
            .is_cancelled()
    );
    f.finish(child).await;
    assert_eq!(f.runtime.get(parent).await.unwrap(), original);
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 2);
    f.shutdown().await;
}

#[tokio::test]
async fn legacy_parent_cannot_silently_become_owned_and_same_run_cancelled_parent_cannot_spawn() {
    let f = Fixture::new().await;
    let legacy = f.runtime.spawn(f.request("legacy", None)).await.unwrap();
    f.ready(legacy).await;
    assert!(
        matches!(f.runtime.spawn_for_run(f.request("owned",Some(legacy)),Some(f.a.clone())).await,Err(RuntimeError::Invalid(message)) if message.contains("explicitly adopt legacy parent"))
    );
    f.refuse_active_adoption(&f.a, legacy).await;
    f.finish(legacy).await;
    f.adopt(&f.a, legacy).await;
    let child = f
        .runtime
        .spawn_for_run(f.request("owned", Some(legacy)), Some(f.a.clone()))
        .await
        .unwrap();
    f.ready(child).await;
    assert_eq!(f.runtime.get(legacy).await.unwrap().completion, None);
    assert_eq!(
        f.runtime.get(child).await.unwrap().completion,
        Some(f.a.reference())
    );
    f.runtime.cancel(child).await.unwrap();
    assert!(f.runtime.wait(child).await.unwrap().is_err());
    assert!(
        matches!(f.runtime.spawn(f.request("late",Some(child))).await,Err(RuntimeError::Invalid(message)) if message.contains("cancelled parent"))
    );
    f.shutdown().await;
}

#[tokio::test]
async fn foreign_or_absent_persistent_coordinator_refuses_before_registering_new_work() {
    let f = Fixture::new().await;
    let foreign_root = tempfile::tempdir().unwrap();
    let foreign_coordinator =
        Coordinator::open(foreign_root.path().join("completion"), foreign_root.path()).unwrap();
    let foreign = RunHandle::create(foreign_coordinator, Uuid::new_v4(), Uuid::new_v4())
        .await
        .unwrap();
    let before = f.runtime.list().await;
    assert!(
        matches!(f.runtime.spawn_for_run(f.request("bad",None),Some(foreign.clone())).await,Err(RuntimeError::Invalid(message)) if message.contains("coordinated persistent store"))
    );
    assert_eq!(f.runtime.list().await, before);
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 0);
    let no_store = SubagentRuntime::new(f.worker.clone(), RuntimeLimits::default(), None).unwrap();
    assert!(
        matches!(no_store.spawn_for_run(f.request("bad",None),Some(f.a.clone())).await,Err(RuntimeError::Invalid(message)) if message.contains("coordinated persistent store"))
    );
    assert!(
        matches!(no_store.pending_owned_shutdown(&f.a.reference()).await,Err(RuntimeError::Invalid(message)) if message.contains("persistent agent state"))
    );
    no_store.shutdown().await;
    f.shutdown().await;
}

#[tokio::test]
async fn owned_followup_requires_original_run_authority_before_a_live_inbox_message() {
    let f = Fixture::new().await;
    let parent = f
        .runtime
        .spawn_for_run(f.request("old", None), Some(f.a.clone()))
        .await
        .unwrap();
    f.ready(parent).await;
    let before = f.runtime.events_after(0).await;
    assert!(
        matches!(f.runtime.follow_up_in_run(None,parent,"unowned followup",Some(f.b.clone())).await,Err(RuntimeError::Invalid(message)) if message.contains("explicitly adopt agent"))
    );
    assert_eq!(f.runtime.events_after(0).await, before);
    assert_eq!(
        f.runtime.control(parent).await.unwrap().inbox.capacity(),
        64
    );
    f.refuse_active_adoption(&f.b, parent).await;
    // Live input uses the already registered original run. Cross-run adoption
    // cannot manufacture authority over an active worker.
    assert_eq!(
        f.runtime
            .follow_up_in_run(None, parent, "finish", Some(f.a.clone()))
            .await
            .unwrap(),
        parent
    );
    // The scripted executor only accepts a Message as successful completion;
    // FollowUp was delivered to the original owner and does not invent success.
    assert!(f.runtime.wait(parent).await.unwrap().is_err());
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.runtime.get(parent).await.unwrap().completion,
        Some(f.a.reference())
    );
    f.shutdown().await;
}

#[tokio::test]
async fn durable_shutdown_keeps_missing_changed_or_live_final_records_pending_until_exact_persistence()
 {
    let f = Fixture::new().await;
    let id = f
        .runtime
        .spawn_for_run(f.request("held", None), Some(f.a.clone()))
        .await
        .unwrap();
    f.ready(id).await;
    f.finish(id).await;
    let store = f.store();
    let exact = store.get(id).await.unwrap().unwrap();
    assert!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap()
            .is_empty()
    );
    let mut tree = store.load().await.unwrap();
    tree.agents.remove(&id);
    store.save(&tree).await.unwrap();
    assert_eq!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap(),
        vec![id]
    );
    tree.agents.insert(
        id,
        AgentRecord {
            status: AgentStatus::Running,
            ..exact.clone()
        },
    );
    store.save(&tree).await.unwrap();
    assert_eq!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap(),
        vec![id]
    );
    tree.agents.insert(
        id,
        AgentRecord {
            completion: Some(f.b.reference()),
            ..exact.clone()
        },
    );
    store.save(&tree).await.unwrap();
    assert_eq!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap(),
        vec![id]
    );
    tree.agents.insert(id, exact);
    store.save(&tree).await.unwrap();
    assert!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap()
            .is_empty()
    );
    f.shutdown().await;
}

#[tokio::test]
async fn archived_final_records_remain_readable_and_corruption_blocks_owned_completion() {
    let f = Fixture::new().await;
    let mut task = f.request("complete", None);
    task.worktree = None;
    let id = f
        .runtime
        .spawn_for_run(task, Some(f.a.clone()))
        .await
        .unwrap();
    let record = f.runtime.wait(id).await.unwrap().unwrap();
    assert!(f.runtime.is_archived(id).await.unwrap());
    assert_eq!(record.summary, "owned completed result");
    assert!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap()
            .is_empty()
    );
    let archive = f.path.with_extension("archive").join(format!("{id}.json"));
    let bytes = std::fs::read(&archive).unwrap();
    std::fs::write(&archive, b"{owned corrupt archive").unwrap();
    assert!(matches!(
        f.runtime.get(id).await,
        Err(RuntimeError::Persistence(_))
    ));
    assert!(matches!(
        f.runtime.wait(id).await,
        Err(RuntimeError::Persistence(_))
    ));
    assert!(
        matches!(f.runtime.pending_owned_shutdown(&f.a.reference()).await,Ok(pending) if pending.is_empty())
    );
    // No live control or tree record remains: pending_owned_shutdown cannot
    // certify an archive by ID it does not own. The completion read refuses it.
    assert!(
        f.a.read_owned(&f.todos, &f.store(), Obligation::Agent(id))
            .await
            .is_err()
    );
    std::fs::write(archive, bytes).unwrap();
    assert_eq!(f.runtime.wait(id).await.unwrap().unwrap(), record);
    f.shutdown().await;
}

fn retained_record(
    root: &Path,
    id: AgentId,
    completion: Option<crate::completion::runtime::RunReference>,
) -> AgentRecord {
    let request = request(root, "retained ordinary work", None);
    let now = Utc::now();
    AgentRecord {
        completion,
        id,
        parent_id: None,
        name: request.name,
        task: request.task,
        status: AgentStatus::Running,
        policy: request.policy,
        budget: request.budget,
        worktree: request.worktree,
        branch: None,
        created_at: now,
        started_at: Some(now),
        finished_at: None,
        updated_at: now,
        recent_progress: vec!["before interruption".into()],
        result: None,
        error: None,
    }
}
#[tokio::test]
async fn startup_refuses_missing_completion_coordinator_ledger_or_obligation_without_archiving_evidence()
 {
    for failure in ["coordinator", "ledger", "obligation"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("agents.json");
        let coordinator = Coordinator::open(root.path().join("completion"), root.path()).unwrap();
        let session = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let reference = crate::completion::runtime::RunReference {
            session_id: session,
            run_id,
        };
        if failure == "obligation" {
            RunHandle::create(coordinator.clone(), session, run_id)
                .await
                .unwrap();
        }
        let mut seed = AgentTreeStore::new(path.clone());
        if failure != "coordinator" {
            seed = seed.with_coordinator(coordinator.clone());
            seed.acquire_runtime_owner().unwrap();
        }
        let id = AgentId::new();
        seed.create(retained_record(root.path(), id, Some(reference)))
            .await
            .unwrap();
        drop(seed);
        let bytes = std::fs::read(&path).unwrap();
        let worker = Arc::new(Worker::default());
        let mut store = AgentTreeStore::new(path.clone());
        if failure != "coordinator" {
            store = store.with_coordinator(coordinator);
        }
        let error =
            SubagentRuntime::new_persistent(worker.clone(), RuntimeLimits::default(), store)
                .await
                .err()
                .unwrap();
        assert!(matches!(error, RuntimeError::Persistence(_)));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(!path.with_extension("archive").exists());
        assert_eq!(worker.entered.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn recovered_owned_and_legacy_records_stay_interrupted_and_never_reexecute_or_complete_their_ledgers()
 {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agents.json");
    let coordinator = Coordinator::open(root.path().join("completion"), root.path()).unwrap();
    let session = Uuid::new_v4();
    let run = RunHandle::create(coordinator.clone(), session, Uuid::new_v4())
        .await
        .unwrap();
    let todos = TodoStore::new(
        root.path().join("todos.json"),
        TodoScope::session(root.path().into(), session),
    )
    .with_coordinator(coordinator.clone());
    let owned = AgentId::new();
    let legacy = AgentId::new();
    run.register(Obligation::Agent(owned)).await.unwrap();
    let mut seed = AgentTreeStore::new(path.clone()).with_coordinator(coordinator.clone());
    seed.acquire_runtime_owner().unwrap();
    seed.create(retained_record(root.path(), owned, Some(run.reference())))
        .await
        .unwrap();
    seed.create(retained_record(root.path(), legacy, None))
        .await
        .unwrap();
    drop(seed);
    let worker = Arc::new(Worker::default());
    let runtime = SubagentRuntime::new_persistent(
        worker.clone(),
        RuntimeLimits::default(),
        AgentTreeStore::new(path).with_coordinator(coordinator),
    )
    .await
    .unwrap();
    for id in [owned, legacy] {
        let record = runtime.get(id).await.unwrap();
        assert_eq!(record.status, AgentStatus::Interrupted);
        assert_eq!(record.recent_progress, vec!["before interruption"]);
        assert!(record.result.is_none());
        assert!(runtime.wait(id).await.unwrap().is_err());
        assert!(runtime.send_message(id, "replay").await.is_err());
    }
    assert_eq!(worker.entered.load(Ordering::SeqCst), 0);
    assert!(run.owns(Obligation::Agent(owned)).await.unwrap());
    let readiness = run
        .snapshot(&todos, &runtime.store().unwrap(), 64)
        .await
        .unwrap();
    assert_eq!(readiness.total, 1);
    assert_eq!(readiness.completed, 0);
    assert_eq!(readiness.incomplete, 1);
    assert!(
        runtime
            .pending_owned_shutdown(&run.reference())
            .await
            .unwrap()
            .is_empty()
    );
    runtime.shutdown().await;
    assert_eq!(worker.retired.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn admission_storage_failure_leaves_a_missing_durable_obligation_without_starting_or_retrying_work()
 {
    let f = Fixture::new().await;
    assert!(f.runtime.list().await.is_empty());
    // With no active agent there is no concurrent tree writer. Preserve exactly
    // the corrupt fixture evidence until its restoration below.
    std::fs::write(&f.path, b"{owned damaged agent tree").unwrap();
    assert!(matches!(
        f.runtime
            .spawn_for_run(f.request("never admitted", None), Some(f.a.clone()))
            .await,
        Err(RuntimeError::Persistence(_))
    ));
    assert_eq!(
        std::fs::read(&f.path).unwrap(),
        b"{owned damaged agent tree"
    );
    assert!(f.runtime.list().await.is_empty());
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 0);
    std::fs::write(
        &f.path,
        serde_json::to_vec(&super::super::AgentTree::default()).unwrap(),
    )
    .unwrap();
    let readiness = f.a.snapshot(&f.todos, &f.store(), 64).await.unwrap();
    assert_eq!(readiness.total, 1);
    assert_eq!(readiness.accounted, 0);
    assert_eq!(readiness.completed, 0);
    assert_eq!(readiness.unresolved.len(), 1);
    assert!(
        f.a.readiness_lease(&f.todos, &f.store(), 64)
            .await
            .unwrap()
            .seal(crate::completion::FinalOutcome::Completed, None)
            .await
            .is_err()
    );
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 0);
    f.shutdown().await;
}

#[tokio::test]
async fn sealed_parent_ledger_refuses_new_agent_registration_before_tree_or_executor_publication() {
    let f = Fixture::new().await;
    f.a.readiness_lease(&f.todos, &f.store(), 64)
        .await
        .unwrap()
        .seal(crate::completion::FinalOutcome::Completed, None)
        .await
        .unwrap();
    let events = f.runtime.events_after(0).await;
    assert!(matches!(
        f.runtime
            .spawn_for_run(f.request("after final decision", None), Some(f.a.clone()))
            .await,
        Err(RuntimeError::Persistence(_))
    ));
    assert!(f.runtime.list().await.is_empty());
    assert_eq!(f.runtime.events_after(0).await, events);
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 0);
    assert!(f.a.decision().await.unwrap().is_some());
    f.shutdown().await;
}

#[tokio::test]
async fn observed_executor_success_does_not_override_failed_terminal_persistence_or_replay_the_worker()
 {
    let f = Fixture::new().await;
    let id = f
        .runtime
        .spawn_for_run(f.request("held", None), Some(f.a.clone()))
        .await
        .unwrap();
    f.ready(id).await;
    let durable = f.store().load().await.unwrap();
    assert_eq!(durable.agents[&id].status, AgentStatus::Running);
    std::fs::write(&f.path, b"{owned terminal persistence failure").unwrap();
    f.finish(id).await;
    let terminal = f.runtime.get_retained(id).await.unwrap();
    assert_eq!(terminal.status, AgentStatus::Completed);
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 1);
    assert!(matches!(
        f.runtime.pending_owned_shutdown(&f.a.reference()).await,
        Err(RuntimeError::Persistence(_))
    ));
    std::fs::write(&f.path, serde_json::to_vec(&durable).unwrap()).unwrap();
    assert_eq!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap(),
        vec![id]
    );
    // Reconciliation stores the same already observed terminal record. It does
    // not run, infer, re-admit, resend an inbox command or repeat an effect.
    f.store().update(terminal.clone()).await.unwrap();
    assert!(
        f.runtime
            .pending_owned_shutdown(&f.a.reference())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.runtime.wait(id).await.unwrap().unwrap().summary,
        terminal.result.unwrap()
    );
    assert_eq!(f.worker.entered.load(Ordering::SeqCst), 1);
    f.shutdown().await;
}
