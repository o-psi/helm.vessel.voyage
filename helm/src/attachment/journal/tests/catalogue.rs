use super::*;

fn cancel(request: &TurnAdmission, run_id: Uuid) -> LocalCancelRequest {
    LocalCancelRequest {
        session_id: request.session_id,
        run_id,
        installation_id: request.machine_id,
        principal_id: request.principal_id,
        expires_at_ms: 60_000,
    }
}

#[test]
fn catalogue_pages_are_bounded_metadata_only_and_authoritatively_versioned() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    for _ in 0..3 {
        let mut another = Session::new(session.workspace.clone(), "model".into());
        another.name = Some("display name".into());
        another
            .messages
            .push(Message::new(Role::User, "PRIVATE_CANONICAL_CANARY"));
        journal.create_session(&another).unwrap();
    }
    assert!(journal.list_session_summaries(None, 0).is_err());
    assert!(journal.list_session_summaries(None, 101).is_err());
    let first = journal.list_session_summaries(None, 2).unwrap();
    let second = journal.list_session_summaries(first.next_after, 2).unwrap();
    assert_eq!(first.sessions.len(), 2);
    assert_eq!(second.sessions.len(), 2);
    assert!(second.next_after.is_none());
    let mut rows = first.sessions;
    rows.extend(second.sessions);
    assert!(rows.windows(2).all(|w| w[0].id < w[1].id));
    let active = rows.iter().find(|s| s.id == session.id).unwrap();
    assert_eq!(active.revision, 1);
    assert_eq!(active.active_run.as_ref().unwrap().id, run.id);
    assert_eq!(
        active.active_run.as_ref().unwrap().state,
        RunState::Accepted
    );
    let encoded = serde_json::to_string(&rows).unwrap();
    assert!(!encoded.contains("PRIVATE_CANONICAL_CANARY"));
    assert!(!encoded.contains("messages") && !encoded.contains("provider"));
    assert!(journal.create_session(&session).is_err());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 1);
}

#[test]
fn cancel_requests_bind_the_exact_local_actor_run_and_deadline_without_the_execution_lock() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    let mut other = Journal::open(journal.directory.clone()).unwrap();
    assert!(other.acquire_execution(session.id).is_err());
    let correct = cancel(&request, run.id);
    for field in 0..4 {
        let mut wrong = correct.clone();
        match field {
            0 => wrong.installation_id = Uuid::new_v4(),
            1 => wrong.principal_id = Uuid::new_v4(),
            2 => wrong.session_id = Uuid::new_v4(),
            _ => wrong.run_id = Uuid::new_v4(),
        }
        assert!(
            other
                .request_cancel_local_with_clock(&wrong, || Ok(1))
                .is_err()
        );
        assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
    }
    for now in [-1, 60_000, i64::MAX] {
        assert!(
            other
                .request_cancel_local_with_clock(&correct, || Ok(now))
                .is_err()
        );
        assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
    }
    assert!(
        other
            .request_cancel_local_with_clock(&correct, || anyhow::bail!("clock unavailable"))
            .is_err()
    );
    assert_eq!(
        other
            .request_cancel_local_with_clock(&correct, || Ok(1))
            .unwrap(),
        CancelRequestOutcome::Requested { duplicate: false }
    );
    assert_eq!(
        other
            .request_cancel_local_with_clock(&correct, || panic!("retry samples no clock"))
            .unwrap(),
        CancelRequestOutcome::Requested { duplicate: true }
    );
    assert!(journal.local_cancel_requested(session.id, run.id).unwrap());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 1);
    assert_eq!(journal.run(run.id).unwrap().state, RunState::Accepted);
    assert!(journal.mark_running(&guard, run.id).is_err());
    assert!(other.acquire_execution(session.id).is_err());
}

#[test]
fn committed_cancel_wins_acceptance_and_terminal_success_but_never_rewrites_prior_terminal() {
    for accepted_first in [false, true] {
        let (_dir, mut journal, session, request) = setup();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        let messages = vec![
            Message::new(Role::User, &request.prompt),
            Message::new(Role::Assistant, "retained provisional text"),
        ];
        journal
            .checkpoint_canonical(&guard, run.id, &messages, &Usage::default())
            .unwrap();
        if accepted_first {
            journal
                .accept_checkpoint(&guard, run.id, &messages, &Usage::default())
                .unwrap();
        }
        journal
            .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
            .unwrap();
        assert!(
            journal
                .accept_checkpoint(&guard, run.id, &messages, &Usage::default())
                .is_err()
        );
        let terminal = journal
            .finish_classified(
                &guard,
                run.id,
                RunState::Completed,
                None,
                None,
                Some(&crate::agent::StopReason::Completed),
            )
            .unwrap();
        assert_eq!(terminal.state, RunState::Cancelled);
        let current = journal.load_session(session.id).unwrap();
        assert_eq!(
            current.session.messages.last().unwrap().content,
            "retained provisional text"
        );
        let revision = current.revision;
        assert_eq!(
            journal
                .request_cancel_local_with_clock(&cancel(&request, run.id), || panic!(
                    "terminal observes no clock"
                ))
                .unwrap(),
            CancelRequestOutcome::AlreadyTerminal {
                state: RunState::Cancelled
            }
        );
        assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
    }
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    journal
        .finish(
            &guard,
            run.id,
            RunState::Completed,
            None,
            Some("completed before cancel"),
        )
        .unwrap();
    let revision = journal.load_session(session.id).unwrap().revision;
    assert_eq!(
        journal
            .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
            .unwrap(),
        CancelRequestOutcome::AlreadyTerminal {
            state: RunState::Completed
        }
    );
    assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
    assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
}

#[test]
fn cancel_intent_survives_reopen_but_recovery_remains_interrupted() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal
        .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
        .unwrap();
    let path = journal.directory.clone();
    drop(guard);
    drop(journal);
    let mut journal = Journal::open(path).unwrap();
    assert!(journal.local_cancel_requested(session.id, run.id).unwrap());
    let guard = journal.acquire_execution(session.id).unwrap();
    assert_eq!(
        journal.recover_interrupted(&guard).unwrap().unwrap().state,
        RunState::Interrupted
    );
    assert!(
        journal
            .admit_turn(&guard, &request, 90000)
            .unwrap()
            .duplicate
    );
}

#[test]
fn schema_four_upgrade_is_quiescent_preserves_steering_and_fences_stale_connections() {
    let (_dir, journal, session, request) = setup();
    journal
        .connection
        .execute_batch("DROP TABLE remote_withdrawal; DROP TABLE remote_cleanup_attestations; DROP TABLE remote_text; DROP TABLE remote_tools; DROP TABLE remote_events; DROP TABLE remote_receipts; DROP TABLE remote_session; DROP TABLE local_tool_reconciliations; DROP TABLE local_cleanup_obligations; DROP TABLE local_cancel_intents; UPDATE attachment_schema SET version=4;")
        .unwrap();
    let path = journal.directory.clone();
    drop(journal);
    let mut journal = Journal::open(path.clone()).unwrap();
    let stale = Journal::open(path).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    assert!(journal.upgrade_quiescent().is_err());
    journal.mark_running(&guard, run.id).unwrap();
    assert!(journal.steering_page(run.id, 0, 1).unwrap().is_empty());
    journal
        .finish(&guard, run.id, RunState::Cancelled, Some("fixture"), None)
        .unwrap();
    assert!(journal.upgrade_quiescent().is_err());
    drop(guard);
    journal.upgrade_quiescent().unwrap();
    assert_eq!(journal.opened_schema, SCHEMA_VERSION);
    assert!(stale.list_session_summaries(None, 1).is_err());
    assert!(journal.steering_page(run.id, 0, 1).unwrap().is_empty());
    assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
}

#[test]
fn busy_and_failed_insert_leave_no_cancel_receipt_and_preserve_sqlite_error() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    let mut other = Journal::open(journal.directory.clone()).unwrap();
    let tx = journal
        .connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .unwrap();
    let error = other
        .request_cancel_local_with_clock(&cancel(&request, run.id), || {
            panic!("busy samples no clock")
        })
        .unwrap_err();
    assert!(error.downcast_ref::<rusqlite::Error>().is_some());
    tx.rollback().unwrap();
    other.connection.execute_batch("CREATE TRIGGER reject_cancel BEFORE INSERT ON local_cancel_intents BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        other
            .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
            .is_err()
    );
    assert!(!journal.local_cancel_requested(session.id, run.id).unwrap());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 1);
    other
        .connection
        .execute_batch("DROP TRIGGER reject_cancel;")
        .unwrap();
    other
        .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
        .unwrap();
    let mut retry = cancel(&request, run.id);
    retry.expires_at_ms = -1;
    assert_eq!(
        other
            .request_cancel_local_with_clock(&retry, || panic!("existing intent never expires"))
            .unwrap(),
        CancelRequestOutcome::Requested { duplicate: true }
    );
}

#[test]
fn catalogue_and_cancel_ignore_provider_payload_but_reject_malformed_bindings() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    // Valid JSON with invalid conversation/partial field types proves these APIs
    // do not deserialize the full canonical Session or provider output record.
    journal
        .connection
        .execute(
            "UPDATE sessions SET state=json_set(state,'$.messages',42)",
            [],
        )
        .unwrap();
    journal
        .connection
        .execute(
            "UPDATE runs SET record=json_set(record,'$.partial_text',42)",
            [],
        )
        .unwrap();
    assert!(journal.load_session(session.id).is_err());
    assert!(journal.run(run.id).is_err());
    assert_eq!(
        journal
            .list_session_summaries(None, 1)
            .unwrap()
            .sessions
            .len(),
        1
    );
    journal
        .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
        .unwrap();
    assert!(journal.local_cancel_requested(session.id, run.id).unwrap());
    journal
        .connection
        .execute(
            "UPDATE runs SET record=json_set(record,'$.principal_id',?1)",
            [Uuid::new_v4().to_string()],
        )
        .unwrap();
    assert!(journal.local_cancel_requested(session.id, run.id).is_err());
    assert!(
        journal
            .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
            .is_err()
    );
}

#[test]
fn cross_process_cancel_fixture() {
    let Some(path) = std::env::var_os("HELM_LOCAL_CANCEL_FIXTURE") else {
        return;
    };
    let mut journal = Journal::open(PathBuf::from(path)).unwrap();
    let page = journal.list_session_summaries(None, 1).unwrap();
    let session = &page.sessions[0];
    let run_id = session.active_run.as_ref().unwrap().id;
    assert!(journal.acquire_execution(session.id).is_err());
    let request = LocalCancelRequest {
        session_id: session.id,
        run_id,
        installation_id: std::env::var("HELM_LOCAL_CANCEL_INSTALLATION")
            .unwrap()
            .parse()
            .unwrap(),
        principal_id: std::env::var("HELM_LOCAL_CANCEL_PRINCIPAL")
            .unwrap()
            .parse()
            .unwrap(),
        expires_at_ms: 60_000,
    };
    journal
        .request_cancel_local_with_clock(&request, || Ok(1))
        .unwrap();
    // Abrupt process exit proves receipt persistence independent of destructor.
    std::process::exit(0);
}

#[test]
fn independent_process_requests_cancellation_without_taking_execution_ownership() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "attachment::journal::tests::catalogue::cross_process_cancel_fixture",
            "--nocapture",
        ])
        .env("HELM_LOCAL_CANCEL_FIXTURE", &journal.directory)
        .env(
            "HELM_LOCAL_CANCEL_INSTALLATION",
            request.machine_id.to_string(),
        )
        .env(
            "HELM_LOCAL_CANCEL_PRINCIPAL",
            request.principal_id.to_string(),
        )
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(journal.local_cancel_requested(session.id, run.id).unwrap());
    assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
    let terminal = journal
        .finish(
            &guard,
            run.id,
            RunState::Completed,
            None,
            Some("must not publish success"),
        )
        .unwrap();
    assert_eq!(terminal.state, RunState::Cancelled);
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
    let mut next = request;
    next.command_id = Uuid::new_v4();
    next.expected_revision = journal.load_session(session.id).unwrap().revision;
    let fresh = journal.admit_turn(&guard, &next, 1).unwrap().run;
    assert!(
        !journal
            .local_cancel_requested(session.id, fresh.id)
            .unwrap()
    );
    journal.mark_running(&guard, fresh.id).unwrap();
}

#[test]
fn cleanup_obligation_survives_terminal_and_recovery_until_distinct_immutable_confirmation() {
    for observed in [true, false] {
        let (_dir, mut journal, session, request) = setup();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
        journal.register_local_cleanup(&guard, run.id).unwrap();
        journal.register_local_cleanup(&guard, run.id).unwrap();
        assert_eq!(
            journal.list_session_summaries(None, 1).unwrap().sessions[0].pending_cleanup_run,
            Some(run.id)
        );
        assert!(
            journal
                .confirm_local_cleanup_observed(&guard, run.id)
                .is_err()
        );
        journal.mark_running(&guard, run.id).unwrap();
        assert!(journal.register_local_cleanup(&guard, run.id).is_err());
        let path = journal.directory.clone();
        drop(guard);
        drop(journal);
        let mut journal = Journal::open(path).unwrap();
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.recover_interrupted(&guard).unwrap();
        assert!(
            journal
                .admit_turn(&guard, &request, 90_000)
                .unwrap()
                .duplicate
        );
        let mut next = TurnAdmission {
            command_id: Uuid::new_v4(),
            expected_revision: journal.load_session(session.id).unwrap().revision,
            ..request
        };
        assert!(journal.admit_turn(&guard, &next, 1).is_err());
        assert!(
            journal
                .attest_local_cleanup(&guard, run.id, Uuid::new_v4(), next.principal_id)
                .is_err()
        );
        if observed {
            journal
                .confirm_local_cleanup_observed(&guard, run.id)
                .unwrap();
            journal
                .confirm_local_cleanup_observed(&guard, run.id)
                .unwrap();
            assert!(
                journal
                    .attest_local_cleanup(&guard, run.id, next.machine_id, next.principal_id)
                    .is_err()
            );
        } else {
            journal
                .attest_local_cleanup(&guard, run.id, next.machine_id, next.principal_id)
                .unwrap();
            journal
                .attest_local_cleanup(&guard, run.id, next.machine_id, next.principal_id)
                .unwrap();
            assert!(
                journal
                    .confirm_local_cleanup_observed(&guard, run.id)
                    .is_err()
            );
        }
        assert!(
            journal.list_session_summaries(None, 1).unwrap().sessions[0]
                .pending_cleanup_run
                .is_none()
        );
        let provenance: String = journal
            .connection
            .query_row(
                "SELECT confirmation FROM local_cleanup_obligations WHERE run_id=?1",
                [run.id.to_string()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            provenance,
            if observed {
                "observed"
            } else {
                "operator_attested"
            }
        );
        next.expected_revision = journal.load_session(session.id).unwrap().revision;
        journal.admit_turn(&guard, &next, 1).unwrap();
    }
}

#[test]
fn failed_cleanup_registration_commits_no_obligation_and_terminal_cleanup_blocks_next_turn() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.connection.execute_batch("CREATE TRIGGER reject_cleanup BEFORE INSERT ON local_cleanup_obligations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(journal.register_local_cleanup(&guard, run.id).is_err());
    assert!(
        journal.list_session_summaries(None, 1).unwrap().sessions[0]
            .pending_cleanup_run
            .is_none()
    );
    journal
        .connection
        .execute_batch("DROP TRIGGER reject_cleanup;")
        .unwrap();
    journal.register_local_cleanup(&guard, run.id).unwrap();
    journal
        .finish(
            &guard,
            run.id,
            RunState::Cancelled,
            Some("before dispatch"),
            None,
        )
        .unwrap();
    let next = TurnAdmission {
        command_id: Uuid::new_v4(),
        expected_revision: journal.load_session(session.id).unwrap().revision,
        ..request
    };
    assert!(journal.admit_turn(&guard, &next, 1).is_err());
    journal
        .confirm_local_cleanup_observed(&guard, run.id)
        .unwrap();
    journal.admit_turn(&guard, &next, 1).unwrap();
}

#[test]
fn cancellation_settles_pending_steering_and_failed_cleanup_confirmation_stays_blocked() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.register_local_cleanup(&guard, run.id).unwrap();
    journal.mark_running(&guard, run.id).unwrap();
    let steering = SteeringAdmission {
        receipt_id: Uuid::new_v4(),
        session_id: session.id,
        run_id: run.id,
        actor: SteeringActor {
            machine_id: request.machine_id,
            principal_id: request.principal_id,
        },
        expected_revision: journal.load_session(session.id).unwrap().revision,
        expires_at_ms: 60_000,
        text: "queued correction".into(),
    };
    journal.queue_steering(&guard, &steering, 1).unwrap();
    journal
        .request_cancel_local_with_clock(&cancel(&request, run.id), || Ok(1))
        .unwrap();
    assert_eq!(
        journal
            .finish(
                &guard,
                run.id,
                RunState::Completed,
                None,
                Some("stale success")
            )
            .unwrap()
            .state,
        RunState::Cancelled
    );
    assert_eq!(
        journal.steering_record(steering.receipt_id).unwrap().status,
        crate::model::SteeringStatus::NotApplied
    );
    assert_eq!(
        journal.steering_record(steering.receipt_id).unwrap().reason,
        Some(SteeringRejection::Cancelled)
    );
    journal.connection.execute_batch("CREATE TRIGGER reject_confirmation BEFORE UPDATE ON local_cleanup_obligations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        journal
            .confirm_local_cleanup_observed(&guard, run.id)
            .is_err()
    );
    assert_eq!(
        journal.list_session_summaries(None, 1).unwrap().sessions[0].pending_cleanup_run,
        Some(run.id)
    );
    journal
        .connection
        .execute_batch("DROP TRIGGER reject_confirmation;")
        .unwrap();
    journal
        .confirm_local_cleanup_observed(&guard, run.id)
        .unwrap();
}
