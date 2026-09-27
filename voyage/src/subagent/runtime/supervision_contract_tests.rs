use super::*;
struct Offline;
#[async_trait]
impl SubagentExecutor for Offline {
    async fn execute(&self, mut context: ExecutionContext) -> Result<SubagentResult, String> {
        context.progress("offline started").await;
        if context.task == "hold" {
            let cancellation = context.cancellation.clone();
            tokio::select! {
                _ = cancellation.cancelled() => Err("cancelled".into()),
                message = context.recv() => Ok(SubagentResult { summary: format!("{message:?}") }),
            }
        } else {
            Ok(SubagentResult {
                summary: context.task.clone(),
            })
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
        name: "offline".into(),
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
async fn self_and_ancestor_waits_refuse_without_surrendering_execution() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(
        Arc::new(Offline),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 32,
        },
        None,
    )
    .unwrap();
    let parent = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    assert!(matches!(
        runtime.wait_many_as(Some(parent), &[parent]).await,
        Err(RuntimeError::Invalid(_))
    ));
    let mut child = request(root.path(), "hold");
    child.parent_id = Some(parent);
    let child = runtime.spawn(child).await.unwrap();
    assert!(matches!(
        runtime.wait_many_as(Some(child), &[parent]).await,
        Err(RuntimeError::Invalid(_))
    ));
    assert!(
        runtime
            .wait_many_as(Some(parent), &[])
            .await
            .unwrap()
            .is_empty()
    );
    runtime.cancel(child).await.unwrap();
    runtime.cancel(parent).await.unwrap();
    assert!(runtime.wait(child).await.unwrap().is_err());
    assert!(runtime.wait(parent).await.unwrap().is_err());
    runtime.shutdown().await;
}
#[tokio::test]
async fn yielded_parent_slot_allows_queued_child_to_finish() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(
        Arc::new(Offline),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 32,
        },
        None,
    )
    .unwrap();
    let parent = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    // Wait for the first executor to actually hold the only permit.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if runtime
                .control(parent)
                .await
                .unwrap()
                .permit
                .lock()
                .await
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut child = request(root.path(), "child-result");
    child.parent_id = Some(parent);
    let child = runtime.spawn(child).await.unwrap();
    let results = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        runtime.wait_many_as(Some(parent), &[child]),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(results[0].as_ref().unwrap().summary, "child-result");
    runtime.send_message(parent, "resume").await.unwrap();
    assert!(
        runtime
            .wait(parent)
            .await
            .unwrap()
            .unwrap()
            .summary
            .contains("resume")
    );
    runtime.shutdown().await;
}
#[tokio::test]
async fn follow_up_active_agent_is_an_inbox_delivery_not_a_second_spawn() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(Arc::new(Offline), RuntimeLimits::default(), None).unwrap();
    let id = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    assert_eq!(runtime.follow_up(id, "new direction").await.unwrap(), id);
    assert!(
        runtime
            .wait(id)
            .await
            .unwrap()
            .unwrap()
            .summary
            .contains("new direction")
    );
    assert!(runtime.send_message(id, "late").await.is_err());
    assert!(runtime.follow_up(id, "late").await.is_err());
    runtime.shutdown().await;
}
#[test]
fn dropped_wait_guard_and_utf8_previews_preserve_supervision_contracts() {
    let token = CancellationToken::new();
    drop(CancelDroppedWait(Some(token.clone())));
    assert!(token.is_cancelled());
    let token = CancellationToken::new();
    let mut guard = CancelDroppedWait(Some(token.clone()));
    guard.0 = None;
    drop(guard);
    assert!(!token.is_cancelled());
    assert_eq!(preview("small"), "small");
    let text = "界".repeat(2000);
    let output = preview(&text);
    assert!(output.ends_with("… [preview]"));
    assert!(text.starts_with(output.strip_suffix("… [preview]").unwrap()));
    assert!(output.len() < text.len());
}

async fn await_permit(runtime: &SubagentRuntime, id: AgentId) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if runtime
                .control(id)
                .await
                .unwrap()
                .permit
                .lock()
                .await
                .is_some()
            {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn sibling_wait_cycle_and_duplicate_wait_are_rejected_without_deadlock() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(
        Arc::new(Offline),
        RuntimeLimits {
            max_concurrency: 2,
            event_history: 32,
        },
        None,
    )
    .unwrap();
    let first = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    let second = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    await_permit(&runtime, first).await;
    await_permit(&runtime, second).await;
    let waiter_runtime = runtime.clone();
    let waiter =
        tokio::spawn(async move { waiter_runtime.wait_many_as(Some(first), &[second]).await });
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if !runtime
                .control(first)
                .await
                .unwrap()
                .waiting_for
                .lock()
                .await
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let cycle = runtime
        .wait_many_as(Some(second), &[first])
        .await
        .unwrap_err();
    assert!(cycle.to_string().contains("cyclic"), "{cycle}");
    let duplicate = runtime
        .wait_many_as(Some(first), &[second])
        .await
        .unwrap_err();
    assert!(
        duplicate.to_string().contains("already waiting"),
        "{duplicate}"
    );
    assert!(
        runtime
            .control(second)
            .await
            .unwrap()
            .permit
            .lock()
            .await
            .is_some()
    );
    runtime
        .send_message(second, "release sibling")
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(3), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        outcome[0]
            .as_ref()
            .unwrap()
            .summary
            .contains("release sibling")
    );
    assert!(
        runtime
            .control(first)
            .await
            .unwrap()
            .waiting_for
            .lock()
            .await
            .is_empty()
    );
    assert!(
        runtime
            .control(first)
            .await
            .unwrap()
            .permit
            .lock()
            .await
            .is_some()
    );
    runtime.cancel(first).await.unwrap();
    assert!(runtime.wait(first).await.unwrap().is_err());
    runtime.shutdown().await;
}

#[tokio::test]
async fn aborted_yield_cancels_parent_but_does_not_cancel_independent_target() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(
        Arc::new(Offline),
        RuntimeLimits {
            max_concurrency: 1,
            event_history: 32,
        },
        None,
    )
    .unwrap();
    let parent = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    await_permit(&runtime, parent).await;
    let target = runtime.spawn(request(root.path(), "hold")).await.unwrap();
    let waiter_runtime = runtime.clone();
    let waiter =
        tokio::spawn(async move { waiter_runtime.wait_many_as(Some(parent), &[target]).await });
    await_permit(&runtime, target).await;
    waiter.abort();
    assert!(waiter.await.unwrap_err().is_cancelled());
    let parent_outcome =
        tokio::time::timeout(std::time::Duration::from_secs(3), runtime.wait(parent))
            .await
            .unwrap()
            .unwrap();
    assert!(parent_outcome.is_err());
    assert!(!runtime.control(target).await.unwrap().cancel.is_cancelled());
    runtime
        .send_message(target, "survives abandoned wait")
        .await
        .unwrap();
    assert!(
        runtime
            .wait(target)
            .await
            .unwrap()
            .unwrap()
            .summary
            .contains("survives abandoned wait")
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn unknown_targets_and_invalid_spawn_requests_do_not_consume_capacity() {
    let root = tempfile::tempdir().unwrap();
    let runtime = SubagentRuntime::new(Arc::new(Offline), RuntimeLimits::default(), None).unwrap();
    let missing = AgentId::new();
    assert!(matches!(
        runtime.wait(missing).await,
        Err(RuntimeError::Unknown(_))
    ));
    assert!(matches!(
        runtime.send_message(missing, "message").await,
        Err(RuntimeError::Unknown(_))
    ));
    for case in 0..3 {
        let mut invalid = request(root.path(), "offline");
        match case {
            0 => invalid.name = "  ".into(),
            1 => invalid.task = "\n\t".into(),
            _ => invalid.parent_id = Some(missing),
        }
        assert!(runtime.spawn(invalid).await.is_err());
    }
    let id = runtime
        .spawn(request(root.path(), "still usable"))
        .await
        .unwrap();
    assert_eq!(
        runtime.wait(id).await.unwrap().unwrap().summary,
        "still usable"
    );
    runtime.shutdown().await;
}
