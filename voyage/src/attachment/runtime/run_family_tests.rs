//! Run ownership, exact admission and terminal/checkpoint fault boundaries.
use super::family_fixture::*;
use super::*;
use crate::model::Role;
use std::sync::atomic::AtomicUsize;

#[tokio::test]
async fn admission_observes_exact_delivery_without_reentering_preparation_or_allocating_owner() {
    let f = Fixture::new().await;
    let request = f.request().await;
    let prepared = Arc::new(AtomicUsize::new(0));
    let counted = prepared.clone();
    let Admission::New(mut run) = f
        .owner
        .admit_after(request.clone(), Arc::new(|| Ok(now())), move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .await
        .unwrap()
    else {
        panic!("fresh admission")
    };
    let current = run.record().await.unwrap();
    assert_eq!(prepared.load(Ordering::SeqCst), 1);
    let counted = prepared.clone();
    let result = f
        .owner
        .admit_after(
            request.clone(),
            Arc::new(|| anyhow::bail!("clock must not be reused for receipt")),
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("preparation must not replay")
            },
        )
        .await
        .unwrap();
    let Admission::Existing(prior) = result else {
        panic!("duplicate allocated executor")
    };
    assert_eq!(prior.id, current.id);
    assert_eq!(prepared.load(Ordering::SeqCst), 1);
    let mut changed = request.clone();
    changed.prompt = "changed command payload".into();
    assert!(f.owner.admit(changed).await.is_err());
    let mut foreign = request;
    foreign.session_id = Uuid::new_v4();
    assert!(f.owner.lookup_turn(foreign.clone()).await.is_err());
    assert!(f.owner.admit(foreign).await.is_err());
    assert_eq!(f.owner.snapshot().await.unwrap().session.messages.len(), 1);
    run.fail_before_execution().await.unwrap();
}

#[tokio::test]
async fn invalid_clock_and_preparation_failure_do_not_reserve_a_command_or_canonical_input() {
    let f = Fixture::new().await;
    for clock in [
        Arc::new(|| -> anyhow::Result<i64> { Ok(-1) }) as Arc<dyn RuntimeClock>,
        Arc::new(|| Err(anyhow::anyhow!("synthetic unavailable clock"))),
        Arc::new(|| Ok(i64::MAX)),
    ] {
        let request = f.request().await;
        let id = request.command_id;
        assert!(f.owner.admit_with_clock(request, clock).await.is_err());
        assert!(f.owner.process_receipt(id).await.unwrap().is_none());
    }
    let request = f.request().await;
    let id = request.command_id;
    assert!(
        f.owner
            .admit_after(request, Arc::new(SystemClock), || anyhow::bail!(
                "owned preparation refused"
            ))
            .await
            .is_err()
    );
    assert!(f.owner.process_receipt(id).await.unwrap().is_none());
    assert_eq!(f.owner.snapshot().await.unwrap().revision, 0);
}

#[tokio::test]
async fn authority_is_fresh_for_both_new_and_duplicate_admission() {
    let f = Fixture::new().await;
    let authority = Authority::new();
    let request = f.request().await;
    let Admission::New(mut run) = f
        .owner
        .admit_authorized(request.clone(), authority.clone())
        .await
        .unwrap()
    else {
        panic!("fresh")
    };
    authority.valid.store(false, Ordering::SeqCst);
    assert!(
        f.owner
            .admit_authorized(request.clone(), authority.clone())
            .await
            .is_err()
    );
    let other = f.request().await;
    assert!(
        f.owner
            .admit_authorized(other, authority.clone())
            .await
            .is_err()
    );
    assert!(authority.checks.load(Ordering::SeqCst) >= 3);
    assert_eq!(
        f.owner.lookup_turn(request).await.unwrap().unwrap().id,
        run.run_id
    );
    run.fail_before_execution().await.unwrap();
}

#[tokio::test]
async fn retained_checkpoint_excludes_recovery_new_admission_and_cleanup_until_last_callback_drops()
{
    let f = Fixture::new().await;
    let (_, run) = f.admit().await;
    run.register_local_cleanup().await.unwrap();
    let checkpoint = run.checkpoint();
    let another = checkpoint.clone();
    drop(run);
    assert!(!checkpoint.cleanup_exclusive(1));
    assert!(checkpoint.finish_cleanup(1).await.is_err());
    assert!(f.owner.recover_interrupted().await.is_err());
    assert!(f.owner.admit(f.request().await).await.is_err());
    drop(another);
    assert!(checkpoint.cleanup_exclusive(1));
    checkpoint.finish_cleanup(1).await.unwrap();
    let record = f.owner.process_snapshot().await.unwrap();
    assert_eq!(record["run"]["state"], "interrupted");
    assert!(record["pending_cleanup_run"].is_null());
    drop(checkpoint);
    let (_, mut next) = f.admit().await;
    next.fail_before_execution().await.unwrap();
}

#[tokio::test]
async fn checkpoint_wrong_run_identity_fails_with_authored_diagnostic_and_no_history_mutation() {
    let (_root, owner, run) = super::checkpoint_tests::fixture_authorized(None).await;
    let before = owner.snapshot().await.unwrap();
    let checkpoint = run.checkpoint();
    let previous = {
        let mut store = owner.store.lock().unwrap();
        let old = store.run_id;
        store.run_id = Uuid::new_v4();
        old
    };
    assert!(
        checkpoint
            .partial("must not enter another run")
            .await
            .is_err()
    );
    {
        owner.store.lock().unwrap().run_id = previous;
    }
    assert_eq!(owner.snapshot().await.unwrap().revision, before.revision);
    assert!(
        run.token
            .checkpoint_failure
            .lock()
            .unwrap()
            .as_deref()
            .unwrap()
            .contains("state or authority validation failed")
    );
    assert!(run.record().await.unwrap().partial_text.is_empty());
}

#[tokio::test]
async fn private_storage_error_is_not_exposed_as_checkpoint_or_conversation_text() {
    let (_root, owner, run) = super::checkpoint_tests::fixture_authorized(None).await;
    let checkpoint = run.checkpoint();
    let before = owner.snapshot().await.unwrap();
    assert!(
        checkpoint
            .storage_named(Operation::PartialText, |_| Err::<(), _>(
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "synthetic-private-storage-diagnostic"
                )
                .into()
            ))
            .await
            .is_err()
    );
    let reason = run
        .token
        .checkpoint_failure
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert_eq!(
        reason,
        "Checkpoint failed during partial text: storage error."
    );
    assert!(!reason.contains("synthetic-private"));
    assert_eq!(owner.snapshot().await.unwrap().revision, before.revision);
}

#[tokio::test]
async fn execute_refuses_foreign_model_workspace_external_channel_and_poisoned_turn_before_provider()
 {
    for case in 0..4 {
        let f = Fixture::new().await;
        let (_, mut run) = f.admit().await;
        let other = tempfile::tempdir().unwrap();
        let workspace = if case == 0 {
            other.path()
        } else {
            f.root.path()
        };
        let model = if case == 1 { "wrong-model" } else { "fixture" };
        let (agent, calls) = agent(workspace, model, Reply::Text("must not dispatch"));
        if case == 2 {
            run.token.poisoned.store(true, Ordering::SeqCst);
        }
        let input = if case == 3 {
            Some(crate::agent::steering_channel(1).1)
        } else {
            None
        };
        assert!(matches!(
            run.execute(&agent, CancellationToken::new(), input).await,
            Err(AgentError::Checkpoint(_))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(run.record().await.unwrap().state, RunState::Failed);
        assert_eq!(f.owner.snapshot().await.unwrap().session.messages.len(), 1);
    }
}

#[tokio::test]
async fn pre_cancel_and_late_cancel_never_publish_a_successful_terminal_classification() {
    for late in [false, true] {
        let f = Fixture::new().await;
        let (_, mut run) = f.admit().await;
        let (agent, calls) = agent(f.root.path(), "fixture", Reply::Text("owned answer"));
        let cancel = CancellationToken::new();
        if !late {
            cancel.cancel();
        }
        let after = cancel.clone();
        let result = run
            .execute_before_finish(&agent, cancel, None, move || {
                if late {
                    after.cancel();
                }
                Ok(())
            })
            .await;
        assert!(matches!(result, Err(AgentError::Cancelled)));
        assert_eq!(run.record().await.unwrap().state, RunState::Cancelled);
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(late));
    }
}

#[tokio::test]
async fn execution_preserves_exact_accepted_input_and_retains_cleanable_terminal_result() {
    let f = Fixture::new().await;
    let (request, mut run) = f.admit().await;
    run.register_local_cleanup().await.unwrap();
    let original = f.owner.snapshot().await.unwrap().session.messages[0].clone();
    let (agent, calls) = agent(
        f.root.path(),
        "fixture",
        Reply::Text("Ordinary checkpoint answer 世界"),
    );
    let result = run
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(matches!(result.stop_reason, StopReason::Completed));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let saved = f.owner.snapshot().await.unwrap().session;
    assert_eq!(
        serde_json::to_value(&saved.messages[0]).unwrap(),
        serde_json::to_value(original).unwrap()
    );
    assert_eq!(
        saved
            .messages
            .iter()
            .filter(|m| m.role == Role::User && m.content == request.prompt)
            .count(),
        1
    );
    assert!(
        saved
            .messages
            .iter()
            .any(|m| m.role == Role::Assistant && m.content == "Ordinary checkpoint answer 世界")
    );
    assert!(
        run.execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    run.confirm_local_cleanup_observed().await.unwrap();
    assert!(f.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn provider_failure_preserves_authored_failure_and_never_copies_private_diagnostic_to_saved_history()
 {
    let f = Fixture::new().await;
    let (_, mut run) = f.admit().await;
    let (agent, calls) = agent(f.root.path(), "fixture", Reply::Fail);
    assert!(
        run.execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let saved = f.owner.snapshot().await.unwrap();
    assert_eq!(run.record().await.unwrap().state, RunState::Failed);
    assert!(
        !serde_json::to_string(&saved.session)
            .unwrap()
            .contains("synthetic-private-provider-diagnostic")
    );
}

#[tokio::test]
async fn late_finalization_failure_leaves_recovery_obligation_and_does_not_reinvoke_provider() {
    let f = Fixture::new().await;
    let (_, mut run) = f.admit().await;
    run.register_local_cleanup().await.unwrap();
    let (agent, calls) = agent(
        f.root.path(),
        "fixture",
        Reply::Text("accepted before finalization"),
    );
    assert!(
        run.execute_before_finish(&agent, CancellationToken::new(), None, || Err(
            AgentError::Checkpoint(CheckpointError)
        ))
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        run.record().await.unwrap().state,
        RunState::Accepted | RunState::Running
    ));
    assert!(f.owner.recover_interrupted().await.is_err());
    drop(run);
    let interrupted = f.owner.recover_interrupted().await.unwrap().unwrap();
    assert_eq!(interrupted.state, RunState::Interrupted);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!f.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
}

#[tokio::test]
async fn immutable_admitted_input_mismatch_refuses_inference_and_preserves_saved_text() {
    let f = Fixture::new().await;
    let (_, mut run) = f.admit().await;
    run.input.as_mut().unwrap().1 = "forged prompt".into();
    let (agent, calls) = agent(f.root.path(), "fixture", Reply::Text("must not dispatch"));
    assert!(
        run.execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.owner.snapshot().await.unwrap().session.messages[0].content,
        "Original owned user input 世界"
    );
}

#[tokio::test]
async fn unstreamed_reasoning_and_preview_metadata_do_not_invent_canonical_messages() {
    let (_root, owner, run) = super::checkpoint_tests::fixture_authorized(None).await;
    let before = owner.snapshot().await.unwrap().session.messages;
    let checkpoint = run.checkpoint();
    checkpoint
        .unstreamed("unstreamed progress 世界")
        .await
        .unwrap();
    checkpoint
        .reasoning_previews(&[voyage_protocol::reasoning_preview::ReasoningPreview {
            attempt_id: Uuid::new_v4(),
            index: 0,
            kind: voyage_protocol::reasoning_preview::ReasoningKind::Summary,
            text: "bounded reasoning preview".into(),
            truncated: false,
            finalized: false,
        }])
        .await
        .unwrap();
    checkpoint.tool_previews(&[]).await.unwrap();
    let saved = owner.snapshot().await.unwrap().session.messages;
    assert_eq!(
        serde_json::to_value(saved).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert!(
        run.record()
            .await
            .unwrap()
            .partial_text
            .contains("unstreamed progress")
    );
}
