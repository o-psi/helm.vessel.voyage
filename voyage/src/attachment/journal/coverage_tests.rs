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
