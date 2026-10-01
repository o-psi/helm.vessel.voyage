//! Durable ordinary Goal data accounting with explicit, synthetic observations.
//! No observer report grants continuation or establishes remote/native cleanup.
use super::super::family_fixture::*;
use super::*;
use serde_json::json;
use voyage_protocol::{goals::*, process::RuntimeCommand};

async fn active_goal(f: &Fixture) {
    f.owner
        .update_goal(
            super::super::super::journal::GoalAuthority {
                installation_id: f.actor.installation_id,
                principal_id: f.actor.principal_id,
                grant: None,
            },
            RuntimeCommand::GoalUpdate {
                command_id: Uuid::new_v4(),
                expected_revision: f.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                action: GoalAction::Set {
                    objective: "Owned ordinary Goal data transitions".into(),
                    limits: GoalLimits {
                        runs: 10,
                        tokens: 1000,
                        elapsed_ms: 30_000,
                        no_progress_runs: 3,
                    },
                    replace_goal_id: None,
                    continue_automatically: true,
                },
            },
        )
        .await
        .unwrap();
}
async fn reserved(f: &Fixture) -> (Uuid, super::super::RunOwner, UsageObserver) {
    active_goal(f).await;
    let reservation = f
        .owner
        .reserve_goal_turn(
            f.owner.goal().await.unwrap().revision,
            f.incarnation,
            "Reserved Goal input".into(),
        )
        .await
        .unwrap();
    let RuntimeCommand::Submit {
        budget,
        coordination,
        command_id,
        expected_revision,
        expires_at_ms,
        prompt,
    } = reservation.command
    else {
        panic!("ordinary Goal reservation")
    };
    let request = super::super::TurnAdmission {
        budget,
        coordination,
        operator_name: None,
        command_id,
        machine_id: f.actor.installation_id,
        principal_id: f.actor.principal_id,
        session_id: f.owner.session_id(),
        expected_revision,
        expires_at_ms: expires_at_ms as i64,
        prompt,
        parts: vec![],
    };
    let super::super::Admission::New(mut run) = f.owner.admit(request).await.unwrap() else {
        panic!("fresh reserved Goal turn")
    };
    run.register_local_cleanup().await.unwrap();
    run.start_operator().await.unwrap();
    let observer = UsageObserver {
        owner: f.owner.clone(),
        command: command_id,
        incarnation: f.incarnation,
    };
    (command_id, run, observer)
}
fn allocation() -> AllocationRequest {
    AllocationRequest {
        command_id: Uuid::new_v4(),
        destination: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        tokens: 100,
        elapsed_ms: 10_000,
        expires_at_ms: expiry(),
    }
}
fn usage(budget: ExecutionBudget, complete: bool, cleanup: bool) -> ExecutionUsage {
    ExecutionUsage {
        session_id: budget.session_id,
        budget,
        run_id: Uuid::new_v4(),
        input_tokens: 3,
        output_tokens: 2,
        elapsed_ms: 25,
        complete,
        cleanup_observed: cleanup,
    }
}

#[tokio::test]
async fn idle_goal_wrappers_are_read_only_and_do_not_manufacture_a_meter_or_report_binding() {
    let f = Fixture::new().await;
    assert!(f.owner.goal_continuation().await.unwrap().is_none());
    assert!(f.owner.goal_obstruction().await.unwrap().is_none());
    assert!(
        f.owner
            .begin_execution_meter(Uuid::new_v4(), f.incarnation, None)
            .await
            .unwrap()
            .is_none()
    );
    assert!(f.owner.goal_allocations(0, 128).await.unwrap().is_empty());
    for limit in [0, 129] {
        assert!(f.owner.goal_allocations(0, limit).await.is_err());
    }
    let (_, run) = f.running().await;
    assert!(f.owner.goal_tool(run.run_id).await.unwrap().is_none());
    run.finish_operator(Ok("ordinary non-Goal".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
    assert!(
        f.owner
            .settle_metered_run(run.run_id, None, true)
            .await
            .unwrap()
            .is_none()
    );
    assert!(f.owner.goal().await.unwrap().goal.is_none());
}

#[tokio::test]
async fn goal_reservation_and_meter_are_incarnation_bound_one_use_and_human_pause_revokes_continuation()
 {
    let f = Fixture::new().await;
    let (command, run, _) = reserved(&f).await;
    assert!(
        f.owner
            .begin_execution_meter(command, Uuid::new_v4(), None)
            .await
            .is_err()
    );
    assert!(
        f.owner
            .begin_execution_meter(command, f.incarnation, None)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.owner
            .begin_execution_meter(command, f.incarnation, None)
            .await
            .is_err()
    );
    let snapshot = f.owner.goal().await.unwrap();
    let goal = snapshot.goal.unwrap();
    f.owner
        .update_goal(
            super::super::super::journal::GoalAuthority {
                installation_id: f.actor.installation_id,
                principal_id: f.actor.principal_id,
                grant: None,
            },
            RuntimeCommand::GoalUpdate {
                command_id: Uuid::new_v4(),
                expected_revision: f.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                action: GoalAction::Pause { goal_id: goal.id },
            },
        )
        .await
        .unwrap();
    assert!(f.owner.goal_continuation().await.unwrap().is_none());
    assert!(
        f.owner
            .reserve_goal_turn(
                f.owner.goal().await.unwrap().revision,
                f.incarnation,
                "must not resume".into()
            )
            .await
            .is_err()
    );
    run.finish_operator(Ok("terminal fixture".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
}

#[tokio::test]
async fn observed_usage_settles_once_without_declaring_the_objective_complete() {
    let f = Fixture::new().await;
    let (command, run, observer) = reserved(&f).await;
    f.owner
        .begin_execution_meter(command, f.incarnation, None)
        .await
        .unwrap()
        .unwrap();
    let request = RequestObservation {
        request_id: Uuid::new_v4(),
        revision: 0,
        input_tokens: Some(3),
        output_tokens: Some(2),
        complete: true,
    };
    observer.record(request.clone()).await.unwrap();
    observer.record(request).await.unwrap();
    run.finish_operator(Ok("one reserved step complete".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
    let measurement = Some(GoalMeasurement {
        input_tokens: 3,
        output_tokens: 2,
        elapsed_ms: 25,
        complete: true,
    });
    let first = f
        .owner
        .settle_metered_run(run.run_id, measurement.clone(), true)
        .await
        .unwrap();
    assert!(first.is_some());
    assert_eq!(
        f.owner
            .settle_metered_run(run.run_id, measurement, true)
            .await
            .unwrap(),
        first
    );
    let goal = f.owner.goal().await.unwrap().goal.unwrap();
    assert_eq!(
        (
            goal.usage.runs,
            goal.usage.input_tokens,
            goal.usage.output_tokens
        ),
        (1, 3, 2)
    );
    assert_ne!(goal.status, GoalStatus::Complete);
}

#[tokio::test]
async fn incomplete_usage_and_unobserved_cleanup_fence_automatic_continuation_without_refunding_uncertainty()
 {
    let f = Fixture::new().await;
    let (command, run, observer) = reserved(&f).await;
    f.owner
        .begin_execution_meter(command, f.incarnation, None)
        .await
        .unwrap()
        .unwrap();
    observer
        .record(RequestObservation {
            request_id: Uuid::new_v4(),
            revision: 0,
            input_tokens: Some(4),
            output_tokens: None,
            complete: false,
        })
        .await
        .unwrap();
    run.finish_operator(Err("owned synthetic failure".into()), false)
        .await
        .unwrap();
    let receipt = f
        .owner
        .settle_metered_run(run.run_id, None, false)
        .await
        .unwrap();
    assert!(receipt.is_some());
    let goal = f.owner.goal().await.unwrap().goal.unwrap();
    assert!(!goal.continuation_authorized);
    assert!(goal.usage.unmeasured_runs > 0);
    assert!(f.owner.goal_continuation().await.unwrap().is_none());
    run.confirm_local_cleanup_observed().await.unwrap();
    assert_ne!(
        f.owner.goal().await.unwrap().goal.unwrap().status,
        GoalStatus::Complete
    );
}

#[tokio::test]
async fn allocation_and_nonadmission_close_are_exact_destination_and_receipt_bound() {
    let f = Fixture::new().await;
    let (command, run, observer) = reserved(&f).await;
    f.owner
        .begin_execution_meter(command, f.incarnation, None)
        .await
        .unwrap()
        .unwrap();
    let request = allocation();
    let budget = observer.allocate(request.clone()).await.unwrap();
    assert_eq!(budget.command_id, request.command_id);
    assert_eq!(observer.allocate(request.clone()).await.unwrap(), budget);
    let mut changed = request.clone();
    changed.tokens += 1;
    assert!(observer.allocate(changed).await.is_err());
    assert!(
        observer
            .close_allocation(
                request.destination,
                request.command_id,
                json!({"status":"unknown","command_id":request.command_id})
            )
            .await
            .is_err()
    );
    assert!(
        observer
            .close_allocation(
                Uuid::new_v4(),
                request.command_id,
                json!({"status":"not_admitted","command_id":request.command_id})
            )
            .await
            .is_err()
    );
    observer
        .close_allocation(
            request.destination,
            request.command_id,
            json!({"status":"not_admitted","command_id":request.command_id}),
        )
        .await
        .unwrap();
    observer
        .close_allocation(
            request.destination,
            request.command_id,
            json!({"status":"not_admitted","command_id":request.command_id}),
        )
        .await
        .unwrap();
    run.finish_operator(Ok("no child dispatched".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
    let rows = f.owner.goal_allocations(0, 128).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].closed);
    assert!(rows[0].dispatch.is_none());
}

#[tokio::test]
async fn child_dispatch_is_pinned_before_effect_and_changed_payload_or_unknown_closure_fails() {
    use voyage_protocol::vessel::VoyageCommand;
    let f = Fixture::new().await;
    let (command, run, observer) = reserved(&f).await;
    f.owner
        .begin_execution_meter(command, f.incarnation, None)
        .await
        .unwrap()
        .unwrap();
    let request = allocation();
    let budget = observer.allocate(request.clone()).await.unwrap();
    let dispatch = crate::provider::goal_meter::AllocationDispatch::Voyage {
        command: Box::new(VoyageCommand::Submit {
            budget: Some(budget.clone()),
            coordination: None,
            command_id: budget.command_id,
            expected_revision: 0,
            expires_at_ms: budget.expires_at_ms,
            prompt: "exact child intent".into(),
        }),
    };
    observer.dispatch(dispatch.clone()).await.unwrap();
    observer.dispatch(dispatch.clone()).await.unwrap();
    let mut changed = dispatch;
    if let crate::provider::goal_meter::AllocationDispatch::Voyage { command } = &mut changed {
        if let VoyageCommand::Submit { prompt, .. } = command.as_mut() {
            *prompt = "changed".into();
        }
    }
    assert!(observer.dispatch(changed).await.is_err());
    run.finish_operator(Ok("parent terminal".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
    assert!(
        f.owner
            .close_goal_allocation(request.destination, request.command_id, None)
            .await
            .is_err()
    );
    let rows = f.owner.goal_allocations(0, 128).await.unwrap();
    assert!(rows[0].dispatch.is_some());
    assert!(!rows[0].closed);
}

#[tokio::test]
async fn child_usage_attribution_is_immutable_and_late_observation_cannot_restore_authority() {
    let f = Fixture::new().await;
    let (command, run, observer) = reserved(&f).await;
    f.owner
        .begin_execution_meter(command, f.incarnation, None)
        .await
        .unwrap()
        .unwrap();
    let request = allocation();
    let budget = observer.allocate(request.clone()).await.unwrap();
    let receipt = usage(budget, false, false);
    assert!(
        observer
            .settle_allocation(Uuid::new_v4(), receipt.clone())
            .await
            .is_err()
    );
    observer
        .settle_allocation(request.destination, receipt.clone())
        .await
        .unwrap();
    run.finish_operator(Ok("parent terminal".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
    f.owner
        .settle_metered_run(run.run_id, None, false)
        .await
        .unwrap();
    let mut observed = receipt.clone();
    observed.input_tokens += 5;
    observed.cleanup_observed = true;
    assert!(
        f.owner
            .reconcile_goal_allocation(request.destination, receipt.clone(), Some(observed.clone()))
            .await
            .unwrap()
    );
    assert!(
        !f.owner
            .reconcile_goal_allocation(request.destination, receipt, Some(observed))
            .await
            .unwrap()
    );
    let goal = f.owner.goal().await.unwrap().goal.unwrap();
    assert!(!goal.continuation_authorized);
    assert!(goal.usage.unmeasured_runs > 0);
    assert_ne!(goal.status, GoalStatus::Complete);
}

#[tokio::test]
async fn abandoned_unadmitted_goal_reservation_never_becomes_a_replayed_turn() {
    let f = Fixture::new().await;
    active_goal(&f).await;
    let reserved = f
        .owner
        .reserve_goal_turn(
            f.owner.goal().await.unwrap().revision,
            f.incarnation,
            "retained reservation".into(),
        )
        .await
        .unwrap();
    let command = reserved.command.mutation_id().unwrap();
    f.owner
        .abandon_goal_turn(command, GoalStopReason::Interrupted)
        .await
        .unwrap();
    f.owner.recover_goal_turn().await.unwrap();
    assert!(f.owner.goal_continuation().await.unwrap().is_none());
    assert_eq!(f.owner.snapshot().await.unwrap().session.messages.len(), 0);
    let receipt = f.owner.process_receipt(command).await.unwrap().unwrap();
    assert_eq!(receipt["status"], "not_admitted");
    assert_ne!(
        f.owner.goal().await.unwrap().goal.unwrap().status,
        GoalStatus::Complete
    );
}

#[tokio::test]
async fn offline_goal_fence_and_explicit_stop_preserve_objective_and_never_enable_continuation() {
    let f = Fixture::new().await;
    active_goal(&f).await;
    let before = f.owner.goal().await.unwrap().goal.unwrap();
    f.owner.fence_offline_goal().await.unwrap();
    let fenced = f.owner.goal().await.unwrap().goal.unwrap();
    assert_eq!(fenced.objective, before.objective);
    assert!(!fenced.continuation_authorized);
    assert!(
        f.owner
            .stop_goal(0, GoalStopReason::ProviderFailure)
            .await
            .is_err()
    );
    f.owner
        .stop_goal(
            f.owner.goal().await.unwrap().revision,
            GoalStopReason::UnresolvedEffects,
        )
        .await
        .unwrap();
    assert_ne!(
        f.owner.goal().await.unwrap().goal.unwrap().status,
        GoalStatus::Complete
    );
}

#[tokio::test]
async fn goal_report_read_cannot_change_control_and_report_requires_current_canonical_evidence() {
    let f = Fixture::new().await;
    let (command, run, _) = reserved(&f).await;
    f.owner
        .begin_execution_meter(command, f.incarnation, None)
        .await
        .unwrap()
        .unwrap();
    let tool = f.owner.goal_tool(run.run_id).await.unwrap().unwrap();
    let context = crate::tools::reliability_tests::context(f.root.path());
    let before = f.owner.goal().await.unwrap();
    let read = tool
        .execute(json!({"action":"read"}), &context)
        .await
        .unwrap();
    assert!(read.contains("Owned ordinary Goal"));
    assert_eq!(f.owner.goal().await.unwrap(), before);
    assert!(tool.execute(json!({"action":"report","report":{"outcome":"complete","summary":"unsupported success","evidence":[{"call_id":"invented","conclusion":"not canonical"}]}}),&context).await.is_err());
    assert_eq!(f.owner.goal().await.unwrap(), before);
    run.finish_operator(Ok("terminal without report".into()), false)
        .await
        .unwrap();
    run.confirm_local_cleanup_observed().await.unwrap();
}
