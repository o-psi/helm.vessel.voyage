//! Ordinary assignment, consent, steering and saved observation data transitions.
use super::family_fixture::*;
use super::*;
use crate::attachment::journal::{SteeringActor, SteeringAdmission};
use serde_json::json;
use voyage_protocol::process::{
    AssignmentObservation, AssignmentRequest, ParticipantPolicy, RuntimeCommand,
};

fn assignment(f: &Fixture, run: Uuid) -> (Uuid, AssignmentRequest) {
    let destination = Uuid::new_v4();
    (
        destination,
        AssignmentRequest {
            budget: None,
            assignment_id: Uuid::new_v4(),
            binding_id: Uuid::new_v4(),
            binding_revision: 1,
            parent_vessel_id: Uuid::new_v4(),
            parent_session_id: f.owner.session_id(),
            parent_run_id: run,
            expires_at_ms: expiry(),
            task: "Observe owned fixture work".into(),
            context: vec![],
            policy: ParticipantPolicy {
                access: "read-only".into(),
                legacy_deny_commands: vec![],
                inherit_env: vec![],
                github_enabled: false,
                timeout_secs: 2,
                max_output_bytes: 4096,
                max_subagents: 1,
            },
        },
    )
}
fn observed(
    destination: Uuid,
    request: &AssignmentRequest,
    state: &str,
    cleanup: bool,
) -> AssignmentObservation {
    AssignmentObservation {
        admission_closed: false,
        execution_usage: None,
        execution_usage_observed: None,
        assignment_id: request.assignment_id,
        participant_vessel_id: destination,
        parent_session_id: request.parent_session_id,
        parent_run_id: request.parent_run_id,
        child_session_id: request.assignment_id,
        child_incarnation: Some(Uuid::new_v4()),
        run_id: Some(Uuid::new_v4()),
        state: state.into(),
        cleanup_observed: cleanup,
        result: None,
    }
}
async fn ended(run: &mut RunOwner) {
    run.finish_operator(Ok("Owned operator ended".into()), false)
        .await
        .unwrap();
}

#[tokio::test]
async fn assignment_admission_is_exact_actor_destination_payload_and_parent_scoped() {
    let f = Fixture::new().await;
    let (_, mut run) = f.running().await;
    let run_id = run.record().await.unwrap().id;
    let (destination, request) = assignment(&f, run_id);
    let first = f
        .owner
        .record_assignment(f.actor.principal_id, destination, request.clone())
        .await
        .unwrap();
    assert_eq!(first["state"], "acceptance_unknown");
    let duplicate = f
        .owner
        .record_assignment(f.actor.principal_id, destination, request.clone())
        .await
        .unwrap();
    assert_eq!(duplicate["duplicate"], true);
    assert!(
        f.owner
            .record_assignment(Uuid::new_v4(), destination, request.clone())
            .await
            .is_err()
    );
    assert!(
        f.owner
            .record_assignment(f.actor.principal_id, Uuid::new_v4(), request.clone())
            .await
            .is_err()
    );
    let mut changed = request.clone();
    changed.task = "different work".into();
    assert!(
        f.owner
            .record_assignment(f.actor.principal_id, destination, changed)
            .await
            .is_err()
    );
    assert_eq!(
        f.owner
            .assignment_request(run_id, request.assignment_id)
            .await
            .unwrap()
            .unwrap()
            .task,
        request.task
    );
    assert!(
        f.owner
            .assignment_request(Uuid::new_v4(), request.assignment_id)
            .await
            .is_err()
    );
    assert!(
        f.owner
            .assignment_result(Uuid::new_v4(), request.assignment_id)
            .await
            .is_err()
    );
    assert!(
        f.owner
            .assignment_observations(Uuid::new_v4())
            .await
            .is_err()
    );
    assert_eq!(
        f.owner
            .assignment_observations(run_id)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        f.owner
            .assignment_result(run_id, request.assignment_id)
            .await
            .unwrap()
            .is_null()
    );
    assert!(
        f.owner
            .record_assignment(f.actor.principal_id, destination, assignment(&f, run_id).1)
            .await
            .is_err()
    );
    ended(&mut run).await;
}

#[tokio::test]
async fn assignment_terminal_result_never_establishes_cleanup_until_exact_observation_arrives() {
    let f = Fixture::new().await;
    let (_, mut run) = f.running().await;
    let id = run.record().await.unwrap().id;
    let (destination, request) = assignment(&f, id);
    f.owner
        .record_assignment(f.actor.principal_id, destination, request.clone())
        .await
        .unwrap();
    let mut event = observed(destination, &request, "completed", false);
    event.result = Some(json!({"text":"actual retained synthetic result"}));
    let pending = f.owner.update_assignment(event.clone()).await.unwrap();
    assert_eq!(pending["state"], "cleanup_unknown");
    assert_eq!(pending["cleanup_observed"], false);
    ended(&mut run).await;
    run.confirm_local_cleanup_observed().await.unwrap();
    assert!(!f.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
    event.cleanup_observed = true;
    let settled = f.owner.update_assignment(event.clone()).await.unwrap();
    assert_eq!(settled["state"], "completed");
    assert!(f.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
    assert_eq!(
        f.owner
            .assignment_result(id, request.assignment_id)
            .await
            .unwrap()["result"],
        event.result.clone().unwrap()
    );
    let mut changed = event;
    changed.cleanup_observed = false;
    assert!(f.owner.update_assignment(changed).await.is_err());
}

#[tokio::test]
async fn assignment_observation_refuses_forged_child_attribution_state_result_and_cleanup_regression()
 {
    let f = Fixture::new().await;
    let (_, mut run) = f.running().await;
    let id = run.record().await.unwrap().id;
    let (destination, request) = assignment(&f, id);
    f.owner
        .record_assignment(f.actor.principal_id, destination, request.clone())
        .await
        .unwrap();
    let first = observed(destination, &request, "running", false);
    f.owner.update_assignment(first.clone()).await.unwrap();
    for case in 0..5 {
        let mut bad = first.clone();
        match case {
            0 => bad.child_session_id = Uuid::new_v4(),
            1 => bad.parent_run_id = Uuid::new_v4(),
            2 => bad.participant_vessel_id = Uuid::new_v4(),
            3 => bad.child_incarnation = Some(Uuid::new_v4()),
            _ => bad.cleanup_observed = true,
        };
        assert!(f.owner.update_assignment(bad).await.is_err());
    }
    let mut completed = first;
    completed.state = "completed".into();
    completed.cleanup_observed = true;
    completed.result = Some(json!({"text":"exact final result"}));
    f.owner.update_assignment(completed.clone()).await.unwrap();
    let mut bad = completed.clone();
    bad.state = "failed".into();
    assert!(f.owner.update_assignment(bad).await.is_err());
    let mut bad = completed;
    bad.result = Some(json!({"text":"changed"}));
    assert!(f.owner.update_assignment(bad).await.is_err());
    ended(&mut run).await;
    run.confirm_local_cleanup_observed().await.unwrap();
}

#[tokio::test]
async fn permanent_assignment_nonadmission_closes_only_the_exact_uncertain_child() {
    let f = Fixture::new().await;
    let (_, mut run) = f.running().await;
    let id = run.record().await.unwrap().id;
    let (destination, request) = assignment(&f, id);
    f.owner
        .record_assignment(f.actor.principal_id, destination, request.clone())
        .await
        .unwrap();
    let mut closed = observed(destination, &request, "cancelled", true);
    closed.run_id = None;
    closed.child_incarnation = None;
    closed.admission_closed = true;
    let first = f.owner.update_assignment(closed.clone()).await.unwrap();
    assert_eq!(first["state"], "cancelled");
    let mut regression = closed.clone();
    regression.admission_closed = false;
    assert!(f.owner.update_assignment(regression).await.is_err());
    let next = assignment(&f, id);
    f.owner
        .record_assignment(f.actor.principal_id, next.0, next.1)
        .await
        .unwrap();
    ended(&mut run).await;
}

#[tokio::test]
async fn consent_selected_and_custom_answers_are_exact_to_current_decision_and_do_not_execute_work()
{
    for custom in [false, true] {
        let f = Fixture::new().await;
        let (_, mut run) = f.running().await;
        let id = run.record().await.unwrap().id;
        let decision = Uuid::new_v4();
        f.owner.create_decision(id,f.incarnation,decision,expiry() as i64,json!({"kind":"question","question":{"question":"Choose owned fixture outcome","options":["one","two"]}})).await.unwrap();
        let revision = f.owner.snapshot().await.unwrap().revision;
        let response = if custom {
            json!({"status":"custom","answer":"actual custom fixture answer"})
        } else {
            json!({"status":"selected","index":1,"answer":"two"})
        };
        let command = RuntimeCommand::Respond {
            command_id: Uuid::new_v4(),
            expected_revision: revision,
            expires_at_ms: expiry(),
            run_id: id,
            decision_id: decision,
            response: response.clone(),
        };
        let first = f
            .owner
            .respond_decision(f.incarnation, command.clone(), || Ok(()))
            .await
            .unwrap();
        assert_eq!(
            f.owner
                .respond_decision(f.incarnation, command, || Ok(()))
                .await
                .unwrap(),
            first
        );
        assert_eq!(
            f.owner.decision_response(decision).await.unwrap(),
            Some(response)
        );
        assert!(
            f.owner
                .decisions(f.incarnation)
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .is_empty()
        );
        ended(&mut run).await;
        run.confirm_local_cleanup_observed().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_and_expired_consent_never_commits_an_answer_or_successful_receipt() {
    let f = Fixture::new().await;
    let (_, mut run) = f.running().await;
    let id = run.record().await.unwrap().id;
    let decision = Uuid::new_v4();
    f.owner
        .create_decision(
            id,
            f.incarnation,
            decision,
            expiry() as i64,
            json!({"kind":"question","question":{"options":["one","two"]}}),
        )
        .await
        .unwrap();
    for response in [
        json!({"status":"selected","index":1,"answer":"one"}),
        json!({"status":"custom","answer":""}),
        json!({"status":"custom","answer":"x\u{1b}"}),
        json!({"unknown":true}),
    ] {
        let command = RuntimeCommand::Respond {
            command_id: Uuid::new_v4(),
            expected_revision: f.owner.snapshot().await.unwrap().revision,
            expires_at_ms: expiry(),
            run_id: id,
            decision_id: decision,
            response,
        };
        let request = command.mutation_id().unwrap();
        assert!(
            f.owner
                .respond_decision(f.incarnation, command, || Ok(()))
                .await
                .is_err()
        );
        assert!(f.owner.process_receipt(request).await.unwrap().is_none());
        assert!(f.owner.decision_response(decision).await.unwrap().is_none());
    }
    let expired = Uuid::new_v4();
    f.owner
        .create_decision(id, f.incarnation, expired, 0, json!({"kind":"approval"}))
        .await
        .unwrap();
    assert_eq!(
        f.owner.decision_response(expired).await.unwrap(),
        Some(json!("expired"))
    );
    for outcome in ["invalidated", "cancelled", "expired"] {
        let id = Uuid::new_v4();
        f.owner
            .create_decision(
                run.run_id,
                f.incarnation,
                id,
                expiry() as i64,
                json!({"kind":"approval"}),
            )
            .await
            .unwrap();
        f.owner.finish_decision(id, outcome).await.unwrap();
        assert_eq!(
            f.owner.decision_response(id).await.unwrap(),
            Some(json!(outcome))
        );
    }
    assert!(f.owner.finish_decision(decision, "approved").await.is_err());
    ended(&mut run).await;
    run.confirm_local_cleanup_observed().await.unwrap();
}

#[tokio::test]
async fn steering_registration_is_once_per_unstarted_turn_and_rechecks_actor_and_time() {
    let f = Fixture::new().await;
    let (_, mut run) = f.admit().await;
    let permitted = Arc::new(AtomicBool::new(true));
    let allow = permitted.clone();
    let handle = run
        .enable_steering_with_clock(
            Arc::new(move |_: SteeringActor, _, _| {
                anyhow::ensure!(allow.load(Ordering::SeqCst), "steering revoked");
                Ok(())
            }),
            Arc::new(|| Ok(now())),
        )
        .unwrap();
    assert!(run.enable_steering(Arc::new(|_, _, _| Ok(()))).is_err());
    let make = || SteeringAdmission {
        parts: vec![],
        coordination: None,
        receipt_id: Uuid::new_v4(),
        session_id: f.owner.session_id(),
        run_id: run.run_id,
        actor: SteeringActor {
            machine_id: f.actor.installation_id,
            principal_id: f.actor.principal_id,
        },
        expected_revision: 1,
        expires_at_ms: now() + 60_000,
        text: "exact steering".into(),
    };
    let request = make();
    let first = handle.submit(request.clone()).await.unwrap();
    assert!(!first.duplicate);
    assert!(handle.submit(request).await.unwrap().duplicate);
    permitted.store(false, Ordering::SeqCst);
    assert!(handle.submit(make()).await.is_err());
    drop(handle);
    run.fail_before_execution().await.unwrap();
}

#[tokio::test]
async fn closing_steering_receiver_keeps_a_durable_refusal_and_never_delivers_after_terminal() {
    let f = Fixture::new().await;
    let (_, mut run) = f.admit().await;
    let handle = run.enable_steering(Arc::new(|_, _, _| Ok(()))).unwrap();
    run.steering_receiver.take();
    let request = SteeringAdmission {
        parts: vec![],
        coordination: None,
        receipt_id: Uuid::new_v4(),
        session_id: f.owner.session_id(),
        run_id: run.run_id,
        actor: SteeringActor {
            machine_id: f.actor.installation_id,
            principal_id: f.actor.principal_id,
        },
        expected_revision: 1,
        expires_at_ms: now() + 60_000,
        text: "closed receiver fixture".into(),
    };
    let refused = handle.submit(request.clone()).await.unwrap();
    assert_eq!(
        refused.record.status,
        crate::model::SteeringStatus::NotApplied
    );
    assert_eq!(
        refused.record.reason,
        Some(crate::attachment::journal::SteeringRejection::Closed)
    );
    assert!(handle.submit(request).await.unwrap().duplicate);
    drop(handle);
    run.fail_before_execution().await.unwrap();
}

#[tokio::test]
async fn projections_are_read_only_and_keep_streamed_text_separate_from_canonical_history() {
    let f = Fixture::new().await;
    let (_, run) = f.running().await;
    let checkpoint = run.checkpoint();
    let before = f.owner.snapshot().await.unwrap();
    checkpoint
        .partial("streamed owned progress 世界")
        .await
        .unwrap();
    let canonical = f.owner.process_history(0, 100, None).await.unwrap();
    assert_eq!(
        canonical["messages"].as_array().unwrap().len(),
        before.session.messages.len()
    );
    let public = f.owner.observations(0, 128).await.unwrap();
    let live = f.owner.live_observations(0, 128).await.unwrap();
    assert_eq!(public["projection"], "public-v1");
    assert_eq!(live["projection"], "public-v2");
    assert!(live.to_string().contains("streamed owned progress"));
    assert!(!public.to_string().contains("streamed owned progress"));
    for limit in [0, 129] {
        assert!(f.owner.observations(0, limit).await.is_err());
        assert!(f.owner.live_observations(0, limit).await.is_err());
    }
    assert_eq!(
        f.owner.snapshot().await.unwrap().session.messages.len(),
        before.session.messages.len()
    );
}

#[tokio::test]
async fn cleanup_requests_are_metadata_until_observation_and_cannot_relabel_a_live_turn_as_retired()
{
    let f = Fixture::new().await;
    let (_, run) = f.admit().await;
    run.register_local_cleanup().await.unwrap();
    let id = run.run_id;
    assert!(!f.owner.local_cancel_requested(id).await.unwrap());
    f.owner
        .process_metadata(
            f.actor,
            RuntimeCommand::Cancel {
                command_id: Uuid::new_v4(),
                expected_revision: f.owner.snapshot().await.unwrap().revision,
                expires_at_ms: expiry(),
                run_id: id,
            },
        )
        .await
        .unwrap();
    assert!(f.owner.local_cancel_requested(id).await.unwrap());
    assert!(run.checkpoint().local_cancel_requested().await.unwrap());
    assert!(f.owner.attest_local_cleanup(id, f.actor).await.is_err());
    assert!(!f.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
    drop(run);
    f.owner.recover_interrupted().await.unwrap();
    f.owner.attest_local_cleanup(id, f.actor).await.unwrap();
    assert!(f.owner.process_snapshot().await.unwrap()["pending_cleanup_run"].is_null());
    assert_eq!(
        f.owner.process_snapshot().await.unwrap()["run"]["state"],
        "interrupted"
    );
}
