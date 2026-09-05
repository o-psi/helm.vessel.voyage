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

#[test]
fn remote_recovery_keeps_local_attribution_and_never_invents_successful_tools() {
    use super::super::super::local_actor::LocalActor;
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let actor = LocalActor {
        installation_id: binding.local_installation_id,
        principal_id: binding.local_principal_id,
    };
    assert_eq!(journal.remote_local_binding(&actor).unwrap().0, session.id);
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
    assert!(
        journal
            .attest_remote_cleanup(&guard, &binding, run.id, &actor)
            .is_err()
    );
    let mut messages = journal.load_session(session.id).unwrap().session.messages;
    let mut call = Message::new(Role::Assistant, "");
    call.tool_calls.push(crate::model::ToolCall {
        id: "unknown-call".into(),
        name: "write_file".into(),
        arguments: serde_json::json!({"path":"effect","content":"unknown"}),
    });
    messages.push(call);
    journal
        .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
        .unwrap();
    journal.recover_interrupted(&guard).unwrap();
    let wrong = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: actor.principal_id,
    };
    assert!(journal.remote_local_binding(&wrong).is_err());
    assert!(
        journal
            .attest_remote_cleanup(&guard, &binding, run.id, &wrong)
            .is_err()
    );
    journal.connection.execute_batch("CREATE TRIGGER reject_attestation BEFORE INSERT ON remote_cleanup_attestations BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(
        journal
            .attest_remote_cleanup(&guard, &binding, run.id, &actor)
            .is_err()
    );
    let confirmation: Option<String> = journal
        .connection
        .query_row(
            "SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?1",
            [run.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(confirmation.is_none());
    journal
        .connection
        .execute_batch("DROP TRIGGER reject_attestation;")
        .unwrap();
    journal
        .attest_remote_cleanup(&guard, &binding, run.id, &actor)
        .unwrap();
    journal
        .attest_remote_cleanup(&guard, &binding, run.id, &actor)
        .unwrap();
    assert!(
        journal
            .confirm_local_cleanup_observed(&guard, run.id)
            .is_err()
    );
    let expected = serde_json::to_value(
        match journal.remote_replay(&binding, session.id, 0, 128).unwrap() {
            RemoteReplay::Events { events, .. } => events,
            _ => panic!("missing events"),
        },
    )
    .unwrap();
    let revision = journal.load_session(session.id).unwrap().revision;
    let request = super::super::LocalReconcileRequest {
        session_id: session.id,
        run_id: run.id,
        installation_id: actor.installation_id,
        principal_id: actor.principal_id,
        expected_revision: revision,
    };
    assert!(journal.reconcile_local_tools(&guard, &request).is_err());
    let result = journal
        .reconcile_remote_tools(&guard, &request, &binding)
        .unwrap();
    assert_eq!(result.revision, revision + 1);
    assert_eq!(result.tool_call_ids, ["unknown-call"]);
    assert!(
        journal
            .reconcile_remote_tools(&guard, &request, &binding)
            .unwrap()
            .duplicate
    );
    let saved = journal.load_session(session.id).unwrap();
    assert_eq!(
        serde_json::to_value(&saved.session.messages[..messages.len()]).unwrap(),
        serde_json::to_value(&messages).unwrap()
    );
    let last = saved.session.messages.last().unwrap();
    assert_eq!(last.tool_success, Some(false));
    assert!(last.content.contains("outcome is unknown"));
    assert_eq!(journal.run(run.id).unwrap().state, RunState::Interrupted);
    let actual = serde_json::to_value(
        match journal.remote_replay(&binding, session.id, 0, 128).unwrap() {
            RemoteReplay::Events { events, .. } => events,
            _ => panic!("missing events"),
        },
    )
    .unwrap();
    assert_eq!(actual, expected, "terminal public history stays immutable");
    let record: String = journal
        .connection
        .query_row(
            "SELECT record FROM local_tool_reconciliations WHERE run_id=?1",
            [run.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&record).unwrap();
    assert_eq!(
        record["request"]["installation_id"],
        actor.installation_id.to_string()
    );
    assert_eq!(
        record["request"]["principal_id"],
        actor.principal_id.to_string()
    );
    assert_ne!(actor.principal_id, binding.owner_id);
}

#[test]
fn schema_six_upgrade_preserves_private_authority_receipts_and_rejects_stale_writers() {
    let (_dir, mut journal, mut session, binding) = fixture();
    session
        .messages
        .push(Message::new(Role::User, "private history"));
    journal.create_session(&session).unwrap();
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
    journal
        .append_text(&guard, run.id, "private partial")
        .unwrap();
    journal
        .finish(&guard, run.id, RunState::Interrupted, Some("fixture"), None)
        .unwrap();
    let saved = serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap();
    let record = serde_json::to_value(journal.run(run.id).unwrap()).unwrap();
    let events: String = journal
        .connection
        .query_row(
            "SELECT group_concat(event) FROM events WHERE session_id=?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    drop(guard);
    journal.connection.execute_batch("DROP TABLE remote_cleanup_attestations; DROP TABLE remote_text; DROP TABLE remote_tools; DROP TABLE remote_events; DROP TABLE remote_receipts; DROP TABLE remote_session; UPDATE attachment_schema SET version=6;").unwrap();
    let path = journal.directory.clone();
    drop(journal);
    let mut journal = Journal::open(path.clone()).unwrap();
    assert_eq!(journal.opened_schema, 6);
    let mut stale = Journal::open(path).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    assert!(
        journal.upgrade_quiescent().is_err(),
        "live execution fence must block upgrade"
    );
    drop(guard);
    journal
        .upgrade_with(|| {
            assert!(
                stale
                    .create_session(&Session::new(session.workspace.clone(), "fixture".into()))
                    .is_err(),
                "creation cannot escape enumerated upgrade fences"
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(journal.opened_schema, 7);
    assert!(
        stale
            .create_session(&Session::new(
                session.workspace.clone(),
                "stale writer".into()
            ))
            .is_err()
    );
    assert_eq!(
        serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap(),
        saved
    );
    assert_eq!(
        serde_json::to_value(journal.lookup_command(&request).unwrap().unwrap()).unwrap(),
        record
    );
    let after: String = journal
        .connection
        .query_row(
            "SELECT group_concat(event) FROM events WHERE session_id=?1",
            [session.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after, events);
    assert!(journal.remote_session(&binding).unwrap().is_none());
    assert!(
        journal
            .create_remote_session(&Session::new(session.workspace, "fixture".into()), &binding)
            .is_err(),
        "upgrade never adopts private history"
    );
}

#[cfg(unix)]
#[test]
fn public_projection_commit_busy_rolls_back_private_suffix_offset_and_event_together() {
    use std::sync::Arc;
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
    journal
        .configure_remote_redaction(
            &guard,
            run.id,
            Arc::new(crate::tools::Redactor::new(["REMOTE_SECRET".into()])),
        )
        .unwrap();
    journal.mark_running(&guard, run.id).unwrap();
    journal.append_text(&guard, run.id, "REMOTE_").unwrap();
    journal
        .connection
        .pragma_update(None, "cache_spill", false)
        .unwrap();
    let before: (i64,i64,i64)=journal.connection.query_row("SELECT (SELECT next_sequence FROM remote_session),(SELECT raw_offset FROM remote_text),(SELECT count(*) FROM remote_events)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let reader = Connection::open(journal.directory.join("journal.sqlite3")).unwrap();
    reader
        .execute_batch("BEGIN DEFERRED; SELECT count(*) FROM remote_events;")
        .unwrap();
    let changes = journal.connection.total_changes();
    let error = journal.append_text(&guard, run.id, "SECRET").unwrap_err();
    assert!(
        matches!(error.downcast_ref::<rusqlite::Error>(),Some(rusqlite::Error::SqliteFailure(code,_)) if code.code==rusqlite::ErrorCode::DatabaseBusy)
    );
    assert!(
        journal.connection.total_changes() > changes + 3,
        "public projection ran before COMMIT failed"
    );
    assert!(journal.connection.is_autocommit());
    let after:(i64,i64,i64)=journal.connection.query_row("SELECT (SELECT next_sequence FROM remote_session),(SELECT raw_offset FROM remote_text),(SELECT count(*) FROM remote_events)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(after, before);
    assert_eq!(journal.run(run.id).unwrap().partial_text, "REMOTE_");
    reader.execute_batch("ROLLBACK;").unwrap();
    journal.append_text(&guard, run.id, "SECRET").unwrap();
    assert_eq!(journal.run(run.id).unwrap().partial_text, "REMOTE_SECRET");
    let RemoteReplay::Events { events, .. } =
        journal.remote_replay(&binding, session.id, 0, 128).unwrap()
    else {
        panic!("missing public events")
    };
    let text = events
        .iter()
        .filter_map(|event| match &event.event {
            RunEvent::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(text, "[REDACTED]");
}

#[test]
fn remote_retention_requires_snapshot_and_corrupt_rows_never_skip_disclosure_checks() {
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let run = Uuid::new_v4();
    let tx = journal.connection.transaction().unwrap();
    for _ in 0..=MAX_EVENTS {
        publish(&tx, run, RunEvent::Running {}).unwrap();
    }
    tx.commit().unwrap();
    assert!(matches!(
        journal.remote_replay(&binding, session.id, 0, 128).unwrap(),
        RemoteReplay::SnapshotRequired { .. }
    ));
    let RemoteReplay::Events { events, latest } =
        journal.remote_replay(&binding, session.id, 1, 128).unwrap()
    else {
        panic!("retained suffix missing")
    };
    assert_eq!(events.len(), 128);
    assert_eq!(events[0].cursor.get(), 2);
    assert_eq!(latest, 1025);
    for limit in [0, 129, usize::MAX] {
        assert!(
            journal
                .remote_replay(&binding, session.id, 1, limit)
                .is_err()
        );
    }
    journal
        .connection
        .execute(
            "UPDATE remote_events SET event=?1 WHERE sequence=2",
            ["X".repeat(131073)],
        )
        .unwrap();
    assert!(journal.remote_replay(&binding, session.id, 1, 128).is_err());
    journal
        .connection
        .execute("UPDATE remote_session SET binding=?1", ["X".repeat(8193)])
        .unwrap();
    assert!(journal.remote_session(&binding).is_err());
    assert!(journal.remote_snapshot(&binding, session.id).is_err());
}

#[test]
fn public_outbox_byte_cap_preserves_contiguous_replay_and_bounded_pages() {
    let (_dir, mut journal, session, binding) = fixture();
    journal.create_remote_session(&session, &binding).unwrap();
    let run = Uuid::new_v4();
    let tx = journal.connection.transaction().unwrap();
    for _ in 0..600 {
        publish(
            &tx,
            run,
            RunEvent::TextDelta {
                text: "x".repeat(16384),
            },
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let (first, count, bytes): (i64, i64, i64) = journal
        .connection
        .query_row(
            "SELECT min(sequence),count(*),sum(length(CAST(event AS BLOB))) FROM remote_events",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(first > 1 && count < 600);
    assert!(bytes <= MAX_PUBLIC_BYTES as i64);
    let RemoteReplay::Events { events, latest } = journal
        .remote_replay(&binding, session.id, (first - 1) as u64, 128)
        .unwrap()
    else {
        panic!("retained suffix missing")
    };
    assert!(events.len() < 128 && !events.is_empty());
    assert!(serde_json::to_vec(&events).unwrap().len() <= 192 * 1024);
    for (offset, event) in events.iter().enumerate() {
        assert_eq!(event.cursor.get(), first as u64 + offset as u64);
    }
    assert_eq!(latest, 600);
}
