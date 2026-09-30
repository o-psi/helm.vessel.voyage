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
