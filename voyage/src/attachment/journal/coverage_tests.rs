//! Second coverage batch: durable process operations and ownership handoffs.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::{RuntimeCommand, RuntimeInitialization};

#[path = "coverage_tests/catalogue_tests.rs"]
mod catalogue;
#[path = "coverage_tests/configuration_tests.rs"]
mod configuration;
#[path = "coverage_tests/controls_tests.rs"]
mod controls;
#[path = "coverage_tests/transfers_tests.rs"]
mod transfers;

fn fixture() -> (tempfile::TempDir, Journal, Session, ExecutionGuard) {
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
    journal.initialize_cleanup_progress(&guard).unwrap();
    journal
        .initialize_command_bindings(&guard, Uuid::new_v4())
        .unwrap();
    (root, journal, session, guard)
}

fn admit(journal: &mut Journal, guard: &ExecutionGuard) -> RunRecord {
    let request = TurnAdmission {
        coordination: None,
        operator_name: None,
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: guard.session_id,
        expected_revision: journal.load_session(guard.session_id).unwrap().revision,
        expires_at_ms: 61000,
        prompt: "retained input".into(),
        parts: vec![],
    };
    let admission = journal.admit_turn(guard, &request, 1000).unwrap();
    assert!(!admission.duplicate);
    let supervised: bool = journal
        .connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='process_commands')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    if supervised {
        journal
            .initialize_command_bindings(guard, admission.run.principal_id)
            .unwrap();
    }
    admission.run
}

fn revision(journal: &Journal, session: Uuid) -> u64 {
    journal.load_session(session).unwrap().revision
}

fn rename(expected_revision: u64, name: &str) -> RuntimeCommand {
    RuntimeCommand::Rename {
        command_id: Uuid::new_v4(),
        expected_revision,
        expires_at_ms: 61000,
        name: name.into(),
    }
}

fn actor() -> crate::attachment::local_actor::LocalActor {
    crate::attachment::local_actor::LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    }
}

#[test]
fn command_reservation_rejection_is_immutable_sanitized_and_durable() {
    let (root, mut journal, session, guard) = fixture();
    let principal = Uuid::new_v4();
    let command = rename(0, "reserved");
    let id = command.mutation_id().unwrap();
    journal
        .bind_process_command(&guard, id, principal, &command)
        .unwrap();
    journal
        .bind_process_command(&guard, id, principal, &command)
        .unwrap();
    assert!(
        journal
            .bind_process_command(&guard, id, Uuid::new_v4(), &command)
            .is_err()
    );
    assert!(
        journal
            .reject_unadmitted(&guard, id, Uuid::new_v4(), "foreign")
            .is_err()
    );
    let reason = format!("\n\t{}\r", "é".repeat(600));
    let receipt = journal
        .reject_unadmitted(&guard, id, principal, &reason)
        .unwrap()
        .unwrap();
    assert_eq!(receipt["status"], "rejected");
    assert_eq!(receipt["reason"].as_str().unwrap().chars().count(), 512);
    assert!(
        !receipt["reason"]
            .as_str()
            .unwrap()
            .chars()
            .any(char::is_control)
    );
    assert!(
        journal
            .reject_unadmitted(&guard, id, principal, "replacement")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        journal
            .resolve_process_command(&guard, id, principal, Some(&command))
            .unwrap(),
        receipt
    );
    assert_eq!(revision(&journal, session.id), 0);
    drop(journal);
    let reopened = Journal::open(root.path().join("journal")).unwrap();
    assert_eq!(reopened.process_receipt(id).unwrap().unwrap(), receipt);
}

#[test]
fn resolution_without_payload_permanently_reserves_id_and_actor() {
    let (_root, mut journal, session, guard) = fixture();
    let principal = Uuid::new_v4();
    let command = rename(0, "delayed");
    let id = command.mutation_id().unwrap();
    let receipt = journal
        .resolve_process_command(&guard, id, principal, None)
        .unwrap();
    assert_eq!(receipt["status"], "not_admitted");
    assert_eq!(
        journal
            .resolve_process_command(&guard, id, principal, None)
            .unwrap(),
        receipt
    );
    assert!(
        journal
            .resolve_process_command(&guard, id, Uuid::new_v4(), None)
            .is_err()
    );
    journal
        .bind_process_command(&guard, id, principal, &command)
        .unwrap();
    assert_eq!(journal.process_receipt(id).unwrap().unwrap(), receipt);
    assert!(
        journal
            .resolve_process_command(&guard, Uuid::nil(), principal, None)
            .is_err()
    );
    assert!(
        journal
            .resolve_process_command(&guard, Uuid::new_v4(), Uuid::nil(), None)
            .is_err()
    );
    assert!(
        journal
            .resolve_process_command(&guard, Uuid::new_v4(), principal, Some(&command))
            .is_err()
    );
    assert_eq!(revision(&journal, session.id), 0);
}

#[test]
fn session_resource_obligations_recover_and_require_matching_attestation_actor() {
    let (_root, mut journal, _session, guard) = fixture();
    journal.initialize_session_resources(&guard).unwrap();
    let run = admit(&mut journal, &guard);
    let id = Uuid::new_v4();
    journal
        .session_resource_adopt(&guard, id, run.id, "root_terminals")
        .unwrap();
    assert_eq!(
        journal.session_resources(guard.session_id).unwrap()[0]["state"],
        "owned"
    );
    let witness = actor();
    assert!(
        journal
            .attest_session_resource(&guard, id, witness)
            .is_err()
    );
    journal.initialize_session_resources(&guard).unwrap();
    assert_eq!(
        journal.session_resources(guard.session_id).unwrap()[0]["state"],
        "cleanup_unknown"
    );
    assert!(journal.session_resource_closed(&guard, id).is_err());
    journal
        .attest_session_resource(&guard, id, witness)
        .unwrap();
    journal
        .attest_session_resource(&guard, id, witness)
        .unwrap();
    assert!(
        journal
            .attest_session_resource(&guard, id, actor())
            .is_err()
    );
    assert_eq!(
        journal.session_resources(guard.session_id).unwrap(),
        json!([])
    );
    assert!(
        journal
            .session_resource_adopt(&guard, id, run.id, "root_terminals")
            .is_err()
    );
}

#[test]
fn session_resource_adoption_rejects_invalid_identity_and_preserves_closed_rows() {
    let (_root, mut journal, session, guard) = fixture();
    journal.initialize_session_resources(&guard).unwrap();
    let run = admit(&mut journal, &guard);
    for (id, kind) in [
        (Uuid::nil(), "terminals".to_owned()),
        (Uuid::new_v4(), String::new()),
        (Uuid::new_v4(), "x".repeat(65)),
    ] {
        assert!(
            journal
                .session_resource_adopt(&guard, id, run.id, &kind)
                .is_err()
        );
    }
    assert!(
        journal
            .session_resource_adopt(&guard, Uuid::new_v4(), Uuid::new_v4(), "terminals")
            .is_err()
    );
    assert_eq!(journal.session_resources(session.id).unwrap(), json!([]));
    let id = Uuid::new_v4();
    journal
        .session_resource_adopt(&guard, id, run.id, "terminals")
        .unwrap();
    assert!(
        journal
            .session_resource_closed(&guard, Uuid::new_v4())
            .is_err()
    );
    journal.session_resource_closed(&guard, id).unwrap();
    assert!(journal.session_resource_closed(&guard, id).is_err());
    journal.initialize_session_resources(&guard).unwrap();
    let resources = journal.session_resources(session.id).unwrap();
    assert_eq!(resources, json!([]));
    assert!(
        journal
            .session_resource_adopt(&guard, id, run.id, "terminals")
            .is_err()
    );
    assert!(journal.retain_interrupted_cleanup(&guard).is_err());
    assert_eq!(
        journal.retained_cleanup(session.id).unwrap()["resources"],
        json!([])
    );
}

#[test]
fn reservation_rejects_nil_actors_and_oversized_payload_without_consuming_id() {
    let (_root, mut journal, session, guard) = fixture();
    let principal = Uuid::new_v4();
    let id = Uuid::new_v4();
    let mut command = rename(0, &"x".repeat(128 * 1024));
    assert!(
        journal
            .bind_process_command(&guard, id, principal, &command)
            .is_err()
    );
    command = rename(0, "small");
    assert!(
        journal
            .bind_process_command(&guard, Uuid::nil(), principal, &command)
            .is_err()
    );
    assert!(
        journal
            .bind_process_command(&guard, id, Uuid::nil(), &command)
            .is_err()
    );
    journal
        .bind_process_command(&guard, id, principal, &command)
        .unwrap();
    assert!(journal.process_receipt(id).unwrap().is_none());
    let rejected = journal
        .reject_unadmitted(&guard, id, principal, "valid reservation survives")
        .unwrap()
        .unwrap();
    assert_eq!(rejected["reason"], "valid reservation survives");
    assert_eq!(revision(&journal, session.id), 0);
}

#[test]
fn reservation_and_receipt_reads_are_fenced_to_execution_owner() {
    let (_root, mut journal, session, guard) = fixture();
    let other = Session::new(session.workspace.clone(), "other".into());
    journal.create_session(&other).unwrap();
    let other_guard = journal.acquire_execution(other.id).unwrap();
    journal.initialize_process_commands(&other_guard).unwrap();
    journal
        .initialize_command_bindings(&other_guard, Uuid::new_v4())
        .unwrap();
    let principal = Uuid::new_v4();
    let command = rename(0, "owner binding");
    let id = command.mutation_id().unwrap();
    journal
        .bind_process_command(&guard, id, principal, &command)
        .unwrap();
    assert!(
        journal
            .bind_process_command(&other_guard, id, Uuid::new_v4(), &command)
            .is_err()
    );
    assert!(
        journal
            .resolve_process_command(&other_guard, id, Uuid::new_v4(), None)
            .is_err()
    );
    assert!(journal.process_receipt(id).unwrap().is_none());
    assert_eq!(revision(&journal, session.id), 0);
    assert_eq!(revision(&journal, other.id), 0);
}

mod durable_projection_batch {
    use super::*;
    use crate::context::WorkingContext;

    #[test]
    fn working_context_round_trip_preserves_canonical_messages() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        let before = j.load_session(s.id).unwrap();
        let mut context = j.load_working_context(&g, run.id).unwrap();
        context.generation = 12;
        j.save_working_context(&g, run.id, &context).unwrap();
        assert_eq!(j.load_working_context(&g, run.id).unwrap().generation, 12);
        let after = j.load_session(s.id).unwrap();
        assert_eq!(
            serde_json::to_value(after.session.messages).unwrap(),
            serde_json::to_value(before.session.messages).unwrap()
        );
        assert_eq!(after.revision, before.revision + 1);
    }
    #[test]
    fn repeated_working_context_records_each_checkpoint() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        let context = j.load_working_context(&g, run.id).unwrap();
        let before = revision(&j, s.id);
        j.save_working_context(&g, run.id, &context).unwrap();
        j.save_working_context(&g, run.id, &context).unwrap();
        assert_eq!(revision(&j, s.id), before + 2);
    }
    #[test]
    fn context_read_refuses_unknown_run() {
        let (_root, mut j, _, g) = fixture();
        assert!(j.load_working_context(&g, Uuid::new_v4()).is_err());
    }
    #[test]
    fn context_write_refuses_unknown_run_without_revision_change() {
        let (_root, mut j, s, g) = fixture();
        assert!(
            j.save_working_context(&g, Uuid::new_v4(), &WorkingContext::default())
                .is_err()
        );
        assert_eq!(revision(&j, s.id), 0);
    }
    #[test]
    fn context_read_refuses_finished_run() {
        let (_root, mut j, _, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        j.finish(
            &g,
            run.id,
            RunState::Cancelled,
            Some("fixture cancellation"),
            None,
        )
        .unwrap();
        assert!(j.load_working_context(&g, run.id).is_err());
    }
    #[test]
    fn context_write_refuses_finished_run() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        j.finish(
            &g,
            run.id,
            RunState::Cancelled,
            Some("fixture cancellation"),
            None,
        )
        .unwrap();
        let before = revision(&j, s.id);
        assert!(
            j.save_working_context(&g, run.id, &WorkingContext::default())
                .is_err()
        );
        assert_eq!(revision(&j, s.id), before);
    }
    #[test]
    fn invalid_context_range_rolls_back_projection() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        let before = revision(&j, s.id);
        let context: WorkingContext = serde_json::from_value(json!({
            "generation": 2, "entries": [{"start": 999, "fingerprints": ["bad"],
                "replacement": crate::model::Message::new(crate::model::Role::Assistant, "summary")}]
        })).unwrap();
        assert!(j.save_working_context(&g, run.id, &context).is_err());
        assert_eq!(revision(&j, s.id), before);
        assert_eq!(j.load_working_context(&g, run.id).unwrap().generation, 0);
    }
    fn object() -> crate::github::repository::Object {
        crate::github::repository::Object::parse("https://github.com/o-psi/voyage/issues/288")
            .unwrap()
    }
    fn reference(title: &str) -> crate::github::operator::Reference {
        crate::github::operator::Reference {
            object: object(),
            head: None,
            fetched_at: chrono::DateTime::from_timestamp(if title == "second" { 2 } else { 1 }, 0)
                .unwrap(),
        }
    }
    #[test]
    fn reference_add_and_replace_are_durable_not_duplicated() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        assert!(
            j.operator_reference(&g, run.id, Some(reference("first")), None)
                .unwrap()
        );
        assert!(
            j.operator_reference(&g, run.id, Some(reference("second")), None)
                .unwrap()
        );
        let refs = j.load_session(s.id).unwrap().session.github_references;
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].fetched_at.timestamp(), 2);
    }
    #[test]
    fn identical_reference_is_revision_noop() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        j.operator_reference(&g, run.id, Some(reference("same")), None)
            .unwrap();
        let before = revision(&j, s.id);
        assert!(
            !j.operator_reference(&g, run.id, Some(reference("same")), None)
                .unwrap()
        );
        assert_eq!(revision(&j, s.id), before);
    }
    #[test]
    fn forgetting_reference_is_idempotent() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        j.operator_reference(&g, run.id, Some(reference("remove")), None)
            .unwrap();
        assert!(
            j.operator_reference(&g, run.id, None, Some(object()))
                .unwrap()
        );
        let before = revision(&j, s.id);
        assert!(
            !j.operator_reference(&g, run.id, None, Some(object()))
                .unwrap()
        );
        assert_eq!(revision(&j, s.id), before);
        assert!(
            j.load_session(s.id)
                .unwrap()
                .session
                .github_references
                .is_empty()
        );
    }
    #[test]
    fn empty_reference_edit_is_refused() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        let before = revision(&j, s.id);
        assert!(j.operator_reference(&g, run.id, None, None).is_err());
        assert_eq!(revision(&j, s.id), before);
    }
    #[test]
    fn conflicting_reference_edit_is_refused() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        assert!(
            j.operator_reference(&g, run.id, Some(reference("conflict")), Some(object()))
                .is_err()
        );
        assert!(
            j.load_session(s.id)
                .unwrap()
                .session
                .github_references
                .is_empty()
        );
    }
    #[test]
    fn issue_reference_cannot_have_pull_request_head() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        let mut invalid = reference("invalid");
        invalid.head = Some("a".repeat(40));
        assert!(
            j.operator_reference(&g, run.id, Some(invalid), None)
                .is_err()
        );
        assert!(
            j.load_session(s.id)
                .unwrap()
                .session
                .github_references
                .is_empty()
        );
    }
    #[test]
    fn reference_edit_requires_active_run() {
        let (_root, mut j, s, g) = fixture();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        j.finish(
            &g,
            run.id,
            RunState::Cancelled,
            Some("fixture cancellation"),
            None,
        )
        .unwrap();
        assert!(
            j.operator_reference(&g, run.id, Some(reference("late")), None)
                .is_err()
        );
        assert!(
            j.load_session(s.id)
                .unwrap()
                .session
                .github_references
                .is_empty()
        );
    }
    #[test]
    fn reference_edit_unknown_run_is_atomic() {
        let (_root, mut j, s, g) = fixture();
        assert!(
            j.operator_reference(&g, Uuid::new_v4(), Some(reference("unknown")), None)
                .is_err()
        );
        assert_eq!(revision(&j, s.id), 0);
    }
    #[test]
    fn incarnation_binding_is_idempotent() {
        let (_root, mut j, s, mut g) = fixture();
        let id = Uuid::new_v4();
        j.bind_notification_incarnation(&mut g, id).unwrap();
        j.bind_notification_incarnation(&mut g, id).unwrap();
        assert_eq!(g.incarnation, Some(id));
        assert_eq!(revision(&j, s.id), 0);
    }
    #[test]
    fn incarnation_cannot_be_rebound() {
        let (_root, mut j, _, mut g) = fixture();
        let id = Uuid::new_v4();
        j.bind_notification_incarnation(&mut g, id).unwrap();
        assert!(
            j.bind_notification_incarnation(&mut g, Uuid::new_v4())
                .is_err()
        );
        assert_eq!(g.incarnation, Some(id));
    }
    #[test]
    fn nil_incarnation_does_not_bind_owner() {
        let (_root, mut j, _, mut g) = fixture();
        assert!(
            j.bind_notification_incarnation(&mut g, Uuid::nil())
                .is_err()
        );
        assert!(g.incarnation.is_none());
    }
    #[test]
    fn notifications_reject_zero_limit() {
        let (_root, mut j, s, _) = fixture();
        assert!(j.notification_events(s.id, 0, 0).is_err());
    }
    #[test]
    fn notifications_reject_oversized_limit() {
        let (_root, mut j, s, _) = fixture();
        assert!(j.notification_events(s.id, 0, 129).is_err());
    }
    #[test]
    fn notifications_reject_overflow_cursor() {
        let (_root, mut j, s, _) = fixture();
        assert!(j.notification_events(s.id, u64::MAX, 1).is_err());
    }
    #[test]
    fn notifications_reject_unknown_session() {
        let (_root, mut j, _, _) = fixture();
        assert!(j.notification_events(Uuid::new_v4(), 0, 1).is_err());
    }
    #[test]
    fn notifications_reject_future_cursor() {
        let (_root, mut j, s, _) = fixture();
        assert!(j.notification_events(s.id, 1, 128).is_err());
    }
    #[test]
    fn empty_notifications_have_stable_cursor() {
        let (_root, mut j, s, _) = fixture();
        let value = j.notification_events(s.id, 0, 128).unwrap();
        assert_eq!(value["events"], json!([]));
        assert_eq!(value["next_after"], 0);
        assert_eq!(value["has_more"], false);
    }
    #[test]
    fn attributed_admission_exports_original_incarnation() {
        let (_root, mut j, s, mut g) = fixture();
        let incarnation = Uuid::new_v4();
        j.bind_notification_incarnation(&mut g, incarnation)
            .unwrap();
        let run = admit(&mut j, &g);
        j.mark_running(&g, run.id).unwrap();
        j.finish(
            &g,
            run.id,
            RunState::Cancelled,
            Some("fixture cancellation"),
            None,
        )
        .unwrap();
        let events = j.notification_events(s.id, 0, 128).unwrap();
        let encoded = serde_json::to_string(&events).unwrap();
        assert!(encoded.contains(&incarnation.to_string()));
        assert!(encoded.contains(&run.id.to_string()));
        assert!(!events["events"].as_array().unwrap().is_empty());
    }
}
