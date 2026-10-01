//! The UI adapter observes an owned local runtime, never a provider or UI executor.
use super::*;
use crate::subagent::{
    self, ExecutionContext, InboxMessage, SpawnRequest, SubagentExecutor, SubagentResult,
};
use std::{
    collections::BTreeSet,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct Worker {
    entered: AtomicUsize,
    retired: Arc<AtomicUsize>,
    messages: Mutex<Vec<(Uuid, InboxMessage)>>,
}
struct Retired(Arc<AtomicUsize>);
impl Drop for Retired {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
#[async_trait]
impl SubagentExecutor for Worker {
    async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
        self.entered.fetch_add(1, Ordering::SeqCst);
        let _retired = Retired(self.retired.clone());
        // This executor owns no OS process, provider request or worktree. The
        // drop counter observes this future's retirement, not external cleanup.
        assert_eq!(context.policy.access, crate::config::AccessMode::ReadOnly);
        assert!(context.policy.allowed_tools.is_empty());
        context.progress(format!("ready:{}", context.task)).await;
        if context.task == "fail" {
            return Err("owned executor failure".into());
        }
        if context.task.starts_with("finish:") {
            return Ok(SubagentResult {
                summary: context.task,
            });
        }
        if context.task == "many" {
            for n in 0..30 {
                context.progress(format!("bounded progress {n}")).await;
            }
            return Ok(SubagentResult {
                summary: "many finished".into(),
            });
        }
        let cancel = context.cancellation.clone();
        loop {
            tokio::select! {
                _=cancel.cancelled()=>return Err("owned cancellation observed".into()),
                message=context.recv()=>{
                    let Some(message)=message else{return Err("owned inbox closed".into());};
                    self.messages.lock().unwrap().push((context.id.0,message.clone()));
                    match message {
                        InboxMessage::Message(text) if text=="finish"=>return Ok(SubagentResult {summary:format!("finished:{}",context.task)}),
                        InboxMessage::Message(text)=>context.progress(format!("message:{text}")).await,
                        InboxMessage::FollowUp(text)=>context.progress(format!("followup:{text}")).await,
                    }
                }
            }
        }
    }
}
fn request(name: &str, task: &str, parent: Option<subagent::AgentId>) -> SpawnRequest {
    let budget = subagent::AgentBudget {
        max_tokens: 50,
        max_terminals: 0,
    };
    SpawnRequest {
        parent_id: parent,
        name: name.into(),
        task: task.into(),
        policy: subagent::AgentPolicy {
            access: crate::config::AccessMode::ReadOnly,
            readable_roots: vec![],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: subagent::ApprovalPolicy::Deny,
            budget: budget.clone(),
        },
        budget,
        worktree: None,
        branch: None,
    }
}
fn runtime(
    worker: Arc<Worker>,
    concurrency: usize,
    history: usize,
) -> Arc<subagent::SubagentRuntime> {
    Arc::new(
        subagent::SubagentRuntime::new(
            worker,
            subagent::RuntimeLimits {
                max_concurrency: concurrency,
                event_history: history,
            },
            None,
        )
        .unwrap(),
    )
}
async fn progress(runtime: &subagent::SubagentRuntime, id: subagent::AgentId, text: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if runtime
                .get(id)
                .await
                .unwrap()
                .recent_progress
                .iter()
                .any(|line| line == text)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
async fn outcome(
    runtime: &subagent::SubagentRuntime,
    id: subagent::AgentId,
) -> Result<SubagentResult, String> {
    tokio::time::timeout(Duration::from_secs(3), runtime.wait(id))
        .await
        .unwrap()
        .unwrap()
}
async fn retire(runtime: &subagent::SubagentRuntime, worker: &Worker) {
    tokio::time::timeout(Duration::from_secs(3), runtime.shutdown())
        .await
        .unwrap();
    assert_eq!(
        worker.entered.load(Ordering::SeqCst),
        worker.retired.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn live_supervision_preserves_parentage_exact_inbox_and_independent_filtered_replay() {
    let worker = Arc::new(Worker::default());
    let runtime = runtime(worker.clone(), 2, 128);
    let supervisor = RuntimeAgentSupervisor::new(runtime.clone());
    let mut live = supervisor.subscribe();
    let parent = runtime
        .spawn(request("parent", "parent", None))
        .await
        .unwrap();
    let child = runtime
        .spawn(request("child", "child", Some(parent)))
        .await
        .unwrap();
    progress(&runtime, parent, "ready:parent").await;
    progress(&runtime, child, "ready:child").await;
    let tree = supervisor.tree().await.unwrap();
    assert_eq!(tree.len(), 2);
    let view = tree
        .iter()
        .find(|view| view.id == AgentId(child.0))
        .unwrap();
    assert_eq!(view.parent, Some(AgentId(parent.0)));
    assert_eq!(view.status, AgentStatus::Running);
    assert!(view.started_at.is_some());
    assert!(view.result.is_none());
    assert!(!view.status.is_terminal());
    supervisor
        .send_message(AgentId(child.0), "selected human message".into())
        .await
        .unwrap();
    assert_eq!(
        supervisor
            .follow_up(AgentId(child.0), "same live child".into())
            .await
            .unwrap(),
        AgentId(child.0)
    );
    progress(&runtime, child, "followup:same live child").await;
    assert_eq!(
        *worker.messages.lock().unwrap(),
        vec![
            (
                child.0,
                InboxMessage::Message("selected human message".into())
            ),
            (child.0, InboxMessage::FollowUp("same live child".into()))
        ]
    );
    supervisor
        .send_message(AgentId(child.0), "finish".into())
        .await
        .unwrap();
    assert_eq!(
        outcome(&runtime, child).await.unwrap().summary,
        "finished:child"
    );
    let replay = supervisor.replay(AgentId(child.0), None).await.unwrap();
    assert_eq!(replay.agent.status, AgentStatus::Completed);
    assert_eq!(replay.agent.result.as_deref(), Some("finished:child"));
    assert!(replay.agent.error.is_none());
    assert!(
        replay
            .history
            .as_ref()
            .is_some_and(|status| !status.durable)
    );
    assert!(
        replay
            .events
            .iter()
            .all(|event| event.agent_id == AgentId(child.0))
    );
    assert!(
        replay
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::FollowUpQueued { child: None }))
    );
    let tail = replay.events.last().unwrap().sequence;
    assert!(
        supervisor
            .inspect(AgentId(child.0), Some(tail))
            .await
            .unwrap()
            .is_empty()
    );
    supervisor
        .send_message(AgentId(parent.0), "finish".into())
        .await
        .unwrap();
    outcome(&runtime, parent).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = live.recv().await.unwrap();
            if event.agent_id == AgentId(child.0)
                && matches!(event.kind, AgentEventKind::Completed { .. })
            {
                assert_eq!(event.sequence, tail);
                break;
            }
        }
    })
    .await
    .unwrap();
    retire(&runtime, &worker).await;
}

#[tokio::test]
async fn supervisor_cancel_observes_live_future_retirement_and_never_starts_cancelled_queued_work()
{
    let worker = Arc::new(Worker::default());
    let runtime = runtime(worker.clone(), 1, 64);
    let supervisor = RuntimeAgentSupervisor::new(runtime.clone());
    let active = runtime
        .spawn(request("active", "held", None))
        .await
        .unwrap();
    progress(&runtime, active, "ready:held").await;
    let queued = runtime
        .spawn(request("queued", "queued", None))
        .await
        .unwrap();
    assert_eq!(
        runtime.get(queued).await.unwrap().status,
        subagent::AgentStatus::Queued
    );
    supervisor.cancel(AgentId(queued.0)).await.unwrap();
    assert!(outcome(&runtime, queued).await.is_err());
    assert_eq!(worker.entered.load(Ordering::SeqCst), 1);
    supervisor.cancel(AgentId(active.0)).await.unwrap();
    assert!(outcome(&runtime, active).await.is_err());
    for id in [active, queued] {
        let replay = supervisor.replay(AgentId(id.0), None).await.unwrap();
        assert_eq!(replay.agent.status, AgentStatus::Cancelled);
        assert!(replay.agent.status.is_terminal());
        assert!(replay.agent.result.is_none());
        assert!(replay.agent.error.is_some());
        assert!(
            replay
                .events
                .iter()
                .any(|event| matches!(event.kind, AgentEventKind::Cancelled))
        );
    }
    retire(&runtime, &worker).await;
    assert_eq!(worker.retired.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn terminal_followup_creates_a_new_child_without_rewriting_the_final_parent() {
    let worker = Arc::new(Worker::default());
    let runtime = runtime(worker.clone(), 1, 64);
    let supervisor = RuntimeAgentSupervisor::new(runtime.clone());
    let retained = tempfile::tempdir().unwrap();
    let mut parent_request = request("completed", "finish:original", None);
    // A terminal leaf without retained worktree metadata is archived by the
    // runtime. This owned empty path deliberately keeps the parent retained;
    // the scripted executor performs no Git/worktree operations.
    parent_request.worktree = Some(retained.path().to_path_buf());
    let parent = runtime.spawn(parent_request).await.unwrap();
    outcome(&runtime, parent).await.unwrap();
    let before = runtime.get(parent).await.unwrap();
    let child = supervisor
        .follow_up(AgentId(parent.0), "finish:explicit new task".into())
        .await
        .unwrap();
    assert_ne!(child, AgentId(parent.0));
    assert_eq!(
        outcome(&runtime, subagent::AgentId(child.0))
            .await
            .unwrap()
            .summary,
        "finish:explicit new task"
    );
    assert_eq!(runtime.get(parent).await.unwrap(), before);
    let replay = supervisor.replay(child, None).await.unwrap();
    assert_eq!(replay.agent.parent, Some(AgentId(parent.0)));
    assert_eq!(replay.agent.task, "finish:explicit new task");
    assert!(
        matches!(supervisor.send_message(AgentId(parent.0),"cannot mutate final work".into()).await,Err(SupervisionError::Failed(message)) if message.contains("terminal"))
    );
    runtime
        .clear_worktree(subagent::AgentId(child.0))
        .await
        .unwrap();
    runtime.clear_worktree(parent).await.unwrap();
    assert!(runtime.is_archived(parent).await.unwrap());
    retire(&runtime, &worker).await;
}

#[tokio::test]
async fn unknown_agents_and_executor_failure_have_distinct_bounded_supervision_errors() {
    let worker = Arc::new(Worker::default());
    let runtime = runtime(worker.clone(), 1, 32);
    let supervisor = RuntimeAgentSupervisor::new(runtime.clone());
    let unknown = AgentId(Uuid::new_v4());
    assert!(
        matches!(supervisor.inspect(unknown,None).await,Err(SupervisionError::NotFound(id)) if id==unknown)
    );
    assert!(
        matches!(supervisor.replay(unknown,None).await,Err(SupervisionError::NotFound(id)) if id==unknown)
    );
    assert!(
        matches!(supervisor.send_message(unknown,"message".into()).await,Err(SupervisionError::NotFound(id)) if id==unknown)
    );
    assert!(
        matches!(supervisor.follow_up(unknown,"followup".into()).await,Err(SupervisionError::NotFound(id)) if id==unknown)
    );
    assert!(
        matches!(supervisor.cancel(unknown).await,Err(SupervisionError::NotFound(id)) if id==unknown)
    );
    let retained = tempfile::tempdir().unwrap();
    let mut failed_request = request("failed", "fail", None);
    failed_request.worktree = Some(retained.path().to_path_buf());
    let failed = runtime.spawn(failed_request).await.unwrap();
    assert_eq!(
        outcome(&runtime, failed).await.unwrap_err(),
        "owned executor failure"
    );
    let replay = supervisor.replay(AgentId(failed.0), None).await.unwrap();
    assert_eq!(replay.agent.status, AgentStatus::Failed);
    assert_eq!(
        replay.agent.error.as_deref(),
        Some("owned executor failure")
    );
    assert!(replay.agent.result.is_none());
    assert!(replay.events.iter().any(|event|matches!(&event.kind,AgentEventKind::Failed {error} if error=="owned executor failure")));
    retire(&runtime, &worker).await;
    assert!(
        matches!(supervisor.follow_up(AgentId(failed.0),"new work after shutdown".into()).await,Err(SupervisionError::Failed(error)) if error.contains("shutting down"))
    );
    runtime.clear_worktree(failed).await.unwrap();
    assert!(runtime.is_archived(failed).await.unwrap());
}

#[tokio::test]
async fn bounded_replay_reports_cursor_loss_without_losing_the_final_record_or_inventing_events() {
    let worker = Arc::new(Worker::default());
    let runtime = runtime(worker.clone(), 1, 3);
    let supervisor = RuntimeAgentSupervisor::new(runtime.clone());
    let id = runtime.spawn(request("many", "many", None)).await.unwrap();
    outcome(&runtime, id).await.unwrap();
    let replay = supervisor
        .replay(
            AgentId(id.0),
            Some(subagent::HistoryCursor {
                epoch: Uuid::new_v4(),
                sequence: 0,
            }),
        )
        .await
        .unwrap();
    let status = replay.history.as_ref().unwrap();
    assert!(status.cursor_gap);
    assert!(status.evicted_through > 0);
    assert_eq!(replay.events.len(), 3);
    assert_eq!(status.first_sequence, Some(replay.events[0].sequence));
    assert_eq!(replay.agent.result.as_deref(), Some("many finished"));
    assert_eq!(replay.agent.recent_progress.len(), 20);
    assert_eq!(
        replay.agent.recent_progress.last().unwrap(),
        "bounded progress 29"
    );
    assert!(
        replay
            .events
            .iter()
            .all(|event| event.agent_id == AgentId(id.0))
    );
    let current = supervisor
        .replay(AgentId(id.0), Some(status.cursor))
        .await
        .unwrap();
    assert!(current.events.is_empty());
    assert!(!current.history.unwrap().cursor_gap);
    assert_eq!(current.agent.result, replay.agent.result);
    let before_first = subagent::HistoryCursor {
        epoch: status.cursor.epoch,
        sequence: 0,
    };
    assert!(
        supervisor
            .replay(AgentId(id.0), Some(before_first))
            .await
            .unwrap()
            .history
            .unwrap()
            .cursor_gap
    );
    retire(&runtime, &worker).await;
}

fn source_event(
    sequence: u64,
    id: subagent::AgentId,
    kind: subagent::SubagentEventKind,
) -> subagent::SubagentEvent {
    subagent::SubagentEvent {
        sequence,
        timestamp: Utc::now(),
        agent_id: id,
        kind,
    }
}
#[tokio::test]
async fn transport_lag_is_cumulative_and_has_no_agent_identity_or_durable_lifecycle_claim() {
    let lag = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let id = subagent::AgentId::new();
    for (first, last, total) in [(1, 5, 4), (6, 8, 6)] {
        let (source, receiver) = broadcast::channel(1);
        let (sink, mut events) = broadcast::channel(16);
        for n in first..=last {
            source
                .send(source_event(
                    n,
                    id,
                    subagent::SubagentEventKind::Progress {
                        text: format!("progress {n}"),
                    },
                ))
                .unwrap();
        }
        drop(source);
        let task = tokio::spawn(forward_events(receiver, sink, lag.clone()));
        let gap = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(gap.agent_id, AgentId(Uuid::nil()));
        assert_eq!(gap.sequence, last - 1);
        assert!(matches!(gap.kind,AgentEventKind::HistoryGap {skipped} if skipped==total));
        let event = events.recv().await.unwrap();
        assert_eq!(event.agent_id, AgentId(id.0));
        assert_eq!(event.sequence, last);
        assert!(
            matches!(event.kind,AgentEventKind::Progress {text} if text==format!("progress {last}"))
        );
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lag.load(std::sync::atomic::Ordering::Relaxed), total);
        assert!(matches!(
            events.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ));
    }
}

#[test]
fn retained_projection_preserves_results_failures_and_missing_or_invalid_timing() {
    let start = Utc::now();
    let id = subagent::AgentId::new();
    let parent = subagent::AgentId::new();
    let spawn = request("retained", "explicit retained task", Some(parent));
    let record = subagent::AgentRecord {
        completion: None,
        id,
        parent_id: Some(parent),
        name: spawn.name,
        task: spawn.task,
        status: subagent::AgentStatus::Interrupted,
        policy: spawn.policy,
        budget: spawn.budget,
        worktree: Some(PathBuf::from("owned-retained-worktree")),
        branch: Some("owned-retained".into()),
        created_at: start,
        started_at: Some(start),
        finished_at: Some(start + chrono::Duration::seconds(7)),
        updated_at: start,
        recent_progress: vec!["last observed progress".into()],
        result: None,
        error: Some("interrupted; cleanup unknown".into()),
    };
    let projected = convert_record(record.clone());
    assert_eq!(projected.id, AgentId(id.0));
    assert_eq!(projected.parent, Some(AgentId(parent.0)));
    assert_eq!(projected.elapsed, Duration::from_secs(7));
    assert_eq!(projected.started_at, Some(start));
    assert_eq!(projected.worktree, record.worktree);
    assert_eq!(projected.recent_progress, record.recent_progress);
    assert_eq!(projected.error, record.error);
    assert_eq!(projected.task, "explicit retained task");
    assert_eq!(projected.status, AgentStatus::Interrupted);
    assert!(projected.status.is_terminal());
    assert!(projected.result.is_none());
    assert_eq!(projected.id.to_string(), id.0.to_string());
    let mut no_start = record.clone();
    no_start.started_at = None;
    assert_eq!(convert_record(no_start).elapsed, Duration::ZERO);
    let mut reverse = record.clone();
    reverse.finished_at = Some(start - chrono::Duration::seconds(1));
    assert_eq!(convert_record(reverse).elapsed, Duration::ZERO);
    for (source, expected, terminal) in [
        (subagent::AgentStatus::Queued, AgentStatus::Queued, false),
        (subagent::AgentStatus::Running, AgentStatus::Running, false),
        (subagent::AgentStatus::Waiting, AgentStatus::Waiting, false),
        (
            subagent::AgentStatus::Completed,
            AgentStatus::Completed,
            true,
        ),
        (subagent::AgentStatus::Failed, AgentStatus::Failed, true),
        (subagent::AgentStatus::TimedOut, AgentStatus::TimedOut, true),
        (
            subagent::AgentStatus::Cancelled,
            AgentStatus::Cancelled,
            true,
        ),
    ] {
        let mut retained = record.clone();
        retained.status = source;
        let view = convert_record(retained);
        assert_eq!(view.status, expected);
        assert_eq!(view.status.is_terminal(), terminal);
        assert_eq!(view.error.as_deref(), Some("interrupted; cleanup unknown"));
    }
    let timed = convert_event(source_event(17, id, subagent::SubagentEventKind::TimedOut));
    assert_eq!(timed.sequence, 17);
    assert_eq!(timed.agent_id, AgentId(id.0));
    assert!(
        matches!(timed.kind,AgentEventKind::TimedOut {error} if error=="runtime budget exhausted")
    );
}

struct InspectOnly {
    view: AgentView,
    events: Vec<AgentEvent>,
    sender: broadcast::Sender<AgentEvent>,
}
#[async_trait]
impl AgentSupervisor for InspectOnly {
    async fn tree(&self) -> Result<Vec<AgentView>, SupervisionError> {
        Ok(vec![self.view.clone()])
    }
    async fn inspect(
        &self,
        id: AgentId,
        after: Option<u64>,
    ) -> Result<Vec<AgentEvent>, SupervisionError> {
        assert_eq!(id, self.view.id);
        Ok(self
            .events
            .iter()
            .filter(|event| event.sequence > after.unwrap_or(0))
            .cloned()
            .collect())
    }
    async fn send_message(&self, _: AgentId, _: String) -> Result<(), SupervisionError> {
        Err(SupervisionError::Unavailable)
    }
    async fn follow_up(&self, _: AgentId, _: String) -> Result<AgentId, SupervisionError> {
        Err(SupervisionError::Unavailable)
    }
    async fn cancel(&self, _: AgentId) -> Result<(), SupervisionError> {
        Err(SupervisionError::Unavailable)
    }
    fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.sender.subscribe()
    }
}
#[tokio::test]
async fn inspect_only_adapter_marks_missing_history_contract_and_respects_sequence_filter() {
    let id = AgentId(Uuid::new_v4());
    let (sender, _) = broadcast::channel(1);
    let now = Utc::now();
    let adapter = InspectOnly {
        view: AgentView {
            id,
            parent: None,
            task: "third-party retained task".into(),
            status: AgentStatus::Completed,
            started_at: None,
            elapsed: Duration::ZERO,
            worktree: None,
            recent_progress: vec![],
            result: Some("retained result".into()),
            error: None,
        },
        events: vec![
            AgentEvent {
                sequence: 1,
                timestamp: now,
                agent_id: id,
                kind: AgentEventKind::Started,
            },
            AgentEvent {
                sequence: 2,
                timestamp: now,
                agent_id: id,
                kind: AgentEventKind::Completed {
                    result: "retained result".into(),
                },
            },
        ],
        sender,
    };
    let inspection = adapter
        .replay(
            id,
            Some(subagent::HistoryCursor {
                epoch: Uuid::new_v4(),
                sequence: 1,
            }),
        )
        .await
        .unwrap();
    assert!(inspection.history.is_none());
    assert_eq!(inspection.transport_skipped, 0);
    assert_eq!(inspection.events.len(), 1);
    assert_eq!(inspection.events[0].sequence, 2);
    assert_eq!(inspection.agent.result.as_deref(), Some("retained result"));
    let unknown = AgentId(Uuid::new_v4());
    assert!(
        matches!(adapter.replay(unknown,None).await,Err(SupervisionError::NotFound(found)) if found==unknown)
    );
}
#[tokio::test]
async fn unavailable_supervisor_has_no_fictitious_tree_events_or_execution_authority() {
    let supervisor = NoAgentSupervisor::default();
    let id = AgentId(Uuid::new_v4());
    let mut events = supervisor.subscribe();
    assert!(supervisor.tree().await.unwrap().is_empty());
    assert!(supervisor.inspect(id, None).await.unwrap().is_empty());
    assert!(
        matches!(supervisor.replay(id,None).await,Err(SupervisionError::NotFound(found)) if found==id)
    );
    assert!(matches!(
        supervisor.send_message(id, "message".into()).await,
        Err(SupervisionError::Unavailable)
    ));
    assert!(matches!(
        supervisor.follow_up(id, "task".into()).await,
        Err(SupervisionError::Unavailable)
    ));
    assert!(matches!(
        supervisor.cancel(id).await,
        Err(SupervisionError::Unavailable)
    ));
    assert!(matches!(
        events.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
    assert!(
        matches!(runtime_error(subagent::RuntimeError::Persistence("owned store unavailable".into())),SupervisionError::Failed(message) if message.contains("owned store unavailable"))
    );
}
