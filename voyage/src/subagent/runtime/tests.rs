use super::*;
struct Executor;
#[async_trait]
impl SubagentExecutor for Executor {
    async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
        context.progress("fixture started").await;
        let cancel = context.cancellation.clone();
        match context.task.as_str() {
            "fail" => Err("fixture failure".into()),
            "wait" => {
                tokio::select! {_=cancel.cancelled()=>Err("cancelled".into()),message=context.recv()=>Ok(SubagentResult{summary:format!("{message:?}")})}
            }
            _ => Ok(SubagentResult {
                summary: format!("done {}", context.task),
            }),
        }
    }
}
fn request(root: &std::path::Path, task: &str) -> SpawnRequest {
    let budget = AgentBudget {
        max_tokens: 100,
        max_terminals: 1,
    };
    SpawnRequest {
        parent_id: None,
        name: "fixture".into(),
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
        worktree: None,
        branch: None,
    }
}
#[tokio::test]
async fn results_messages_failures_and_history_are_retained_without_reexecution() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(Arc::new(Executor), RuntimeLimits::default(), None).unwrap();
    let good = runtime
        .spawn(request(root.path(), "success"))
        .await
        .unwrap();
    let bad = runtime.spawn(request(root.path(), "fail")).await.unwrap();
    let results = runtime.wait_many(&[good, bad]).await.unwrap();
    assert_eq!(results[0].as_ref().unwrap().summary, "done success");
    assert_eq!(results[1].as_ref().unwrap_err(), "fixture failure");
    assert_eq!(runtime.wait(good).await.unwrap(), results[0]);
    let waiting = runtime.spawn(request(root.path(), "wait")).await.unwrap();
    runtime.send_message(waiting, "hello").await.unwrap();
    assert!(
        runtime
            .wait(waiting)
            .await
            .unwrap()
            .unwrap()
            .summary
            .contains("hello")
    );
    let events = runtime.events_after(0).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, SubagentEventKind::Started))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, SubagentEventKind::Progress { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e.kind, SubagentEventKind::Failed { .. }))
    );
    assert!(runtime.send_message(good, "again").await.is_err());
    runtime.shutdown().await;
    assert!(runtime.spawn(request(root.path(), "late")).await.is_err());
}
#[tokio::test]
async fn queued_cancellation_and_parent_policy_limits_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(
        Arc::new(Executor),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 20,
        },
        None,
    )
    .unwrap();
    let parent = runtime.spawn(request(root.path(), "wait")).await.unwrap();
    let mut child = request(root.path(), "success");
    child.parent_id = Some(parent);
    child.budget.max_tokens = 101;
    assert!(runtime.spawn(child.clone()).await.is_err());
    child.budget.max_tokens = 100;
    child.policy.access = crate::config::AccessMode::Unrestricted;
    assert!(runtime.spawn(child).await.is_err());
    let queued = runtime
        .spawn(request(root.path(), "success"))
        .await
        .unwrap();
    runtime.cancel(queued).await.unwrap();
    assert!(runtime.wait(queued).await.unwrap().is_err());
    runtime.cancel(parent).await.unwrap();
    assert!(runtime.wait(parent).await.unwrap().is_err());
    runtime.shutdown().await;
}
#[tokio::test]
async fn persistent_terminal_records_archive_and_reopen_without_restarting() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("agents.json");
    let store = AgentTreeStore::new(path.clone());
    let runtime =
        SubagentRuntime::new_persistent(Arc::new(Executor), RuntimeLimits::default(), store)
            .await
            .unwrap();
    let id = runtime.spawn(request(root.path(), "once")).await.unwrap();
    let result = runtime.wait(id).await.unwrap().unwrap();
    runtime.shutdown().await;
    drop(runtime);
    let runtime = SubagentRuntime::new_persistent(
        Arc::new(Executor),
        RuntimeLimits::default(),
        AgentTreeStore::new(path),
    )
    .await
    .unwrap();
    assert_eq!(runtime.wait(id).await.unwrap().unwrap(), result);
    assert!(runtime.is_archived(id).await.unwrap());
    assert!(runtime.follow_up(id, "repeat").await.is_err());
    assert!(runtime.get(id).await.unwrap().status.is_terminal());
    runtime.shutdown().await;
}
#[tokio::test]
async fn validation_and_unknown_ids_do_not_admit_work() {
    let root = tempfile::tempdir().unwrap();
    for limits in [
        RuntimeLimits {
            max_concurrency: 0,
            event_history: 1,
        },
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 0,
        },
    ] {
        assert!(SubagentRuntime::new(Arc::new(Executor), limits, None).is_err());
    }
    let runtime = SubagentRuntime::new(Arc::new(Executor), RuntimeLimits::default(), None).unwrap();
    for field in [true, false] {
        let mut r = request(root.path(), "valid");
        if field {
            r.name.clear();
        } else {
            r.task = " ".into();
        }
        assert!(runtime.spawn(r).await.is_err());
    }
    assert!(runtime.get(AgentId::new()).await.is_err());
    assert!(runtime.wait(AgentId::new()).await.is_err());
    assert!(runtime.cancel(AgentId::new()).await.is_err());
    assert!(runtime.list().await.is_empty());
    runtime.shutdown().await;
}
