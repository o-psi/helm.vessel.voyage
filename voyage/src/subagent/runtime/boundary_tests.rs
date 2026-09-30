//! Source-prepared cancellation/mailbox assertions with a deterministic executor.
//! No model/provider, shell, terminal, worktree or process effects occur.
use super::*;
use futures_util::FutureExt;
use std::sync::atomic::{AtomicUsize, Ordering};
struct Mailbox {
    started: mpsc::UnboundedSender<AgentId>,
    executions: Arc<AtomicUsize>,
}
#[async_trait]
impl SubagentExecutor for Mailbox {
    async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        if context.task == "mailbox" {
            assert!(matches!(
                context.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            ));
            let mut inbox = context.take_inbox();
            assert!(matches!(
                context.try_recv(),
                Err(mpsc::error::TryRecvError::Disconnected)
            ));
            self.started.send(context.id).unwrap();
            let one = inbox.recv().await.ok_or("first delivery absent")?;
            let two = inbox.recv().await.ok_or("followup absent")?;
            if one != InboxMessage::Message("first".into())
                || two != InboxMessage::FollowUp("second".into())
            {
                return Err("delivery order or identity changed".into());
            }
            Ok(SubagentResult {
                summary: "same execution received ordered messages".into(),
            })
        } else {
            self.started.send(context.id).unwrap();
            context.cancellation.cancelled().await;
            Err("executor acknowledged cancellation".into())
        }
    }
}
fn fixture() -> (
    SubagentRuntime,
    mpsc::UnboundedReceiver<AgentId>,
    Arc<AtomicUsize>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let runtime = SubagentRuntime::new(
        Arc::new(Mailbox {
            started: tx,
            executions: count.clone(),
        }),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 256,
        },
        None,
    )
    .unwrap();
    (runtime, rx, count)
}
fn request(root: &std::path::Path, task: &str, parent: Option<AgentId>) -> SpawnRequest {
    let budget = AgentBudget {
        max_tokens: 100,
        max_terminals: 0,
    };
    SpawnRequest {
        parent_id: parent,
        name: "fixture".into(),
        task: task.into(),
        policy: AgentPolicy {
            access: crate::config::AccessMode::ReadOnly,
            readable_roots: vec![root.into()],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: crate::subagent::ApprovalPolicy::Deny,
            budget: budget.clone(),
        },
        budget,
        worktree: None,
        branch: None,
    }
}
async fn wait(runtime: &SubagentRuntime, id: AgentId) -> Result<SubagentResult, String> {
    tokio::time::timeout(std::time::Duration::from_secs(2), runtime.wait(id))
        .await
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn mailbox_handoff_preserves_message_order_and_followup_does_not_spawn_again() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut started, count) = fixture();
    let id = runtime
        .spawn(request(root.path(), "mailbox", None))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), started.recv())
            .await
            .unwrap(),
        Some(id)
    );
    runtime.send_message(id, "first").await.unwrap();
    assert_eq!(runtime.follow_up(id, "second").await.unwrap(), id);
    assert_eq!(
        wait(&runtime, id).await.unwrap().summary,
        "same execution received ordered messages"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(
        runtime.wait(id).await.unwrap().unwrap().summary,
        "same execution received ordered messages"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let events = runtime.events_after(0).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| e.agent_id == id && matches!(e.kind, SubagentEventKind::Started))
            .count(),
        1
    );
    runtime.shutdown().await;
}
#[tokio::test]
async fn ancestor_cancellation_retires_queued_descendants_without_executing_them() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut started, count) = fixture();
    let parent = runtime
        .spawn(request(root.path(), "block", None))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), started.recv())
            .await
            .unwrap(),
        Some(parent)
    );
    let child = runtime
        .spawn(request(root.path(), "block", Some(parent)))
        .await
        .unwrap();
    let grandchild = runtime
        .spawn(request(root.path(), "block", Some(child)))
        .await
        .unwrap();
    runtime.cancel(parent).await.unwrap();
    for id in [parent, child, grandchild] {
        assert!(wait(&runtime, id).await.is_err());
    }
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let events = runtime.events_after(0).await;
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.kind, SubagentEventKind::Started))
            .count(),
        1
    );
    for id in [parent, child, grandchild] {
        assert!(
            events
                .iter()
                .any(|e| e.agent_id == id && matches!(e.kind, SubagentEventKind::Cancelled))
        );
    }
    runtime.shutdown().await;
}
#[tokio::test]
async fn shutdown_releases_full_inbox_sender_without_dispatching_pending_messages() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut started, count) = fixture();
    let id = runtime
        .spawn(request(root.path(), "block", None))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), started.recv())
            .await
            .unwrap(),
        Some(id)
    );
    for _ in 0..64 {
        runtime.send_message(id, "queued").await.unwrap();
    }
    let blocked = runtime.send_message(id, "overflow");
    tokio::pin!(blocked);
    assert!(blocked.as_mut().now_or_never().is_none());
    tokio::time::timeout(std::time::Duration::from_secs(2), runtime.shutdown())
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(std::time::Duration::from_secs(1), blocked)
            .await
            .unwrap(),
        Err(RuntimeError::Terminal(_))
    ));
    assert!(wait(&runtime, id).await.is_err());
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert!(
        runtime
            .spawn(request(root.path(), "late", None))
            .await
            .is_err()
    );
}

struct Panicking {
    executions: Arc<AtomicUsize>,
}
#[async_trait]
impl SubagentExecutor for Panicking {
    async fn execute(&self, context: ExecutionContext) -> Result<SubagentResult, String> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        if context.task == "cancel-panic" {
            context.cancellation.cancel();
            panic!("PRIVATE executor panic");
        }
        if context.task == "panic" {
            panic!("PRIVATE executor panic");
        }
        Ok(SubagentResult {
            summary: "independent next execution".into(),
        })
    }
}
#[tokio::test]
async fn executor_unwind_is_authored_failure_without_replay_or_lost_capacity() {
    let root = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let runtime = SubagentRuntime::new(
        Arc::new(Panicking {
            executions: count.clone(),
        }),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 32,
        },
        None,
    )
    .unwrap();
    let id = runtime
        .spawn(request(root.path(), "panic", None))
        .await
        .unwrap();
    assert_eq!(
        wait(&runtime, id).await.unwrap_err(),
        "subagent executor interrupted"
    );
    assert_eq!(
        wait(&runtime, id).await.unwrap_err(),
        "subagent executor interrupted"
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    let events = runtime.events_after(0).await;
    assert!(!serde_json::to_string(&events).unwrap().contains("PRIVATE"));
    assert!(events.iter().any(|e|e.agent_id==id&&matches!(&e.kind,SubagentEventKind::Failed{error} if error=="subagent executor interrupted")));
    let next = runtime
        .spawn(request(root.path(), "next", None))
        .await
        .unwrap();
    assert_eq!(
        wait(&runtime, next).await.unwrap().summary,
        "independent next execution"
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    runtime.shutdown().await;
}
#[tokio::test]
async fn explicit_cancellation_remains_cancelled_when_executor_unwinds_in_same_poll() {
    let root = tempfile::tempdir().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let runtime = SubagentRuntime::new(
        Arc::new(Panicking {
            executions: count.clone(),
        }),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 32,
        },
        None,
    )
    .unwrap();
    let id = runtime
        .spawn(request(root.path(), "cancel-panic", None))
        .await
        .unwrap();
    assert!(wait(&runtime, id).await.is_err());
    let events = runtime.events_after(0).await;
    assert!(
        events
            .iter()
            .any(|e| e.agent_id == id && matches!(e.kind, SubagentEventKind::Cancelled))
    );
    assert!(
        !events
            .iter()
            .any(|e| e.agent_id == id && matches!(e.kind, SubagentEventKind::Failed { .. }))
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
    runtime.shutdown().await;
}
