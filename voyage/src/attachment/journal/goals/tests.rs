use super::*;

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
    assert_eq!(j.opened_schema, 13);
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
