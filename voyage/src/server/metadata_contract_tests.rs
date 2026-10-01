//! Metadata/lifecycle commands preserve canonical text and exact receipts;
//! accounting, transfer and consent never invent an execution owner.
use super::family_fixture::*;
use super::*;
use crate::attachment::{
    journal::TurnAdmission,
    runtime::{Admission, RunOwner},
};
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;

async fn admitted(state: &Arc<State>) -> RunOwner {
    let revision = state.owner.snapshot().await.unwrap().revision;
    let request = TurnAdmission {
        budget: None,
        coordination: None,
        operator_name: None,
        command_id: Uuid::new_v4(),
        machine_id: state.actor.installation_id,
        principal_id: state.actor.principal_id,
        session_id: state.registration.session_id,
        expected_revision: revision,
        expires_at_ms: expiry() as i64,
        prompt: "Owned canonical metadata fixture".into(),
        parts: vec![],
    };
    let Admission::New(run) = state.owner.admit(request).await.unwrap() else {
        panic!("fresh owned admission")
    };
    run
}
async fn history(state: &Arc<State>) -> Value {
    call(
        state,
        RuntimeCommand::History {
            offset: 0,
            limit: 100,
            expected_revision: None,
        },
    )
    .await
    .unwrap()
}
async fn completed_history(state: &Arc<State>) {
    let mut run = admitted(state).await;
    run.register_local_cleanup().await.unwrap();
    run.start_operator().await.unwrap();
    run.finish_operator(Ok("Canonical fixture answer 世界".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
}

#[tokio::test]
async fn model_metadata_and_snapshot_use_current_saved_selection_and_exact_replay() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let command = RuntimeCommand::SetModel {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        model: "replacement-model".into(),
    };
    let first = call(&state, command.clone()).await.unwrap();
    assert_eq!(call(&state, command).await.unwrap(), first);
    assert_eq!(
        state.owner.snapshot().await.unwrap().session.model,
        "replacement-model"
    );
    let snapshot = call(&state, RuntimeCommand::Snapshot).await.unwrap();
    assert_eq!(snapshot["inference"]["model"], "replacement-model");
    assert_eq!(snapshot["inference_next_turn"], false);
    assert!(snapshot["inference_current"].is_null());
    assert!(
        call(
            &state,
            RuntimeCommand::SetModel {
                command_id: Uuid::new_v4(),
                expected_revision: first["revision"].as_u64().unwrap(),
                expires_at_ms: expiry(),
                model: "".into()
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn steering_commits_exact_queue_receipt_and_cancel_targets_only_the_owned_run() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let mut run = admitted(&state).await;
    let run_id = run.record().await.unwrap().id;
    let steering = run.enable_steering(Arc::new(|_, _, _| Ok(()))).unwrap();
    let cancel = CancellationToken::new();
    *state.active.lock().await = Some(ActiveRun {
        id: run_id,
        inference: json!({"model":"fixture"}),
        cancel: cancel.clone(),
        steering: Some(steering),
    });
    let revision = state.owner.snapshot().await.unwrap().revision;
    let command = RuntimeCommand::Steer {
        parts: vec![],
        coordination: None,
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        run_id,
        prompt: "Exact steering 世界".into(),
    };
    let first = call(&state, command.clone()).await.unwrap();
    let replay = call(&state, command.clone()).await.unwrap();
    assert_eq!(replay["record"], first["record"]);
    assert_eq!(replay["duplicate"], true);
    let mut collision = command;
    if let RuntimeCommand::Steer { prompt, .. } = &mut collision {
        *prompt = "different payload".into();
    }
    assert!(call(&state, collision).await.is_err());
    let revision = state.owner.snapshot().await.unwrap().revision;
    assert!(
        call(
            &state,
            RuntimeCommand::Cancel {
                command_id: Uuid::new_v4(),
                expected_revision: revision,
                expires_at_ms: expiry(),
                run_id: Uuid::new_v4()
            }
        )
        .await
        .is_err()
    );
    assert!(!cancel.is_cancelled());
    let command = RuntimeCommand::Cancel {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        run_id,
    };
    let receipt = call(&state, command.clone()).await.unwrap();
    assert!(cancel.is_cancelled());
    assert_eq!(call(&state, command).await.unwrap(), receipt);
    *state.active.lock().await = None;
    run.fail_before_execution().await.unwrap();
}

#[tokio::test]
async fn active_lifecycle_model_reconciliation_and_missing_steering_fail_without_effects() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let mut run = admitted(&state).await;
    let run_id = run.record().await.unwrap().id;
    *state.active.lock().await = Some(ActiveRun {
        id: run_id,
        inference: json!({}),
        cancel: CancellationToken::new(),
        steering: None,
    });
    let revision = state.owner.snapshot().await.unwrap().revision;
    for command in [
        RuntimeCommand::SetModel {
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: expiry(),
            model: "must-not-change".into(),
        },
        RuntimeCommand::Clear {
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: expiry(),
            confirm_session_id: state.registration.session_id,
        },
        RuntimeCommand::GoalReconcile {
            offset: 0,
            limit: 10,
            fence_children: false,
        },
        RuntimeCommand::Steer {
            parts: vec![],
            coordination: None,
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: expiry(),
            run_id,
            prompt: "operator has no steering".into(),
        },
    ] {
        assert!(call(&state, command).await.is_err());
    }
    assert_eq!(state.owner.snapshot().await.unwrap().revision, revision);
    *state.active.lock().await = None;
    run.fail_before_execution().await.unwrap();
    assert!(
        call(
            &state,
            RuntimeCommand::Steer {
                parts: vec![],
                coordination: None,
                command_id: Uuid::new_v4(),
                expected_revision: state.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                run_id,
                prompt: "retired run".into()
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn approval_route_rechecks_consent_and_replays_without_executing_a_tool() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let mut run = admitted(&state).await;
    run.start_operator().await.unwrap();
    let run_id = run.record().await.unwrap().id;
    let decision = Uuid::new_v4();
    state
        .owner
        .create_decision(
            run_id,
            state.registration.incarnation,
            decision,
            expiry() as i64,
            json!({"kind":"approval","action":"owned fixture"}),
        )
        .await
        .unwrap();
    let decisions = call(&state, RuntimeCommand::Decisions).await.unwrap();
    assert_eq!(decisions.as_array().unwrap().len(), 1);
    let revision = state.owner.snapshot().await.unwrap().revision;
    let response = RuntimeCommand::Respond {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        run_id,
        decision_id: decision,
        response: json!("approved"),
    };
    let first = call(&state, response.clone()).await.unwrap();
    assert_eq!(call(&state, response).await.unwrap(), first);
    assert_eq!(
        state.owner.decision_response(decision).await.unwrap(),
        Some(json!("approved"))
    );
    assert!(
        call(
            &state,
            RuntimeCommand::Respond {
                command_id: Uuid::new_v4(),
                expected_revision: state.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                run_id,
                decision_id: decision,
                response: json!("denied")
            }
        )
        .await
        .is_err()
    );
    run.finish_operator(Ok("Consent observed; no tool was executed".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
}

#[tokio::test]
async fn compact_preserves_canonical_text_clear_has_exact_confirmation_and_branch_pins_settings() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    completed_history(&state).await;
    let before = history(&state).await;
    let revision = state.owner.snapshot().await.unwrap().revision;
    let output = state.owner.process_snapshot().await.unwrap();
    let run: Uuid = output["run"]["run_id"].as_str().unwrap().parse().unwrap();
    let read = call(
        &state,
        RuntimeCommand::RunOutput {
            run_id: run,
            offset: 0,
            limit: 1000,
        },
    )
    .await
    .unwrap();
    assert!(read.to_string().contains("Canonical fixture answer"));
    let chunk = call(
        &state,
        RuntimeCommand::MessageChunk {
            index: 1,
            offset: 0,
            limit: 1000,
            expected_revision: revision,
        },
    )
    .await
    .unwrap();
    assert!(chunk.to_string().contains("Canonical fixture answer"));

    let compact = RuntimeCommand::Compact {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        retain: 1,
        preserve_canonical: true,
    };
    let receipt = call(&state, compact.clone()).await.unwrap();
    assert_eq!(call(&state, compact).await.unwrap(), receipt);
    assert_eq!(history(&state).await["messages"], before["messages"]);
    let revision = state.owner.snapshot().await.unwrap().revision;
    let branch_id = Uuid::new_v4();
    let branch = RuntimeCommand::Branch {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        branch_id,
        name: Some("Owned branch".into()),
        through_message: None,
    };
    let first = call(&state, branch.clone()).await.unwrap();
    assert_eq!(call(&state, branch).await.unwrap(), first);
    assert_eq!(history(&state).await["messages"], before["messages"]);
    let revision = state.owner.snapshot().await.unwrap().revision;
    assert!(
        call(
            &state,
            RuntimeCommand::Clear {
                command_id: Uuid::new_v4(),
                expected_revision: revision,
                expires_at_ms: expiry(),
                confirm_session_id: Uuid::new_v4()
            }
        )
        .await
        .is_err()
    );
    let clear = RuntimeCommand::Clear {
        command_id: Uuid::new_v4(),
        expected_revision: revision,
        expires_at_ms: expiry(),
        confirm_session_id: state.registration.session_id,
    };
    let receipt = call(&state, clear.clone()).await.unwrap();
    assert_eq!(call(&state, clear).await.unwrap(), receipt);
    assert!(
        history(&state).await["messages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn archive_and_delete_retire_only_after_durable_acceptance_and_deleted_replay_is_exact() {
    for deleting in [false, true] {
        let (_root, state, _provider) = family_fixture::fixture().await;
        let input = Uuid::new_v4();
        state
            .workflows
            .store(
                state.actor.principal_id,
                input,
                vec![("owned".into(), "synthetic value".into())],
            )
            .await
            .unwrap();
        let command = if deleting {
            RuntimeCommand::Delete {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: expiry(),
                confirm_session_id: state.registration.session_id,
            }
        } else {
            RuntimeCommand::Archive {
                command_id: Uuid::new_v4(),
                expected_revision: 0,
                expires_at_ms: expiry(),
                archived: true,
            }
        };
        let receipt = call(&state, command.clone()).await.unwrap();
        assert!(state.shutdown.is_cancelled());
        assert_eq!(state.archive_receipt.lock().await.as_ref(), Some(&receipt));
        let snapshot = state.owner.process_snapshot().await.unwrap();
        assert_eq!(
            snapshot["lifecycle"][if deleting { "deleted" } else { "archived" }],
            true
        );
        if deleting {
            assert!(!state.workflows.pending().await);
            assert_eq!(call(&state, command).await.unwrap(), receipt);
        } else {
            state.workflows.clear().await;
        }
    }
    let (_root, state, _provider) = family_fixture::fixture().await;
    call(
        &state,
        RuntimeCommand::Archive {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: expiry(),
            archived: false,
        },
    )
    .await
    .unwrap();
    assert!(!state.shutdown.is_cancelled());
}

#[tokio::test]
async fn relinquish_exports_private_text_once_and_collision_retains_original_receipt() {
    let (_root, state, _provider) = family_fixture::fixture().await;
    let command = RuntimeCommand::Relinquish {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: expiry(),
        transfer_id: Uuid::new_v4(),
        destination_vessel_id: Uuid::new_v4(),
        prepare_digest: "a".repeat(64),
    };
    assert!(
        super::transfer::relinquish(&state, RuntimeCommand::Health)
            .await
            .is_err()
    );
    let receipt = call(&state, command.clone()).await.unwrap();
    assert_eq!(receipt["status"], "relinquished");
    assert!(state.shutdown.is_cancelled());
    let path = state
        .directory
        .join("transfers")
        .join(format!("{}.json", receipt["transfer_id"].as_str().unwrap()));
    let original = std::fs::read(&path).unwrap();
    assert_eq!(call(&state, command.clone()).await.unwrap(), receipt);
    assert_eq!(std::fs::read(&path).unwrap(), original);
    let artifact: Value = serde_json::from_slice(&original).unwrap();
    assert_eq!(
        artifact["session_id"],
        state.registration.session_id.to_string()
    );
    assert!(!String::from_utf8_lossy(&original).contains("fixture-token"));
    std::fs::write(&path, b"owned collision").unwrap();
    assert!(super::transfer::relinquish(&state, command).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"owned collision");
    assert_eq!(
        state
            .owner
            .process_receipt(receipt["command_id"].as_str().unwrap().parse().unwrap())
            .await
            .unwrap()
            .unwrap(),
        receipt
    );
}
