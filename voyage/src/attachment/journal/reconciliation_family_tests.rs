//! Explicit unknown-outcome recovery: never dispatch, success or rollback.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::RuntimeCommand;

struct Fixture {
    _root: tempfile::TempDir,
    journal: Journal,
    guard: ExecutionGuard,
    run: RunRecord,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut journal = Journal::open(root.path().join("journal")).unwrap();
        let session = Session::new(root.path().into(), "fixture".into());
        journal.create_session(&session).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.initialize_process_commands(&guard).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        journal.initialize_session_resources(&guard).unwrap();
        journal.initialize_cleanup_progress(&guard).unwrap();
        let request = TurnAdmission {
            budget: None,
            coordination: None,
            operator_name: None,
            command_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: 61_000,
            prompt: "Owned interrupted tool fixture".into(),
            parts: vec![],
        };
        let run = journal.admit_turn(&guard, &request, 1000).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        journal.register_local_cleanup(&guard, run.id).unwrap();
        Self {
            _root: root,
            journal,
            guard,
            run,
        }
    }
    fn calls(&mut self, ids: &[&str]) -> Vec<Message> {
        let mut messages = self
            .journal
            .load_session(self.guard.session_id)
            .unwrap()
            .session
            .messages;
        let mut call = Message::new(Role::Assistant, "");
        call.tool_calls = ids
            .iter()
            .map(|id| crate::model::ToolCall {
                id: (*id).into(),
                name: "owned-never-replayed".into(),
                arguments: json!({"fixture":true}),
            })
            .collect();
        messages.push(call);
        self.journal
            .checkpoint_canonical(&self.guard, self.run.id, &messages, &Usage::default())
            .unwrap();
        messages
    }
    fn end(&mut self, cleanup: bool) {
        self.journal
            .finish(
                &self.guard,
                self.run.id,
                RunState::Interrupted,
                Some("owned fixture interruption"),
                None,
            )
            .unwrap();
        if cleanup {
            self.journal
                .confirm_local_cleanup_observed(&self.guard, self.run.id)
                .unwrap();
        }
    }
    fn request(&self) -> LocalReconcileRequest {
        LocalReconcileRequest {
            session_id: self.guard.session_id,
            run_id: self.run.id,
            installation_id: self.run.machine_id,
            principal_id: self.run.principal_id,
            expected_revision: self
                .journal
                .load_session(self.guard.session_id)
                .unwrap()
                .revision,
        }
    }
    fn saved(&self) -> Value {
        serde_json::to_value(
            self.journal
                .load_session(self.guard.session_id)
                .unwrap()
                .session,
        )
        .unwrap()
    }
}

#[test]
fn explicit_reconciliation_appends_ordered_unknown_results_without_changing_prefix_or_terminal_state()
 {
    let mut f = Fixture::new();
    let prefix = f.calls(&["first", "second"]);
    f.end(true);
    let request = f.request();
    let first = f.journal.reconcile_local_tools(&f.guard, &request).unwrap();
    assert_eq!(first.tool_call_ids, vec!["first", "second"]);
    assert!(!first.duplicate);
    let saved = f.journal.load_session(f.guard.session_id).unwrap();
    assert_eq!(
        serde_json::to_value(&saved.session.messages[..prefix.len()]).unwrap(),
        serde_json::to_value(prefix).unwrap()
    );
    for message in &saved.session.messages[saved.session.messages.len() - 2..] {
        assert_eq!(message.role, Role::Tool);
        assert_eq!(message.tool_success, Some(false));
        assert!(message.content.contains("outcome is unknown"));
        assert_eq!(
            message.tool_outcome.as_ref().unwrap().execution,
            voyage_protocol::tool_result::ExecutionOutcome::Unknown
        );
    }
    assert_eq!(
        f.journal.run(f.run.id).unwrap().state,
        RunState::Interrupted
    );
    let before = f.saved();
    let duplicate = f.journal.reconcile_local_tools(&f.guard, &request).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.revision, first.revision);
    assert_eq!(f.saved(), before);
}

#[test]
fn reconciliation_requires_exact_actor_revision_latest_terminal_run_and_positive_cleanup() {
    let mut f = Fixture::new();
    f.calls(&["pending"]);
    let request = f.request();
    assert!(f.journal.reconcile_local_tools(&f.guard, &request).is_err());
    f.end(false);
    let request = f.request();
    assert!(f.journal.reconcile_local_tools(&f.guard, &request).is_err());
    f.journal
        .confirm_local_cleanup_observed(&f.guard, f.run.id)
        .unwrap();
    let request = f.request();
    let before = f.saved();
    for case in 0..4 {
        let mut wrong = request.clone();
        match case {
            0 => wrong.session_id = Uuid::new_v4(),
            1 => wrong.installation_id = Uuid::new_v4(),
            2 => wrong.principal_id = Uuid::new_v4(),
            _ => wrong.expected_revision += 1,
        };
        assert!(f.journal.reconcile_local_tools(&f.guard, &wrong).is_err());
        assert_eq!(f.saved(), before);
    }
    f.journal.reconcile_local_tools(&f.guard, &request).unwrap();
}

#[test]
fn exact_reconciliation_receipt_survives_later_metadata_and_actor_or_payload_collision_does_not() {
    let mut f = Fixture::new();
    f.calls(&["owned"]);
    f.end(true);
    let request = f.request();
    let first = f.journal.reconcile_local_tools(&f.guard, &request).unwrap();
    f.journal
        .process_metadata(
            &f.guard,
            crate::attachment::local_actor::LocalActor {
                installation_id: f.run.machine_id,
                principal_id: f.run.principal_id,
            },
            &RuntimeCommand::Rename {
                command_id: Uuid::new_v4(),
                expected_revision: first.revision,
                expires_at_ms: 61_000,
                name: "later metadata".into(),
            },
            1000,
        )
        .unwrap();
    assert!(
        f.journal
            .reconcile_local_tools(&f.guard, &request)
            .unwrap()
            .duplicate
    );
    let before = f.saved();
    let mut altered = request;
    altered.expected_revision += 1;
    assert!(f.journal.reconcile_local_tools(&f.guard, &altered).is_err());
    assert_eq!(f.saved(), before);
}

#[test]
fn stored_reconciliation_receipt_corruption_is_unavailable_not_absent_and_never_repaired() {
    for case in 0..6 {
        let mut f = Fixture::new();
        f.calls(&["owned"]);
        f.end(true);
        let request = f.request();
        f.journal.reconcile_local_tools(&f.guard, &request).unwrap();
        let before = f.saved();
        let original: String = f
            .journal
            .connection
            .query_row(
                "SELECT record FROM local_tool_reconciliations WHERE run_id=?1",
                [f.run.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        let mut record: Value = serde_json::from_str(&original).unwrap();
        let corrupted = match case {
            0 => "{truncated".into(),
            1 => "x".repeat(MAX_RECEIPT + 1),
            2 => {
                record["revision"] = json!(0);
                record.to_string()
            }
            3 => {
                record["tool_call_ids"] = json!(["owned", "owned"]);
                record.to_string()
            }
            4 => {
                record["tool_call_ids"] = json!([]);
                record.to_string()
            }
            _ => {
                record["automatic"] = json!(true);
                record.to_string()
            }
        };
        f.journal
            .connection
            .execute(
                "UPDATE local_tool_reconciliations SET record=?1 WHERE run_id=?2",
                params![corrupted, f.run.id.to_string()],
            )
            .unwrap();
        assert!(f.journal.reconcile_local_tools(&f.guard, &request).is_err());
        assert_eq!(f.saved(), before);
        let retained: String = f
            .journal
            .connection
            .query_row(
                "SELECT record FROM local_tool_reconciliations WHERE run_id=?1",
                [f.run.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(retained, corrupted);
    }
}

#[test]
fn automatic_unknown_outcomes_preserve_retained_cleanup_and_cannot_be_reinterpreted_as_operator_attestation()
 {
    let mut f = Fixture::new();
    f.calls(&["unresolved"]);
    f.end(false);
    f.journal.retain_interrupted_cleanup(&f.guard).unwrap();
    let request = f.request();
    f.journal.recover_tool_outcomes(&f.guard).unwrap();
    let after = f.saved();
    assert!(f.journal.reconcile_local_tools(&f.guard, &request).is_err());
    f.journal.recover_tool_outcomes(&f.guard).unwrap();
    assert_eq!(f.saved(), after);
    assert_eq!(
        f.journal.run(f.run.id).unwrap().state,
        RunState::Interrupted
    );
    let retained: Option<String> = f
        .journal
        .connection
        .query_row(
            "SELECT confirmation FROM process_retained_cleanup WHERE run_id=?1",
            [f.run.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(retained.is_none());
}

fn pending(ids: &[&str]) -> Message {
    let mut message = Message::new(Role::Assistant, "");
    message.tool_calls = ids
        .iter()
        .map(|id| crate::model::ToolCall {
            id: (*id).into(),
            name: "owned".into(),
            arguments: json!({}),
        })
        .collect();
    message
}
#[test]
fn ambiguous_legacy_tool_groups_are_refused_instead_of_inserting_unknown_results_into_history() {
    for messages in [
        vec![pending(&["same", "same"])],
        vec![pending(&[""])],
        vec![pending(&["bad\u{1b}"])],
        vec![
            pending(&["pending"]),
            Message::new(Role::User, "later user history"),
        ],
        vec![
            pending(&["pending"]),
            Message::new(Role::Assistant, "later assistant history"),
        ],
        vec![
            pending(&["pending"]),
            Message::tool("foreign", "orphan result"),
        ],
        vec![
            pending(&["done"]),
            Message::tool("done", "result"),
            Message::tool("done", "duplicate result"),
        ],
        vec![Message::new(Role::Tool, "no identity")],
    ] {
        assert!(unresolved(&messages).is_err());
    }
    let mut wrong = pending(&["call"]);
    wrong.role = Role::User;
    assert!(unresolved(&[wrong]).is_err());
    let mut wrong = Message::new(Role::Assistant, "bad tool shape");
    wrong.tool_call_id = Some("orphan".into());
    assert!(unresolved(&[wrong]).is_err());
    assert!(unresolved(&[pending(&vec!["x"; MAX_CALLS + 1])]).is_err());
}

#[test]
fn completed_old_tool_ids_can_be_reused_by_a_new_interrupted_response_without_losing_order() {
    let messages = vec![
        Message::tool("old-orphan", "legacy prior outcome"),
        pending(&["reused"]),
        Message::tool("reused", "completed old group"),
        pending(&["reused", "new"]),
        Message::tool("new", "known latest result"),
    ];
    assert_eq!(unresolved(&messages).unwrap(), vec!["reused"]);
}
