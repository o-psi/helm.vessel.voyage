//! Source-prepared two-phase atomicity/receipt assertions. The journal seam uses
//! opaque config strings; native helper account/policy/pipe checks are separate.
use super::*;
fn fixture() -> (tempfile::TempDir, Journal, Session, ExecutionGuard, Uuid) {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path().join("journal")).unwrap();
    let session = Session::new(root.path().canonicalize().unwrap(), "fixture".into());
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    journal
        .retain_initial_configuration(&guard, "{\"source\":true}".into())
        .unwrap();
    (root, journal, session, guard, Uuid::new_v4())
}
fn observe(
    journal: &mut Journal,
    guard: &ExecutionGuard,
    session: Uuid,
    inc: Uuid,
) -> RetiredJournalFacts {
    let request = TransitionRequest {
        schema: SCHEMA,
        session_id: session,
        source_incarnation: inc,
        operation: TransitionOperation::Observe {
            directory: journal.directory.parent().unwrap().into(),
        },
    };
    let TransitionResponse::Facts { facts } =
        journal.retired_transition(guard, &request, None).unwrap()
    else {
        panic!("facts expected")
    };
    facts
}
fn freeze(journal: &Journal, facts: RetiredJournalFacts) -> TransitionRequest {
    TransitionRequest {
        schema: SCHEMA,
        session_id: facts.session_id,
        source_incarnation: facts.source_incarnation,
        operation: TransitionOperation::SourceFreeze {
            directory: journal.directory.parent().unwrap().into(),
            command_id: Uuid::new_v4(),
            transition_id: Uuid::new_v4(),
            target_incarnation: Uuid::new_v4(),
            expected: facts,
            target_uid: unsafe { libc::geteuid() },
            target_gid: unsafe { libc::getegid() },
            target_config_digest: crate::identity_helper::config_digest(b"{\"target\":true}"),
            review_digest: "a".repeat(64),
        },
    }
}
fn prepared(
    journal: &mut Journal,
    guard: &ExecutionGuard,
    request: &TransitionRequest,
) -> PreparedTransitionReceipt {
    let TransitionResponse::Prepared { receipt } =
        journal.retired_transition(guard, request, None).unwrap()
    else {
        panic!("prepared expected")
    };
    receipt
}
fn commit_request(journal: &Journal, receipt: PreparedTransitionReceipt) -> TransitionRequest {
    TransitionRequest {
        schema: SCHEMA,
        session_id: receipt.session_id,
        source_incarnation: receipt.source_incarnation,
        operation: TransitionOperation::TargetCommit {
            directory: journal.directory.parent().unwrap().into(),
            command_id: Uuid::new_v4(),
            expected: receipt,
            target_config_path: "/fixture/private-target/launch.json".into(),
        },
    }
}
#[test]
fn source_freeze_never_receives_target_bytes_and_target_commit_preserves_canonical_history() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let before = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, before.clone());
    let receipt = prepared(&mut journal, &guard, &request);
    assert_eq!(
        settings(&journal.connection, session.id).unwrap(),
        "{\"source\":true}"
    );
    assert!(journal.initial_configuration(session.id).is_err());
    assert_eq!(receipt.previous_revision, before.revision);
    assert_eq!(receipt.prepared_revision, before.revision + 1);
    assert_eq!(receipt.history_digest, before.history_digest);
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(!encoded.contains("private-target"));
    assert!(!encoded.contains("target_launch_config"));
    let target = commit_request(&journal, receipt.clone());
    let TransitionResponse::Committed { receipt: completed } = journal
        .retired_transition(&guard, &target, Some("{\"target\":true}"))
        .unwrap()
    else {
        panic!("committed expected")
    };
    assert_eq!(completed.resulting_revision, receipt.prepared_revision + 1);
    assert_eq!(completed.history_digest, before.history_digest);
    assert_eq!(
        completed.pending_work_digest,
        receipt.retained_pending_work_digest
    );
    assert_eq!(
        journal
            .initial_configuration(session.id)
            .unwrap()
            .as_deref(),
        Some("{\"target\":true}")
    );
    let fresh = observe(&mut journal, &guard, session.id, inc);
    assert_eq!(fresh.history_digest, before.history_digest);
    assert_eq!(fresh.frozen_config_digest, receipt.target_config_digest);
}
#[test]
fn interrupted_old_turn_keeps_command_identity_and_never_becomes_completed() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let admission = TurnAdmission {
        budget: None,
        coordination: None,
        operator_name: None,
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 61000,
        prompt: "retained input".into(),
        parts: vec![],
    };
    let run = journal.admit_turn(&guard, &admission, 1000).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let before = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, before.clone());
    let receipt = prepared(&mut journal, &guard, &request);
    let retained = journal.run(run.id).unwrap();
    assert_eq!(retained.state, RunState::Interrupted);
    assert_eq!(retained.command_id, admission.command_id);
    assert!(!retained.final_checkpointed);
    assert!(receipt.unresolved_cleanup.contains(&run.id));
    assert_ne!(
        receipt.pending_work_digest,
        receipt.retained_pending_work_digest
    );
    assert_eq!(
        observe(&mut journal, &guard, session.id, inc).history_digest,
        before.history_digest
    );
    assert_eq!(
        journal.lookup_command(&admission).unwrap().unwrap().id,
        run.id
    );
}
#[test]
fn lost_output_lookup_and_identical_retries_do_not_advance_revision_again() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let receipt = prepared(&mut journal, &guard, &request);
    assert_eq!(prepared(&mut journal, &guard, &request), receipt);
    let target = commit_request(&journal, receipt.clone());
    let completed = journal
        .retired_transition(&guard, &target, Some("{\"target\":true}"))
        .unwrap();
    let repeated = journal
        .retired_transition(&guard, &target, Some("{\"target\":true}"))
        .unwrap();
    assert_eq!(completed, repeated);
    let lookup = TransitionRequest {
        schema: SCHEMA,
        session_id: session.id,
        source_incarnation: inc,
        operation: TransitionOperation::Lookup {
            directory: journal.directory.parent().unwrap().into(),
            command_id: receipt.command_id,
        },
    };
    assert_eq!(
        journal.retired_transition(&guard, &lookup, None).unwrap(),
        completed
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        receipt.prepared_revision + 1
    );
    let mut conflict = target;
    let TransitionOperation::TargetCommit { command_id, .. } = &mut conflict.operation else {
        unreachable!()
    };
    *command_id = Uuid::new_v4();
    assert!(
        journal
            .retired_transition(&guard, &conflict, Some("{\"target\":true}"))
            .is_err()
    );
}
#[test]
fn changed_facts_or_target_body_roll_back_without_publishing_a_receipt() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let mut facts = observe(&mut journal, &guard, session.id, inc);
    facts.history_digest = "b".repeat(64);
    let wrong = freeze(&journal, facts);
    assert!(journal.retired_transition(&guard, &wrong, None).is_err());
    assert!(!table(&journal.connection, TABLE).unwrap());
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let receipt = prepared(&mut journal, &guard, &request);
    let target = commit_request(&journal, receipt.clone());
    assert!(
        journal
            .retired_transition(&guard, &target, Some("{\"unreviewed\":true}"))
            .is_err()
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        receipt.prepared_revision
    );
    assert_eq!(
        settings(&journal.connection, session.id).unwrap(),
        "{\"source\":true}"
    );
    assert!(journal.initial_configuration(session.id).is_err());
}

#[test]
fn source_freeze_withdraws_goal_continuation_without_certifying_completion() {
    let (_root, mut journal, session, guard, inc) = fixture();
    journal.initialize_process_commands(&guard).unwrap();
    journal.initialize_lifecycle(&guard).unwrap();
    journal.initialize_decisions(&guard).unwrap();
    journal.initialize_assignments(&guard).unwrap();
    journal.initialize_observations(&guard).unwrap();
    let actor = GoalAuthority {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        grant: None,
    };
    journal
        .initialize_command_bindings(&guard, actor.principal_id)
        .unwrap();
    let set = voyage_protocol::process::RuntimeCommand::GoalUpdate {
        command_id: Uuid::new_v4(),
        expected_revision: journal.load_session(session.id).unwrap().revision,
        expires_at_ms: 61000,
        action: voyage_protocol::goals::GoalAction::Set {
            objective: "Retained fixture objective".into(),
            limits: Default::default(),
            replace_goal_id: None,
            continue_automatically: true,
        },
    };
    journal.update_goal(&guard, actor, &set, 1000).unwrap();
    let original = journal.goal(session.id).unwrap();
    assert_eq!(
        original.goal.as_ref().unwrap().status,
        voyage_protocol::goals::GoalStatus::Active
    );
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    prepared(&mut journal, &guard, &request);
    let goal = journal.goal(session.id).unwrap();
    let current = goal.goal.unwrap();
    assert_eq!(current.id, original.goal.unwrap().id);
    assert_eq!(
        current.status,
        voyage_protocol::goals::GoalStatus::NeedsAttention
    );
    assert!(!current.continuation_authorized);
    assert_eq!(
        current.stop_reason,
        Some(voyage_protocol::goals::GoalStopReason::UnresolvedEffects)
    );
    let authority: Option<String> = journal
        .connection
        .query_row(
            "SELECT authority FROM process_goals WHERE session_id=?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(authority.is_none());
}
#[test]
fn altered_target_identity_or_pending_work_refuses_before_private_configuration_replacement() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let receipt = prepared(&mut journal, &guard, &request);
    let mut wrong = receipt.clone();
    wrong.target_uid = wrong.target_uid.wrapping_add(1);
    let target = commit_request(&journal, wrong);
    assert!(
        journal
            .retired_transition(&guard, &target, Some("{\"target\":true}"))
            .is_err()
    );
    journal.connection.execute_batch("CREATE TABLE unexpected_pending_effect(id TEXT PRIMARY KEY); INSERT INTO unexpected_pending_effect VALUES('uncertain');").unwrap();
    let target = commit_request(&journal, receipt.clone());
    assert!(
        journal
            .retired_transition(&guard, &target, Some("{\"target\":true}"))
            .is_err()
    );
    assert_eq!(
        settings(&journal.connection, session.id).unwrap(),
        "{\"source\":true}"
    );
    assert_eq!(
        journal.load_session(session.id).unwrap().revision,
        receipt.prepared_revision
    );
}
#[test]
fn forged_complete_receipt_cannot_enable_lookup_without_actual_committed_configuration() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let prepared = prepared(&mut journal, &guard, &request);
    let forged = TransitionReceipt {
        command_id: Uuid::new_v4(),
        prepared: prepared.clone(),
        resulting_revision: prepared.prepared_revision + 1,
        config_digest: prepared.target_config_digest.clone(),
        history_digest: prepared.history_digest.clone(),
        pending_work_digest: prepared.retained_pending_work_digest.clone(),
    };
    journal.connection.execute("UPDATE execution_transitions SET target_receipt=?1,commit_command_id=?2 WHERE command_id=?3",params![serde_json::to_string(&forged).unwrap(),forged.command_id.to_string(),prepared.command_id.to_string()]).unwrap();
    let lookup = TransitionRequest {
        schema: SCHEMA,
        session_id: session.id,
        source_incarnation: inc,
        operation: TransitionOperation::Lookup {
            directory: journal.directory.parent().unwrap().into(),
            command_id: prepared.command_id,
        },
    };
    assert!(journal.retired_transition(&guard, &lookup, None).is_err());
    assert_eq!(
        settings(&journal.connection, session.id).unwrap(),
        "{\"source\":true}"
    );
}

#[test]
fn pretransfer_source_abort_clears_startup_gate_without_unfreezing_prior_work() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let receipt = prepared(&mut journal, &guard, &request);
    let abort = TransitionRequest {
        schema: SCHEMA,
        session_id: session.id,
        source_incarnation: inc,
        operation: TransitionOperation::AbortSource {
            directory: journal.directory.parent().unwrap().into(),
            command_id: Uuid::new_v4(),
            expected: receipt.clone(),
        },
    };
    let first = journal.retired_transition(&guard, &abort, None).unwrap();
    let TransitionResponse::Aborted { receipt: aborted } = &first else {
        panic!("aborted expected")
    };
    assert_eq!(aborted.history_digest, receipt.history_digest);
    assert_eq!(
        aborted.pending_work_digest,
        receipt.retained_pending_work_digest
    );
    assert_eq!(aborted.resulting_revision, receipt.prepared_revision + 1);
    assert_eq!(
        journal
            .initial_configuration(session.id)
            .unwrap()
            .as_deref(),
        Some("{\"source\":true}")
    );
    assert_eq!(
        journal.retired_transition(&guard, &abort, None).unwrap(),
        first
    );
    let target = commit_request(&journal, receipt);
    assert!(
        journal
            .retired_transition(&guard, &target, Some("{\"target\":true}"))
            .is_err()
    );
}
#[test]
fn source_abort_refuses_replaced_directory_identity_and_committed_target() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let receipt = prepared(&mut journal, &guard, &request);
    let mut copied = receipt.clone();
    copied.source_directory_inode = copied.source_directory_inode.wrapping_add(1);
    let abort = TransitionRequest {
        schema: SCHEMA,
        session_id: session.id,
        source_incarnation: inc,
        operation: TransitionOperation::AbortSource {
            directory: journal.directory.parent().unwrap().into(),
            command_id: Uuid::new_v4(),
            expected: copied,
        },
    };
    assert!(journal.retired_transition(&guard, &abort, None).is_err());
    assert!(journal.initial_configuration(session.id).is_err());
    let target = commit_request(&journal, receipt.clone());
    journal
        .retired_transition(&guard, &target, Some("{\"target\":true}"))
        .unwrap();
    let abort = TransitionRequest {
        operation: TransitionOperation::AbortSource {
            directory: journal.directory.parent().unwrap().into(),
            command_id: Uuid::new_v4(),
            expected: receipt,
        },
        ..abort
    };
    assert!(journal.retired_transition(&guard, &abort, None).is_err());
}

#[test]
fn absent_handoff_lookup_is_read_only_and_corrupt_receipt_is_never_absent() {
    let (_root, mut journal, session, guard, inc) = fixture();
    let id = Uuid::new_v4();
    let lookup = TransitionRequest {
        schema: SCHEMA,
        session_id: session.id,
        source_incarnation: inc,
        operation: TransitionOperation::Lookup {
            directory: journal.directory.parent().unwrap().into(),
            command_id: id,
        },
    };
    let before = journal.load_session(session.id).unwrap().revision;
    assert!(
        matches!(journal.retired_transition(&guard, &lookup, None).unwrap(), TransitionResponse::Absent { command_id } if command_id==id)
    );
    assert!(!table(&journal.connection, TABLE).unwrap());
    assert_eq!(journal.load_session(session.id).unwrap().revision, before);
    let facts = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, facts);
    let receipt = prepared(&mut journal, &guard, &request);
    journal
        .connection
        .execute(
            "UPDATE execution_transitions SET prepared_receipt='invalid' WHERE command_id=?1",
            [receipt.command_id.to_string()],
        )
        .unwrap();
    let lookup = TransitionRequest {
        operation: TransitionOperation::Lookup {
            directory: journal.directory.parent().unwrap().into(),
            command_id: receipt.command_id,
        },
        ..lookup
    };
    assert!(journal.retired_transition(&guard, &lookup, None).is_err());
}

#[test]
fn source_freeze_and_abort_preserve_legacy_schema_and_interrupted_work() {
    let root = tempfile::tempdir().unwrap();
    let directory = prepare_directory(root.path().join("journal")).unwrap();
    drop(open_private_file(&directory.join("journal.sqlite3")).unwrap());
    let source = Connection::open(directory.join("journal.sqlite3")).unwrap();
    source.execute_batch("CREATE TABLE attachment_schema(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL); INSERT INTO attachment_schema VALUES(1,12); CREATE TABLE sessions(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, state TEXT NOT NULL,next_sequence INTEGER NOT NULL DEFAULT 1); CREATE TABLE runs(id TEXT PRIMARY KEY,session_id TEXT NOT NULL,record TEXT NOT NULL,active INTEGER NOT NULL); CREATE TABLE process_configuration(session_id TEXT PRIMARY KEY,settings TEXT NOT NULL);").unwrap();
    let session = Session::new(root.path().canonicalize().unwrap(), "legacy".into());
    let history = serde_json::to_string(&session).unwrap();
    source
        .execute(
            "INSERT INTO sessions(id,revision,state) VALUES(?1,0,?2)",
            params![session.id.to_string(), history],
        )
        .unwrap();
    source
        .execute(
            "INSERT INTO process_configuration VALUES(?1,?2)",
            params![session.id.to_string(), "{\"source\":true}"],
        )
        .unwrap();
    let run = RunRecord {
        source_incarnation: None,
        id: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        state: RunState::Running,
        partial_text: "retained partial".into(),
        tool_previews: Vec::new(),
        reasoning_previews: Vec::new(),
        terminal_reason: None,
        usage: Default::default(),
        final_checkpointed: false,
    };
    source
        .execute(
            "INSERT INTO runs VALUES(?1,?2,?3,1)",
            params![
                run.id.to_string(),
                session.id.to_string(),
                serde_json::to_string(&run).unwrap()
            ],
        )
        .unwrap();
    drop(source);
    let mut journal = Journal::open(directory).unwrap();
    assert_eq!(journal.opened_schema, 12);
    let guard = journal.acquire_execution(session.id).unwrap();
    let inc = Uuid::new_v4();
    let before = observe(&mut journal, &guard, session.id, inc);
    let request = freeze(&journal, before);
    let receipt = prepared(&mut journal, &guard, &request);
    let abort = TransitionRequest {
        schema: SCHEMA,
        session_id: session.id,
        source_incarnation: inc,
        operation: TransitionOperation::AbortSource {
            directory: root.path().to_owned(),
            command_id: Uuid::new_v4(),
            expected: receipt,
        },
    };
    assert!(matches!(
        journal.retired_transition(&guard, &abort, None).unwrap(),
        TransitionResponse::Aborted { .. }
    ));
    assert_eq!(
        journal
            .connection
            .query_row(
                "SELECT version FROM attachment_schema WHERE id=1",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        12
    );
    assert_eq!(
        journal
            .connection
            .query_row(
                "SELECT state FROM sessions WHERE id=?1",
                [session.id.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        history
    );
    assert_eq!(
        settings(&journal.connection, session.id).unwrap(),
        "{\"source\":true}"
    );
    let retained = journal.run(run.id).unwrap();
    assert_eq!(retained.state, RunState::Interrupted);
    assert_eq!(retained.partial_text, run.partial_text);
    assert!(!retained.final_checkpointed);
    assert_eq!(
        journal
            .connection
            .query_row(
                "SELECT active FROM runs WHERE id=?1",
                [run.id.to_string()],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        journal
            .initial_configuration(session.id)
            .unwrap()
            .as_deref(),
        Some("{\"source\":true}")
    );
}

#[test]
fn inherited_execution_guard_closure_keeps_parent_exclusion_and_rejects_wrong_inode() {
    let (_root, journal, session, guard, _inc) = fixture();
    let path = journal
        .directory
        .join(format!("{}.execution.lock", session.id));
    let independent = open_private_file(&path).unwrap();
    let inherited = journal
        .acquire_execution_pinned(session.id, guard.file.try_clone().unwrap())
        .unwrap();
    assert!(independent.try_lock().is_err());
    drop(inherited);
    assert!(independent.try_lock().is_err());
    let other = open_private_file(&journal.directory.join("other.execution.lock")).unwrap();
    assert!(journal.acquire_execution_pinned(session.id, other).is_err());
    assert!(independent.try_lock().is_err());
    drop(guard);
    independent.try_lock().unwrap();
}
