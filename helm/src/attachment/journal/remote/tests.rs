use super::*;
use crate::{model::Message, session::Session};
use voyage_protocol::attachment::{Command, Operation, VERSION};

fn binding() -> RemoteBinding {
    RemoteBinding {
        origin: "http://127.0.0.1:9480".into(),
        machine_id: Uuid::new_v4(),
        owner_id: Uuid::new_v4(),
        epoch: 1,
        local_installation_id: Uuid::new_v4(),
        local_principal_id: Uuid::new_v4(),
    }
}
fn fixture() -> (tempfile::TempDir, Journal, Session, RemoteBinding) {
    let dir = tempfile::tempdir().unwrap();
    let journal = Journal::open(dir.path().join("journal")).unwrap();
    let session = Session::new(dir.path().to_owned(), "fixture-model".into());
    (dir, journal, session, binding())
}

#[test]
fn dedicated_remote_creation_is_atomic_bound_and_never_adopts_private_history() {
    let (dir, mut journal, mut private, binding) = fixture();
    private
        .messages
        .push(Message::new(Role::User, "PRIVATE_CANARY"));
    journal.create_session(&private).unwrap();
    assert!(journal.create_remote_session(&private, &binding).is_err());
    assert!(journal.remote_session(&binding).unwrap().is_none());
    let public = Session::new(private.workspace.clone(), "fixture-model".into());
    assert!(journal.create_remote_session(&public, &binding).is_err());
    assert_eq!(
        journal.load_session(private.id).unwrap().session.messages[0].content,
        "PRIVATE_CANARY"
    );
    let mut journal = Journal::open(dir.path().join("dedicated")).unwrap();
    journal.create_remote_session(&public, &binding).unwrap();
    assert_eq!(journal.remote_session(&binding).unwrap(), Some(public.id));
    let second = Session::new(private.workspace.clone(), "fixture-model".into());
    assert!(journal.create_remote_session(&second, &binding).is_err());
    let mut changed = binding.clone();
    changed.epoch += 1;
    assert!(journal.remote_session(&changed).is_err());
    assert!(journal.remote_replay(&binding, private.id, 0, 100).is_err());
}

#[test]
fn public_replay_has_historical_revision_usage_tool_ids_and_no_private_payload() {
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "visible task".into(),
    };
    let admitted = journal.admit_turn(&guard, &request, 1).unwrap();
    journal.mark_running(&guard, admitted.run.id).unwrap();
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    let mut tool = Message::new(Role::Assistant, "");
    tool.tool_calls.push(crate::model::ToolCall {
        id: "native-non-uuid-id".into(),
        name: "read_file".into(),
        arguments: serde_json::json!({"path":"ARGUMENT_CANARY"}),
    });
    tool.provider_state = Some(serde_json::json!({"opaque":"PROVIDER_STATE_CANARY"}));
    messages.push(tool);
    journal
        .checkpoint_canonical(
            &guard,
            admitted.run.id,
            &messages,
            &Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
        )
        .unwrap();
    messages.push(Message::tool("native-non-uuid-id", "RAW_RESULT_CANARY"));
    journal
        .checkpoint_canonical(
            &guard,
            admitted.run.id,
            &messages,
            &Usage {
                input_tokens: 3,
                output_tokens: 2,
            },
        )
        .unwrap();
    let replay = journal.remote_replay(&binding, session.id, 0, 100).unwrap();
    let RemoteReplay::Events { events, .. } = replay else {
        panic!("expected complete replay")
    };
    assert!(matches!(
        events[0].event,
        RunEvent::Accepted { revision: 1, .. }
    ));
    let started = events
        .iter()
        .find_map(|e| match e.event {
            RunEvent::ToolStarted { tool_call_id, .. } => Some(tool_call_id),
            _ => None,
        })
        .unwrap();
    assert!(events.iter().any(|e|matches!(e.event,RunEvent::ToolFinished{tool_call_id,outcome:ToolOutcome::Succeeded} if tool_call_id==started)));
    assert!(events.iter().any(|e| matches!(
        e.event,
        RunEvent::Usage {
            input_tokens: 3,
            output_tokens: 2
        }
    )));
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event.cursor.get(), index as u64 + 1);
    }
    let encoded = serde_json::to_string(&events).unwrap();
    for canary in [
        "ARGUMENT_CANARY",
        "PROVIDER_STATE_CANARY",
        "RAW_RESULT_CANARY",
        "native-non-uuid-id",
    ] {
        assert!(!encoded.contains(canary));
    }
    assert!(journal.remote_replay(&binding, session.id, 0, 0).is_err());
}

#[test]
fn remote_cancel_receipt_is_atomic_exact_and_never_retargets_after_reconnect() {
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "task".into(),
    };
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    let mut cancel = Command {
        version: VERSION,
        connection_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        command_id: Uuid::new_v4(),
        expires_at_ms: 60000,
        operation: Operation::Cancel {
            session_id: session.id,
            run_id: run.id,
        },
    };
    assert!(
        !journal
            .remote_cancel_with_clock(&binding, &cancel, || Ok(2))
            .unwrap()
            .duplicate
    );
    assert!(journal.local_cancel_requested(session.id, run.id).unwrap());
    cancel.connection_id = Uuid::new_v4();
    assert!(
        journal
            .remote_cancel_with_clock(&binding, &cancel, || Ok(70000))
            .unwrap()
            .duplicate
    );
    cancel.expires_at_ms += 1;
    assert!(
        journal
            .remote_cancel_with_clock(&binding, &cancel, || Ok(2))
            .is_err()
    );
    cancel.command_id = request.command_id;
    assert!(
        journal
            .remote_cancel_with_clock(&binding, &cancel, || Ok(2))
            .is_err()
    );
    assert!(journal.mark_running(&guard, run.id).is_err());
}

#[test]
fn reused_resolved_native_call_ids_receive_distinct_public_invocation_ids() {
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "task".into(),
    };
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    for index in 0..2 {
        let mut message = Message::new(Role::Assistant, "");
        message.tool_calls.push(crate::model::ToolCall {
            id: "call_1".into(),
            name: "read_file".into(),
            arguments: serde_json::json!({"path":"private"}),
        });
        messages.push(message);
        journal
            .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
            .unwrap();
        messages.push(Message::tool("call_1", format!("result {index}")));
        journal
            .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
            .unwrap();
    }
    let RemoteReplay::Events { events, .. } =
        journal.remote_replay(&binding, session.id, 0, 100).unwrap()
    else {
        panic!("missing replay")
    };
    let started: Vec<_> = events
        .iter()
        .filter_map(|event| match event.event {
            RunEvent::ToolStarted { tool_call_id, .. } => Some(tool_call_id),
            _ => None,
        })
        .collect();
    let finished: Vec<_> = events
        .iter()
        .filter_map(|event| match event.event {
            RunEvent::ToolFinished { tool_call_id, .. } => Some(tool_call_id),
            _ => None,
        })
        .collect();
    assert_eq!(started.len(), 2);
    assert_ne!(started[0], started[1]);
    assert_eq!(started, finished);
}

#[test]
fn current_snapshot_separates_terminal_outcome_from_atomic_cleanup_observation() {
    use voyage_protocol::{events::CleanupState, stream::Reply};
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "task".into(),
    };
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.register_local_cleanup(&guard, run.id).unwrap();
    journal.mark_running(&guard, run.id).unwrap();
    journal
        .finish(
            &guard,
            run.id,
            RunState::Interrupted,
            Some("PRIVATE_ERROR_CANARY"),
            None,
        )
        .unwrap();
    let revision = journal.load_session(session.id).unwrap().revision;
    let before = journal.remote_snapshot(&binding, session.id).unwrap();
    let Reply::ExecutionSnapshot {
        session: public,
        run: Some(current),
        latest: before_cursor,
    } = before
    else {
        panic!("missing snapshot")
    };
    assert_eq!(public.revision, revision);
    assert_eq!(
        current.state,
        voyage_protocol::stream::RunState::Interrupted
    );
    assert_eq!(current.cleanup, CleanupState::Unconfirmed);
    journal
        .confirm_local_cleanup_observed(&guard, run.id)
        .unwrap();
    journal
        .confirm_local_cleanup_observed(&guard, run.id)
        .unwrap();
    let after = journal.remote_snapshot(&binding, session.id).unwrap();
    assert!(
        !serde_json::to_string(&after)
            .unwrap()
            .contains("PRIVATE_ERROR_CANARY")
    );
    let Reply::ExecutionSnapshot {
        run: Some(current),
        latest,
        ..
    } = after
    else {
        panic!("missing snapshot")
    };
    assert_eq!(current.cleanup, CleanupState::Observed);
    assert_eq!(latest.get(), before_cursor.get() + 1);
    let RemoteReplay::Events { events, .. } = journal
        .remote_replay(&binding, session.id, before_cursor.get(), 10)
        .unwrap()
    else {
        panic!("missing replay")
    };
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        RunEvent::Cleanup {
            state: CleanupState::Observed
        }
    ));
}

#[test]
fn public_text_redaction_is_transactional_across_deltas_pages_and_reopen() {
    use std::sync::Arc;
    for secret in [
        "REMOTE_SECRET_CANARY",
        "秘密🔐canary",
        "REDACTED",
        "REDA",
        "CTED",
    ] {
        let (dir, mut journal, session, binding) = fixture();
        journal.create_remote_session(&session, &binding).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        let request = TurnAdmission {
            command_id: Uuid::new_v4(),
            machine_id: binding.machine_id,
            principal_id: binding.owner_id,
            session_id: session.id,
            expected_revision: 0,
            expires_at_ms: 60000,
            prompt: "task".into(),
        };
        let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
        journal
            .configure_remote_redaction(
                &guard,
                run.id,
                Arc::new(crate::tools::Redactor::new([secret.into()])),
            )
            .unwrap();
        journal.mark_running(&guard, run.id).unwrap();
        let text = format!("before {secret} after");
        let mut cursor = 0;
        let mut disclosed = String::new();
        for character in text.chars() {
            journal
                .append_text(&guard, run.id, &character.to_string())
                .unwrap();
            loop {
                let RemoteReplay::Events { events, latest } = journal
                    .remote_replay(&binding, session.id, cursor, 1)
                    .unwrap()
                else {
                    panic!("unexpected snapshot")
                };
                for event in events {
                    cursor = event.cursor.get();
                    if let RunEvent::TextDelta { text } = event.event {
                        disclosed.push_str(&text);
                    }
                }
                if cursor == latest {
                    break;
                }
            }
        }
        journal
            .finish(
                &guard,
                run.id,
                RunState::Interrupted,
                Some("fixture interruption"),
                None,
            )
            .unwrap();
        let RemoteReplay::Events { events, .. } = journal
            .remote_replay(&binding, session.id, cursor, 128)
            .unwrap()
        else {
            panic!("unexpected snapshot")
        };
        for event in events {
            if let RunEvent::TextDelta { text } = event.event {
                disclosed.push_str(&text);
            }
        }
        assert_eq!(disclosed, "before [REDACTED] after");
        let expected = serde_json::to_value(
            match journal.remote_replay(&binding, session.id, 0, 128).unwrap() {
                RemoteReplay::Events { events, .. } => events,
                _ => panic!("missing events"),
            },
        )
        .unwrap();
        drop(guard);
        drop(journal);
        let reopened = Journal::open(dir.path().join("journal")).unwrap();
        let actual = serde_json::to_value(
            match reopened
                .remote_replay(&binding, session.id, 0, 128)
                .unwrap()
            {
                RemoteReplay::Events { events, .. } => events,
                _ => panic!("missing events"),
            },
        )
        .unwrap();
        assert_eq!(actual, expected, "replay must not rerun redaction");
        if !"[REDACTED]".contains(secret) {
            assert!(!actual.to_string().contains(secret));
        }
    }
}

#[test]
fn recovery_without_redactor_never_flushes_private_unresolved_text() {
    use std::sync::Arc;
    let (dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "task".into(),
    };
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal
        .configure_remote_redaction(
            &guard,
            run.id,
            Arc::new(crate::tools::Redactor::new(["CRASH_SECRET".into()])),
        )
        .unwrap();
    journal.mark_running(&guard, run.id).unwrap();
    journal.append_text(&guard, run.id, "CRASH_").unwrap();
    drop(guard);
    drop(journal);
    let mut journal = Journal::open(dir.path().join("journal")).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    journal.recover_interrupted(&guard).unwrap();
    let RemoteReplay::Events { events, .. } =
        journal.remote_replay(&binding, session.id, 0, 128).unwrap()
    else {
        panic!("missing events")
    };
    assert!(
        !events
            .iter()
            .any(|event| matches!(event.event, RunEvent::TextDelta { .. }))
    );
    assert_eq!(journal.run(run.id).unwrap().partial_text, "CRASH_");
}
