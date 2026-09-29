use super::*;
use crate::provider::goal_meter::RequestObservation;

fn fixture() -> (
    tempfile::TempDir,
    Journal,
    Session,
    ExecutionGuard,
    GoalAuthority,
) {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path().join("journal")).unwrap();
    let session = Session::new(root.path().canonicalize().unwrap(), "fixture".into());
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    journal.initialize_process_commands(&guard).unwrap();
    journal.initialize_lifecycle(&guard).unwrap();
    journal.initialize_decisions(&guard).unwrap();
    journal.initialize_assignments(&guard).unwrap();
    journal.initialize_observations(&guard).unwrap();
    let authority = GoalAuthority {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        grant: None,
    };
    journal
        .initialize_command_bindings(&guard, authority.principal_id)
        .unwrap();
    (root, journal, session, guard, authority)
}

fn command(j: &Journal, session: Uuid, action: GoalAction) -> RuntimeCommand {
    RuntimeCommand::GoalUpdate {
        command_id: Uuid::new_v4(),
        expected_revision: j.load_session(session).unwrap().revision,
        expires_at_ms: 61_000,
        action,
    }
}

fn set(replace_goal_id: Option<Uuid>, active: bool) -> GoalAction {
    GoalAction::Set {
        objective: "Verify the migration and publish the result".into(),
        limits: GoalLimits::default(),
        replace_goal_id,
        continue_automatically: active,
    }
}

#[test]
fn mutations_are_durable_exact_and_actor_bound() {
    let (root, mut j, session, guard, authority) = fixture();
    let command = command(&j, session.id, set(None, true));
    let receipt = j
        .update_goal(&guard, authority.clone(), &command, 1000)
        .unwrap();
    let before = j.goal(session.id).unwrap();
    assert_eq!(before.revision, 1);
    assert_eq!(before.goal.as_ref().unwrap().status, GoalStatus::Active);
    assert_eq!(
        receipt,
        j.update_goal(&guard, authority.clone(), &command, 999_999)
            .unwrap()
    );
    let mut foreign = authority.clone();
    foreign.principal_id = Uuid::new_v4();
    assert!(j.update_goal(&guard, foreign, &command, 1000).is_err());
    let mut changed = command.clone();
    if let RuntimeCommand::GoalUpdate { action, .. } = &mut changed {
        *action = set(None, false);
    }
    assert!(j.update_goal(&guard, authority, &changed, 1000).is_err());
    assert_eq!(j.goal(session.id).unwrap(), before);
    drop(guard);
    drop(j);
    let j = Journal::open(root.path().join("journal")).unwrap();
    assert_eq!(j.goal(session.id).unwrap(), before);
    assert_eq!(
        j.process_receipt(command.mutation_id().unwrap())
            .unwrap()
            .unwrap(),
        receipt
    );
}

#[test]
fn replacement_clear_and_stale_commands_cannot_overwrite_new_work() {
    let (_root, mut j, session, guard, a) = fixture();
    let first = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &first, 1000).unwrap();
    let id = j.goal(session.id).unwrap().goal.unwrap().id;
    for replace in [None, Some(Uuid::new_v4())] {
        let c = command(&j, session.id, set(replace, true));
        assert!(j.update_goal(&guard, a.clone(), &c, 1000).is_err());
    }
    let stale = command(&j, session.id, GoalAction::Clear { goal_id: id });
    let replacement = command(&j, session.id, set(Some(id), false));
    j.update_goal(&guard, a.clone(), &replacement, 1000)
        .unwrap();
    assert!(j.update_goal(&guard, a.clone(), &stale, 1000).is_err());
    let new = j.goal(session.id).unwrap().goal.unwrap().id;
    let wrong = command(&j, session.id, GoalAction::Clear { goal_id: id });
    assert!(j.update_goal(&guard, a.clone(), &wrong, 1000).is_err());
    let clear = command(&j, session.id, GoalAction::Clear { goal_id: new });
    j.update_goal(&guard, a.clone(), &clear, 1000).unwrap();
    assert_eq!(
        j.goal(session.id).unwrap(),
        GoalSnapshot {
            revision: 3,
            goal: None
        }
    );
    let next = command(&j, session.id, set(None, false));
    j.update_goal(&guard, a, &next, 1000).unwrap();
    assert_eq!(j.goal(session.id).unwrap().revision, 4);
}

#[test]
fn observation_never_discloses_objective_or_private_continuation_authority() {
    let (_root, mut j, session, guard, mut a) = fixture();
    a.grant = Some(GrantBinding {
        grant_id: Uuid::new_v4(),
        revision: 1,
        principal_id: a.principal_id,
    });
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
    let page = j.live_observations(session.id, 0, 128).unwrap();
    let text = serde_json::to_string(&page).unwrap();
    assert!(
        page["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "goal" && e["payload"]["status"] == "active")
    );
    for secret in [
        "Verify the migration",
        &a.principal_id.to_string(),
        &a.installation_id.to_string(),
        &a.grant.unwrap().grant_id.to_string(),
    ] {
        assert!(!text.contains(secret));
    }
    assert!(
        !serde_json::to_string(&j.goal(session.id).unwrap())
            .unwrap()
            .contains("grant")
    );
}

#[test]
fn pause_survives_reopen_and_edit_does_not_resume_or_reset_usage() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
    let id = j.goal(session.id).unwrap().goal.unwrap().id;
    let reserved = j
        .reserve_goal_turn(
            &guard,
            j.goal(session.id).unwrap().revision,
            Uuid::new_v4(),
            "Continue".into(),
            1000,
        )
        .unwrap();
    j.abandon_goal_turn(
        &guard,
        reserved.command.mutation_id().unwrap(),
        GoalStopReason::Interrupted,
        1000,
    )
    .unwrap();
    let pause = command(&j, session.id, GoalAction::Pause { goal_id: id });
    j.update_goal(&guard, a.clone(), &pause, 1000).unwrap();
    let edit = command(
        &j,
        session.id,
        GoalAction::Edit {
            goal_id: id,
            objective: "Revised task".into(),
            limits: GoalLimits::default(),
        },
    );
    j.update_goal(&guard, a.clone(), &edit, 1000).unwrap();
    let g = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(g.status, GoalStatus::Paused);
    assert_eq!(g.usage.runs, 1);
    assert!(!g.continuation_authorized);
    let authority: Option<String> = j
        .connection
        .query_row(
            "SELECT authority FROM process_goals WHERE session_id=?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(authority.is_none());
    drop(guard);
    drop(j);
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    assert_eq!(j.goal(session.id).unwrap().goal.unwrap(), g);
    let resume = command(&j, session.id, GoalAction::Resume { goal_id: id });
    j.update_goal(&guard, a, &resume, 1000).unwrap();
    assert_eq!(
        j.goal(session.id).unwrap().goal.unwrap().status,
        GoalStatus::Active
    );
}

#[test]
fn bounds_reject_without_changing_canonical_state() {
    let (_root, mut j, session, guard, a) = fixture();
    for objective in [" ".into(), "a".repeat(8193), "hidden\u{1b}[31m".into()] {
        let c = command(
            &j,
            session.id,
            GoalAction::Set {
                objective,
                limits: GoalLimits::default(),
                replace_goal_id: None,
                continue_automatically: true,
            },
        );
        assert!(j.update_goal(&guard, a.clone(), &c, 1000).is_err());
    }
    let mut limits = GoalLimits::default();
    limits.runs = 0;
    let c = command(
        &j,
        session.id,
        GoalAction::Set {
            objective: "bounded".into(),
            limits,
            replace_goal_id: None,
            continue_automatically: true,
        },
    );
    assert!(j.update_goal(&guard, a, &c, 1000).is_err());
    assert_eq!(j.goal(session.id).unwrap(), GoalSnapshot::default());
    assert_eq!(j.load_session(session.id).unwrap().revision, 0);
}

#[test]
fn active_run_blocks_replacement_but_allows_pause() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
    let id = j.goal(session.id).unwrap().goal.unwrap().id;
    j.admit_turn(
        &guard,
        &TurnAdmission {
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: a.installation_id,
            principal_id: a.principal_id,
            session_id: session.id,
            expected_revision: j.load_session(session.id).unwrap().revision,
            expires_at_ms: 61_000,
            prompt: "work".into(),
            parts: vec![],
        },
        1000,
    )
    .unwrap();
    let c = command(&j, session.id, set(Some(id), true));
    assert!(j.update_goal(&guard, a.clone(), &c, 1000).is_err());
    let pause = command(&j, session.id, GoalAction::Pause { goal_id: id });
    j.update_goal(&guard, a, &pause, 1000).unwrap();
    assert!(
        !j.goal(session.id)
            .unwrap()
            .goal
            .unwrap()
            .continuation_authorized
    );
}

#[test]
fn schema_upgrade_fences_pre_goal_writers() {
    let (root, j, session, guard, a) = fixture();
    drop(guard);
    drop(j);
    let db = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE process_goals; UPDATE attachment_schema SET version=12;")
        .unwrap();
    drop(db);
    let stale = Journal::open(root.path().join("journal")).unwrap();
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    assert_eq!(j.goal(session.id).unwrap(), GoalSnapshot::default());
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    assert!(stale.check_schema().is_err());
    assert_eq!(j.opened_schema, SCHEMA_VERSION);
}

#[test]
fn settlement_upgrade_preserves_staged_goal_and_fences_old_writer() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let before = j.goal(session.id).unwrap();
    drop(guard);
    drop(j);
    let db = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.execute_batch(
        "DROP TABLE process_goal_settlements; UPDATE attachment_schema SET version=13;",
    )
    .unwrap();
    drop(db);
    let stale = Journal::open(root.path().join("journal")).unwrap();
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    j.upgrade_quiescent().unwrap();
    assert_eq!(j.goal(session.id).unwrap(), before);
    assert!(stale.check_schema().is_err());
    let guard = j.acquire_execution(session.id).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    j.settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap();
}

#[test]
fn partial_aggregate_retains_subordinate_lower_bound_without_claiming_complete_usage() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    let mut aggregate = measured().unwrap();
    aggregate.complete = false;
    j.settle_goal_run(&guard, run.id, Some(aggregate), true, 1200)
        .unwrap();
    let goal = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(
        (
            goal.usage.input_tokens,
            goal.usage.output_tokens,
            goal.usage.unmeasured_runs
        ),
        (20, 8, 1)
    );
    assert_eq!(goal.status, GoalStatus::NeedsAttention);
    assert_eq!(goal.stop_reason, Some(GoalStopReason::UsageUnknown));
}

#[test]
fn operator_attestation_is_not_observed_cleanup_for_automatic_continuation() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, Some("progress"), RunState::Completed);
    j.connection
        .execute(
            "INSERT INTO local_cleanup_obligations VALUES(?1,?2,?3,?4,'operator_attested')",
            params![
                run.id.to_string(),
                session.id.to_string(),
                run.machine_id.to_string(),
                run.principal_id.to_string()
            ],
        )
        .unwrap();
    j.settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap();
    let goal = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(goal.status, GoalStatus::NeedsAttention);
    assert_eq!(goal.stop_reason, Some(GoalStopReason::UnresolvedEffects));
    assert!(!goal.continuation_authorized);
}

#[test]
fn continuation_is_reserved_once_before_effects_and_pause_fences_delivery() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
    let before = j.goal(session.id).unwrap();
    let id = before.goal.unwrap().id;
    let reservation = j
        .reserve_goal_turn(
            &guard,
            before.revision,
            Uuid::new_v4(),
            "Continue the accepted goal".into(),
            1100,
        )
        .unwrap();
    let command_id = reservation.command.mutation_id().unwrap();
    assert_eq!(reservation.authority.principal_id, a.principal_id);
    let reserved = j.goal(session.id).unwrap();
    assert_eq!(reserved.goal.as_ref().unwrap().usage.runs, 1);
    assert!(
        j.reserve_goal_turn(
            &guard,
            reserved.revision,
            Uuid::new_v4(),
            "Second run".into(),
            1100
        )
        .is_err()
    );
    let replacement = command(&j, session.id, set(Some(id), true));
    assert!(
        j.update_goal(&guard, a.clone(), &replacement, 1100)
            .is_err()
    );
    let pause = command(&j, session.id, GoalAction::Pause { goal_id: id });
    j.update_goal(&guard, a, &pause, 1100).unwrap();
    j.abandon_goal_turn(&guard, command_id, GoalStopReason::Interrupted, 1200)
        .unwrap();
    assert_eq!(
        j.goal(session.id).unwrap().goal.unwrap().status,
        GoalStatus::Paused
    );
    let receipt = j.process_receipt(command_id).unwrap().unwrap();
    assert_eq!(receipt["status"], "not_admitted");
    // Recovery is idempotent; neither usage nor revisions change a second time.
    let before = j.goal(session.id).unwrap();
    j.abandon_goal_turn(&guard, command_id, GoalStopReason::Interrupted, 1300)
        .unwrap();
    assert_eq!(j.goal(session.id).unwrap(), before);
}

#[test]
fn interrupted_reservation_survives_reopen_without_replay_or_success() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let g = j.goal(session.id).unwrap();
    let reservation = j
        .reserve_goal_turn(&guard, g.revision, Uuid::new_v4(), "Continue".into(), 1100)
        .unwrap();
    let id = reservation.command.mutation_id().unwrap();
    drop(guard);
    drop(j);
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    assert!(
        j.reserve_goal_turn(
            &guard,
            j.goal(session.id).unwrap().revision,
            Uuid::new_v4(),
            "Replay".into(),
            1200
        )
        .is_err()
    );
    j.abandon_goal_turn(&guard, id, GoalStopReason::Interrupted, 1200)
        .unwrap();
    let g = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(g.status, GoalStatus::NeedsAttention);
    assert!(!g.continuation_authorized);
    assert_eq!(g.usage.runs, 1);
    assert!(j.process_latest_run(session.id).unwrap().is_none());
    assert_eq!(
        j.process_receipt(id).unwrap().unwrap()["status"],
        "not_admitted"
    );
}

fn start_reserved(j: &mut Journal, guard: &ExecutionGuard, now: i64) -> (RunRecord, Uuid) {
    let incarnation = Uuid::new_v4();
    let revision = j.goal(guard.session_id).unwrap().revision;
    let reserved = j
        .reserve_goal_turn(
            guard,
            revision,
            incarnation,
            "Continue the current goal".into(),
            now,
        )
        .unwrap();
    let RuntimeCommand::Submit {
        command_id,
        expected_revision,
        expires_at_ms,
        prompt,
        ..
    } = reserved.command
    else {
        panic!("expected submit")
    };
    let run = j
        .admit_turn(
            guard,
            &TurnAdmission {
                coordination: None,
                operator_name: None,
                command_id,
                machine_id: reserved.authority.installation_id,
                principal_id: reserved.authority.principal_id,
                session_id: guard.session_id,
                expected_revision,
                expires_at_ms: i64::try_from(expires_at_ms).unwrap(),
                prompt,
                parts: vec![],
            },
            now,
        )
        .unwrap()
        .run;
    j.mark_running(guard, run.id).unwrap();
    (run, incarnation)
}

fn finish_reserved(
    j: &mut Journal,
    guard: &ExecutionGuard,
    now: i64,
    tool: Option<&str>,
    state: RunState,
) -> RunRecord {
    let (run, _) = start_reserved(j, guard, now);
    let mut messages = j.load_session(guard.session_id).unwrap().session.messages;
    if let Some(result) = tool {
        let call = Uuid::new_v4().to_string();
        let mut request = Message::new(Role::Assistant, "");
        request.tool_calls.push(crate::model::ToolCall {
            id: call.clone(),
            name: "read_file".into(),
            arguments: json!({"path":"result.txt"}),
        });
        messages.push(request);
        messages.push(Message::tool_result(call, result, true));
    }
    j.checkpoint_canonical_at(
        guard,
        run.id,
        &messages,
        &Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
        now,
    )
    .unwrap();
    let text = (state == RunState::Completed).then_some("A run result, not proof of the full goal");
    let reason = (state != RunState::Completed).then_some("fixture terminal failure");
    j.finish(guard, run.id, state, reason, text).unwrap()
}

fn measured() -> Option<GoalMeasurement> {
    Some(GoalMeasurement {
        input_tokens: 20,
        output_tokens: 8,
        elapsed_ms: 100,
        complete: true,
    })
}

fn usage_request(
    j: &mut Journal,
    guard: &ExecutionGuard,
    command: Uuid,
    incarnation: Uuid,
    input: u64,
    output: u64,
    complete: bool,
) -> RequestObservation {
    let initial = RequestObservation {
        request_id: Uuid::new_v4(),
        revision: 0,
        input_tokens: None,
        output_tokens: None,
        complete: false,
    };
    j.record_goal_request(guard, command, incarnation, initial.clone())
        .unwrap();
    let final_state = RequestObservation {
        revision: 1,
        input_tokens: Some(input),
        output_tokens: Some(output),
        complete,
        ..initial
    };
    j.record_goal_request(guard, command, incarnation, final_state.clone())
        .unwrap();
    final_state
}

#[test]
fn durable_request_meter_fences_duplicates_reordering_identity_and_regression() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let (run, incarnation) = start_reserved(&mut j, &guard, 1100);
    assert!(
        j.begin_goal_meter(&guard, run.command_id, Uuid::new_v4(), 1100)
            .is_err()
    );
    assert_eq!(
        j.begin_goal_meter(&guard, run.command_id, incarnation, 1110)
            .unwrap(),
        Some((200_000, 3_599_990))
    );
    assert!(
        j.begin_goal_meter(&guard, run.command_id, incarnation, 1110)
            .is_err()
    );
    let observed = usage_request(&mut j, &guard, run.command_id, incarnation, 30, 7, false);
    j.record_goal_request(&guard, run.command_id, incarnation, observed.clone())
        .unwrap();
    for bad in [
        RequestObservation {
            revision: 0,
            ..observed.clone()
        },
        RequestObservation {
            revision: 3,
            ..observed.clone()
        },
        RequestObservation {
            revision: 2,
            input_tokens: Some(29),
            ..observed.clone()
        },
        RequestObservation {
            revision: 2,
            output_tokens: None,
            ..observed.clone()
        },
    ] {
        assert!(
            j.record_goal_request(&guard, run.command_id, incarnation, bad)
                .is_err()
        );
    }
    assert!(
        j.record_goal_request(&guard, run.command_id, Uuid::new_v4(), observed.clone())
            .is_err()
    );
    let completed = RequestObservation {
        revision: 2,
        complete: true,
        ..observed
    };
    j.record_goal_request(&guard, run.command_id, incarnation, completed.clone())
        .unwrap();
    assert!(
        j.record_goal_request(
            &guard,
            run.command_id,
            incarnation,
            RequestObservation {
                revision: 3,
                input_tokens: Some(31),
                ..completed
            }
        )
        .is_err()
    );
    assert_eq!(
        super::metering::retained_usage(&j.connection, run.command_id).unwrap(),
        (30, 7, true, true)
    );
}

#[test]
fn recovery_retains_durable_subordinate_usage_without_replaying_uncertain_requests() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let (run, incarnation) = start_reserved(&mut j, &guard, 1100);
    j.begin_goal_meter(&guard, run.command_id, incarnation, 1100)
        .unwrap();
    usage_request(&mut j, &guard, run.command_id, incarnation, 10, 5, true);
    usage_request(&mut j, &guard, run.command_id, incarnation, 30, 7, false);
    drop(guard);
    drop(j);
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    j.recover_interrupted(&guard).unwrap();
    j.recover_goal_turn(&guard, 1200).unwrap();
    let before = j.goal(session.id).unwrap();
    let g = before.goal.as_ref().unwrap();
    assert_eq!(
        (
            g.usage.input_tokens,
            g.usage.output_tokens,
            g.usage.unmeasured_runs
        ),
        (40, 12, 1)
    );
    assert_eq!(g.status, GoalStatus::NeedsAttention);
    assert!(!g.continuation_authorized);
    assert!(
        j.begin_goal_meter(&guard, run.command_id, Uuid::new_v4(), 1200)
            .is_err()
    );
    j.recover_goal_turn(&guard, 1300).unwrap();
    assert_eq!(j.goal(session.id).unwrap(), before);
    assert_eq!(
        j.connection
            .query_row("SELECT count(*) FROM process_goal_requests", [], |r| r
                .get::<_, u64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn open_durable_request_prevents_claiming_complete_aggregate_usage() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let (run, incarnation) = start_reserved(&mut j, &guard, 1100);
    j.begin_goal_meter(&guard, run.command_id, incarnation, 1100)
        .unwrap();
    let open = usage_request(&mut j, &guard, run.command_id, incarnation, 20, 8, false);
    j.finish(&guard, run.id, RunState::Completed, None, Some("fixture"))
        .unwrap();
    j.settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap();
    let goal = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(goal.usage.unmeasured_runs, 1);
    assert_eq!(goal.stop_reason, Some(GoalStopReason::UsageUnknown));
    assert!(
        j.record_goal_request(
            &guard,
            run.command_id,
            incarnation,
            RequestObservation {
                revision: 2,
                complete: true,
                ..open
            }
        )
        .is_err()
    );
    let tx = j.connection.transaction().unwrap();
    super::super::deletion::scrub(&tx, session.id).unwrap();
    tx.commit().unwrap();
    for table in [
        "process_goal_requests",
        "process_goal_meters",
        "process_goal_turns",
    ] {
        assert_eq!(
            j.connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn schema_fourteen_upgrade_adds_request_accounting_and_preserves_goal() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let before = j.goal(session.id).unwrap();
    drop(guard);
    drop(j);
    let db = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.execute_batch("DROP TABLE process_goal_requests; DROP TABLE process_goal_meters; UPDATE attachment_schema SET version=14;").unwrap();
    drop(db);
    let stale = Journal::open(root.path().join("journal")).unwrap();
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    j.upgrade_quiescent().unwrap();
    assert!(stale.check_schema().is_err());
    assert_eq!(j.goal(session.id).unwrap(), before);
    let guard = j.acquire_execution(session.id).unwrap();
    let (run, incarnation) = start_reserved(&mut j, &guard, 1100);
    j.begin_goal_meter(&guard, run.command_id, incarnation, 1100)
        .unwrap();
    usage_request(&mut j, &guard, run.command_id, incarnation, 1, 1, true);
}

#[test]
fn execution_budget_stops_are_reported_as_limits_instead_of_provider_failure() {
    for (limits, state, now, reason) in [
        (
            GoalLimits {
                tokens: 28,
                ..GoalLimits::default()
            },
            RunState::Failed,
            1200,
            GoalStopReason::TokenLimit,
        ),
        (
            GoalLimits {
                elapsed_ms: 1000,
                ..GoalLimits::default()
            },
            RunState::Cancelled,
            2100,
            GoalStopReason::TimeLimit,
        ),
    ] {
        let (_root, mut j, session, guard, a) = fixture();
        let c = command(
            &j,
            session.id,
            GoalAction::Set {
                objective: "bounded work".into(),
                limits,
                replace_goal_id: None,
                continue_automatically: true,
            },
        );
        j.update_goal(&guard, a, &c, 1000).unwrap();
        let run = finish_reserved(&mut j, &guard, 1100, None, state);
        j.settle_goal_run(&guard, run.id, measured(), true, now)
            .unwrap();
        let goal = j.goal(session.id).unwrap().goal.unwrap();
        assert_eq!(goal.status, GoalStatus::Limited);
        assert_eq!(goal.stop_reason, Some(reason));
        assert!(!goal.continuation_authorized);
    }
}

#[test]
fn terminal_settlement_is_once_only_and_run_completion_is_not_goal_completion() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    let receipt = j
        .settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap()
        .unwrap();
    let before = j.goal(session.id).unwrap();
    let g = before.goal.as_ref().unwrap();
    assert_eq!(g.status, GoalStatus::Active);
    assert_eq!(g.usage.input_tokens, 20);
    assert_eq!(g.usage.output_tokens, 8);
    assert_eq!(g.usage.elapsed_ms, 100);
    assert_eq!(g.usage.no_progress_runs, 1);
    assert_eq!(
        j.settle_goal_run(&guard, run.id, None, false, 1300)
            .unwrap()
            .unwrap(),
        receipt
    );
    assert_eq!(j.goal(session.id).unwrap(), before);
    drop(guard);
    drop(j);
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    assert_eq!(
        j.settle_goal_run(&guard, run.id, None, false, 1400)
            .unwrap()
            .unwrap(),
        receipt
    );
    assert_eq!(j.goal(session.id).unwrap(), before);
}

#[test]
fn missing_usage_is_visible_and_cannot_be_resumed_by_editing_limits() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    j.settle_goal_run(&guard, run.id, None, true, 1200).unwrap();
    let g = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(g.status, GoalStatus::NeedsAttention);
    assert_eq!(g.stop_reason, Some(GoalStopReason::UsageUnknown));
    assert_eq!(g.usage.unmeasured_runs, 1);
    assert_eq!(g.usage.input_tokens, 10);
    let edit = command(
        &j,
        session.id,
        GoalAction::Edit {
            goal_id: g.id,
            objective: g.objective.clone(),
            limits: GoalLimits::default(),
        },
    );
    j.update_goal(&guard, a.clone(), &edit, 1200).unwrap();
    let resume = command(&j, session.id, GoalAction::Resume { goal_id: g.id });
    assert!(j.update_goal(&guard, a, &resume, 1200).is_err());
}

#[test]
fn empty_runs_stop_at_no_progress_limit_without_claiming_success() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    for i in 0..3 {
        let run = finish_reserved(&mut j, &guard, 1100 + i * 100, None, RunState::Completed);
        j.settle_goal_run(&guard, run.id, measured(), true, 1200 + i * 100)
            .unwrap();
    }
    let g = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(g.status, GoalStatus::Limited);
    assert_eq!(g.stop_reason, Some(GoalStopReason::NoProgress));
    assert_eq!(g.usage.runs, 3);
    assert!(!g.continuation_authorized);
}

#[test]
fn identical_tool_results_with_fresh_call_ids_do_not_count_as_new_progress() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    for (i, result, expected) in [
        (0, "same bytes", 0),
        (1, "same bytes", 1),
        (2, "changed bytes", 0),
    ] {
        let run = finish_reserved(
            &mut j,
            &guard,
            1100 + i * 100,
            Some(result),
            RunState::Completed,
        );
        j.settle_goal_run(&guard, run.id, measured(), true, 1200 + i * 100)
            .unwrap();
        assert_eq!(
            j.goal(session.id)
                .unwrap()
                .goal
                .unwrap()
                .usage
                .no_progress_runs,
            expected
        );
    }
}

#[test]
fn incomplete_or_cancelled_work_and_unknown_cleanup_never_continue() {
    for (state, cleanup, reason) in [
        (RunState::Cancelled, true, GoalStopReason::Cancelled),
        (RunState::Interrupted, true, GoalStopReason::Interrupted),
        (RunState::Failed, true, GoalStopReason::ProviderFailure),
        (RunState::Incomplete, true, GoalStopReason::ProviderFailure),
        (
            RunState::Completed,
            false,
            GoalStopReason::UnresolvedEffects,
        ),
    ] {
        let (_root, mut j, session, guard, a) = fixture();
        let c = command(&j, session.id, set(None, true));
        j.update_goal(&guard, a, &c, 1000).unwrap();
        let run = finish_reserved(&mut j, &guard, 1100, None, state);
        j.settle_goal_run(&guard, run.id, measured(), cleanup, 1200)
            .unwrap();
        let g = j.goal(session.id).unwrap().goal.unwrap();
        assert_eq!(g.status, GoalStatus::NeedsAttention);
        assert_eq!(g.stop_reason, Some(reason));
        assert!(!g.continuation_authorized);
    }
}

#[test]
fn pause_wins_over_automatic_settlement_and_usage_still_accumulates() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    let id = j.goal(session.id).unwrap().goal.unwrap().id;
    let pause = command(&j, session.id, GoalAction::Pause { goal_id: id });
    j.update_goal(&guard, a, &pause, 1150).unwrap();
    j.settle_goal_run(&guard, run.id, measured(), false, 1200)
        .unwrap();
    let g = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(g.status, GoalStatus::Paused);
    assert_eq!(g.stop_reason, Some(GoalStopReason::UserPaused));
    assert_eq!(g.usage.input_tokens, 20);
    assert!(!g.continuation_authorized);
}

#[test]
fn recovered_accepted_turn_accounts_lower_bound_once_and_never_replays() {
    let (root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    assert!(
        j.abandon_goal_turn(&guard, run.command_id, GoalStopReason::Interrupted, 1200)
            .is_err()
    );
    drop(guard);
    drop(j);
    let mut j = Journal::open(root.path().join("journal")).unwrap();
    let guard = j.acquire_execution(session.id).unwrap();
    j.recover_goal_turn(&guard, 1200).unwrap();
    let before = j.goal(session.id).unwrap();
    j.recover_goal_turn(&guard, 1300).unwrap();
    assert_eq!(j.goal(session.id).unwrap(), before);
    assert_eq!(before.goal.as_ref().unwrap().usage.unmeasured_runs, 1);
    assert!(!before.goal.unwrap().continuation_authorized);
    assert_eq!(
        j.process_latest_run(session.id).unwrap().unwrap().id,
        run.id
    );
    assert_eq!(
        j.process_receipt(run.command_id).unwrap().unwrap()["status"],
        "accepted"
    );
}

#[test]
fn aggregate_below_canonical_usage_rolls_back_and_corrected_settlement_can_finish() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, None, RunState::Completed);
    let before = j.goal(session.id).unwrap();
    assert!(
        j.settle_goal_run(
            &guard,
            run.id,
            Some(GoalMeasurement {
                input_tokens: 9,
                output_tokens: 5,
                elapsed_ms: 100,
                complete: true,
            }),
            true,
            1200
        )
        .is_err()
    );
    assert_eq!(j.goal(session.id).unwrap(), before);
    j.settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap();
}

#[test]
fn each_finite_budget_stops_new_work_and_remains_distinct_from_success() {
    for (limits, measurement, reason) in [
        (
            GoalLimits {
                runs: 1,
                ..GoalLimits::default()
            },
            measured(),
            GoalStopReason::RunLimit,
        ),
        (
            GoalLimits {
                tokens: 28,
                ..GoalLimits::default()
            },
            measured(),
            GoalStopReason::TokenLimit,
        ),
        (
            GoalLimits {
                elapsed_ms: 1000,
                ..GoalLimits::default()
            },
            Some(GoalMeasurement {
                input_tokens: 20,
                output_tokens: 8,
                elapsed_ms: 1000,
                complete: true,
            }),
            GoalStopReason::TimeLimit,
        ),
    ] {
        let (_root, mut j, session, guard, a) = fixture();
        let c = command(
            &j,
            session.id,
            GoalAction::Set {
                objective: "bounded task".into(),
                limits,
                replace_goal_id: None,
                continue_automatically: true,
            },
        );
        j.update_goal(&guard, a.clone(), &c, 1000).unwrap();
        let run = finish_reserved(&mut j, &guard, 1100, Some("progress"), RunState::Completed);
        j.settle_goal_run(&guard, run.id, measurement, true, 2100)
            .unwrap();
        let snapshot = j.goal(session.id).unwrap();
        let g = snapshot.goal.unwrap();
        assert_eq!(g.status, GoalStatus::Limited);
        assert_eq!(g.stop_reason, Some(reason));
        assert!(!g.continuation_authorized);
        assert!(
            j.reserve_goal_turn(
                &guard,
                snapshot.revision,
                Uuid::new_v4(),
                "More work".into(),
                2200
            )
            .is_err()
        );
        let resume = command(&j, session.id, GoalAction::Resume { goal_id: g.id });
        assert!(j.update_goal(&guard, a, &resume, 2200).is_err());
    }
}

#[test]
fn retained_unknown_effects_block_automatic_continuation() {
    let (_root, mut j, session, guard, a) = fixture();
    j.initialize_session_resources(&guard).unwrap();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(&mut j, &guard, 1100, Some("progress"), RunState::Completed);
    j.connection
        .execute(
            "INSERT INTO process_session_resources VALUES(?1,?2,?3,'fixture','retained_unknown')",
            params![
                Uuid::new_v4().to_string(),
                session.id.to_string(),
                run.id.to_string()
            ],
        )
        .unwrap();
    j.settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap();
    let g = j.goal(session.id).unwrap().goal.unwrap();
    assert_eq!(g.stop_reason, Some(GoalStopReason::UnresolvedEffects));
    assert!(!g.continuation_authorized);
}

#[test]
fn deleting_session_scrubs_goal_reservations_and_settlements_in_foreign_key_order() {
    let (_root, mut j, session, guard, a) = fixture();
    let c = command(&j, session.id, set(None, true));
    j.update_goal(&guard, a, &c, 1000).unwrap();
    let run = finish_reserved(
        &mut j,
        &guard,
        1100,
        Some("private result"),
        RunState::Completed,
    );
    j.settle_goal_run(&guard, run.id, measured(), true, 1200)
        .unwrap();
    let tx = j.connection.transaction().unwrap();
    super::super::deletion::scrub(&tx, session.id).unwrap();
    tx.commit().unwrap();
    for table in [
        "process_goals",
        "process_goal_turns",
        "process_goal_settlements",
    ] {
        let count: u64 = j
            .connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(
        j.process_receipt(c.mutation_id().unwrap())
            .unwrap()
            .unwrap()["status"],
        "deleted"
    );
}
