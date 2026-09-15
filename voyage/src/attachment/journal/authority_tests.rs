use super::*;

fn fixture() -> (
    tempfile::TempDir,
    Journal,
    Session,
    ExecutionGuard,
    TurnAdmission,
) {
    let root = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(root.path().join("journal")).unwrap();
    let session = Session::new(root.path().into(), "fixture".into());
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        coordination: None,
        operator_name: Some("First operator".into()),
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 61_000,
        prompt: "durable input".into(),
        parts: vec![],
    };
    (root, journal, session, guard, request)
}

#[test]
fn command_deduplication_binds_payload_and_authority_but_not_presentation() {
    let (root, mut journal, session, guard, request) = fixture();
    assert!(journal.lookup_command(&request).unwrap().is_none());
    let run = journal.admit_turn(&guard, &request, 1_000).unwrap().run;
    let before = journal.load_session(session.id).unwrap().revision;
    let mut renamed = request.clone();
    renamed.operator_name = Some("Renamed operator".into());
    // Duplicate receipts are readable after expiry and never invoke the admission clock.
    let retry = journal
        .admit_turn_with_clock(&guard, &renamed, || anyhow::bail!("clock must not run"))
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.run.id, run.id);
    assert_eq!(journal.load_session(session.id).unwrap().revision, before);
    for case in 0..6 {
        let mut changed = request.clone();
        match case {
            0 => changed.machine_id = Uuid::new_v4(),
            1 => changed.principal_id = Uuid::new_v4(),
            2 => changed.prompt.push('!'),
            3 => changed.expected_revision += 1,
            4 => changed.expires_at_ms += 1,
            _ => changed.session_id = Uuid::new_v4(),
        }
        assert!(journal.lookup_command(&changed).is_err());
        assert!(journal.admit_turn(&guard, &changed, 1_000).is_err());
    }
    drop(journal);
    let journal = Journal::open(root.path().join("journal")).unwrap();
    assert_eq!(
        journal.lookup_command(&request).unwrap().unwrap().id,
        run.id
    );
    let loaded = journal.load_session(session.id).unwrap();
    assert_eq!(loaded.session.messages.len(), 1);
    assert_eq!(
        loaded.session.messages[0].operator_name.as_deref(),
        Some("First operator")
    );
}

#[test]
fn refused_admissions_are_atomic_and_do_not_reserve_command_ids() {
    for case in 0..10 {
        let (_root, mut journal, session, guard, request) = fixture();
        let mut invalid = request.clone();
        let now = match case {
            0 => {
                invalid.command_id = Uuid::nil();
                1_000
            }
            1 => {
                invalid.machine_id = Uuid::nil();
                1_000
            }
            2 => {
                invalid.principal_id = Uuid::nil();
                1_000
            }
            3 => {
                invalid.prompt = " \n\t".into();
                1_000
            }
            4 => {
                invalid.prompt = "x".repeat(MAX_PROMPT + 1);
                1_000
            }
            5 => {
                invalid.expires_at_ms = 1_000;
                1_000
            }
            6 => {
                invalid.expires_at_ms = 301_001;
                1_000
            }
            7 => -1,
            8 => {
                invalid.expected_revision = 9;
                1_000
            }
            _ => {
                invalid.expires_at_ms = i64::MAX;
                i64::MAX - 1
            }
        };
        assert!(journal.admit_turn(&guard, &invalid, now).is_err());
        let unchanged = journal.load_session(session.id).unwrap();
        assert_eq!(unchanged.revision, 0);
        assert!(unchanged.session.messages.is_empty());
        let count: i64 = journal
            .connection
            .query_row("SELECT count(*) FROM commands", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        assert!(
            !journal
                .admit_turn(&guard, &request, 1_000)
                .unwrap()
                .duplicate
        );
    }
}

#[test]
fn execution_guards_are_exclusive_and_bound_to_journal_and_session() {
    let (_root, mut journal, session, guard, request) = fixture();
    assert!(journal.acquire_execution(session.id).is_err());
    let missing = Uuid::new_v4();
    assert!(journal.acquire_execution(missing).is_err());
    assert!(
        !journal
            .directory
            .join(format!("{missing}.owner.lock"))
            .exists()
    );
    assert!(journal.acquire_execution(Uuid::nil()).is_err());
    let other = Session::new(session.workspace.clone(), "other".into());
    journal.create_session(&other).unwrap();
    let other_guard = journal.acquire_execution(other.id).unwrap();
    assert!(journal.admit_turn(&other_guard, &request, 1_000).is_err());
    let (_other_root, mut foreign, _, _, _) = fixture();
    assert!(foreign.admit_turn(&guard, &request, 1_000).is_err());
    drop(guard);
    assert!(journal.acquire_execution(session.id).is_ok());
}

#[test]
fn interrupted_dispatch_retains_partial_evidence_but_never_publishes_it_as_answer() {
    for dispatched in [false, true] {
        let (root, mut journal, session, guard, request) = fixture();
        let run = journal.admit_turn(&guard, &request, 1_000).unwrap().run;
        if dispatched {
            journal.mark_running(&guard, run.id).unwrap();
            journal
                .append_text(&guard, run.id, "uncertain partial")
                .unwrap();
        }
        let mut next = request.clone();
        next.command_id = Uuid::new_v4();
        next.expected_revision = journal.load_session(session.id).unwrap().revision;
        assert!(journal.admit_turn(&guard, &next, 1_000).is_err());
        drop(guard);
        drop(journal);
        let mut reopened = Journal::open(root.path().join("journal")).unwrap();
        let owner = reopened.acquire_execution(session.id).unwrap();
        let recovered = reopened.recover_interrupted(&owner).unwrap().unwrap();
        assert_eq!(recovered.id, run.id);
        assert_eq!(recovered.state, RunState::Interrupted);
        assert_eq!(
            recovered.partial_text,
            if dispatched { "uncertain partial" } else { "" }
        );
        assert!(reopened.recover_interrupted(&owner).unwrap().is_none());
        let canonical = reopened.load_session(session.id).unwrap();
        assert_eq!(canonical.session.messages.len(), 1);
        assert!(
            reopened
                .admit_turn(&owner, &request, i64::MAX)
                .unwrap()
                .duplicate
        );
        next.expected_revision = canonical.revision;
        assert!(reopened.admit_turn(&owner, &next, 1_000).is_ok());
    }
}

#[test]
fn terminal_validation_rolls_back_and_completed_runs_cannot_be_rewritten() {
    let (_root, mut journal, session, guard, request) = fixture();
    let run = journal.admit_turn(&guard, &request, 1_000).unwrap().run;
    assert!(
        journal
            .finish(&guard, run.id, RunState::Completed, None, Some("answer"))
            .is_err()
    );
    assert!(
        journal
            .append_text(&guard, run.id, "before dispatch")
            .is_err()
    );
    journal.mark_running(&guard, run.id).unwrap();
    assert!(journal.mark_running(&guard, run.id).is_err());
    let revision = journal.load_session(session.id).unwrap().revision;
    for (state, reason, text) in [
        (RunState::Running, None, None),
        (RunState::Accepted, None, None),
        (RunState::Failed, None, None),
        (RunState::Cancelled, Some(""), None),
        (RunState::Failed, Some("bad\nreason"), None),
        (
            RunState::Incomplete,
            Some("unfinished"),
            Some("false success"),
        ),
        (RunState::Completed, None, None),
    ] {
        assert!(journal.finish(&guard, run.id, state, reason, text).is_err());
        assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
        assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
    }
    assert!(journal.append_text(&guard, run.id, "").is_err());
    journal.append_text(&guard, run.id, "partial").unwrap();
    let finished = journal
        .finish(
            &guard,
            run.id,
            RunState::Completed,
            None,
            Some("canonical answer"),
        )
        .unwrap();
    assert_eq!(finished.state, RunState::Completed);
    assert_eq!(finished.partial_text, "partial");
    assert!(journal.append_text(&guard, run.id, "late").is_err());
    assert!(
        journal
            .finish(&guard, run.id, RunState::Failed, Some("rewrite"), None)
            .is_err()
    );
    let loaded = journal.load_session(session.id).unwrap();
    assert_eq!(loaded.session.messages.len(), 2);
    assert_eq!(
        loaded.session.messages[1].operator_name.as_deref(),
        Some("First operator")
    );
    assert!(journal.lookup_command(&request).unwrap().unwrap().state == RunState::Completed);
}

#[test]
fn replay_requires_contiguous_owned_events_and_rejects_forged_identity() {
    for case in 0..5 {
        let (_root, mut journal, session, guard, request) = fixture();
        let run = journal.admit_turn(&guard, &request, 1_000).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        journal.append_text(&guard, run.id, "delta").unwrap();
        let events = match journal.replay(session.id, 0).unwrap() {
            Replay::Events(events) => events,
            _ => panic!("fresh replay requires no snapshot"),
        };
        assert_eq!(events.len(), 3);
        assert_eq!(
            events.iter().map(|e| e.sequence).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert!(matches!(events[0].kind, EventKind::Accepted));
        assert!(journal.replay(session.id, 4).is_err());
        assert!(journal.replay(session.id, u64::MAX).is_err());
        match case {
            0 => {
                journal
                    .connection
                    .execute("DELETE FROM events WHERE sequence=1", [])
                    .unwrap();
            }
            1 => {
                journal
                    .connection
                    .execute("DELETE FROM events WHERE sequence=2", [])
                    .unwrap();
            }
            2 => {
                journal
                    .connection
                    .execute("DELETE FROM events", [])
                    .unwrap();
            }
            _ => {
                let mut event = events[0].clone();
                if case == 3 {
                    event.sequence = 9;
                } else {
                    event.run_id = Uuid::new_v4();
                }
                journal
                    .connection
                    .execute(
                        "UPDATE events SET event=?1 WHERE sequence=1",
                        [serde_json::to_string(&event).unwrap()],
                    )
                    .unwrap();
            }
        }
        if case < 3 {
            assert!(matches!(
                journal.replay(session.id, 0).unwrap(),
                Replay::SnapshotRequired { latest_sequence: 3 }
            ));
        } else {
            assert!(journal.replay(session.id, 0).is_err());
        }
        assert!(
            matches!(journal.replay(session.id, 3).unwrap(), Replay::Events(events) if events.is_empty())
        );
        assert_eq!(
            journal.lookup_command(&request).unwrap().unwrap().id,
            run.id
        );
    }
}

#[test]
fn imported_provenance_is_immutable_and_create_never_overwrites() {
    let (root, mut journal, session, _guard, _) = fixture();
    assert!(journal.create_session(&session).is_err());
    let mut imported = Session::new(root.path().into(), "imported".into());
    imported.revision = 17;
    let provenance = crate::attachment::migration::Provenance {
        transfer_id: Uuid::new_v4(),
        session_id: imported.id,
        source_revision: 17,
        source_sha256: "a".repeat(64),
        source: root.path().join("source"),
        destination: journal.directory.clone(),
        backup: root.path().join("backup"),
        workspace: root.path().into(),
    };
    assert_eq!(journal.preflight_import(&provenance).unwrap(), None);
    assert_eq!(
        journal.import_session(&imported, &provenance).unwrap(),
        (17, false)
    );
    assert_eq!(
        journal.import_session(&imported, &provenance).unwrap(),
        (17, true)
    );
    for case in 0..5 {
        let mut changed = provenance.clone();
        match case {
            0 => changed.transfer_id = Uuid::new_v4(),
            1 => changed.session_id = Uuid::new_v4(),
            2 => changed.source_sha256 = "b".repeat(64),
            3 => changed.source_revision += 1,
            _ => changed.backup = root.path().join("other-backup"),
        }
        assert!(journal.preflight_import(&changed).is_err());
        assert!(journal.import_session(&imported, &changed).is_err());
    }
    imported.revision += 1;
    assert!(journal.import_session(&imported, &provenance).is_err());
    assert_eq!(journal.load_session(imported.id).unwrap().revision, 17);
    assert_eq!(journal.load_session(session.id).unwrap().revision, 0);
}

#[test]
fn lifecycle_archive_restore_and_clear_have_durable_idempotent_receipts() {
    use voyage_protocol::process::RuntimeCommand;
    let (_root, mut journal, session, guard, mut request) = fixture();
    journal.initialize_process_commands(&guard).unwrap();
    journal.initialize_lifecycle(&guard).unwrap();
    let archive = RuntimeCommand::Archive {
        command_id: Uuid::new_v4(),
        expected_revision: 0,
        expires_at_ms: 61_000,
        archived: true,
    };
    let receipt = journal
        .apply_lifecycle(&guard, &archive, 1_000, None)
        .unwrap();
    assert_eq!(receipt["archived"], true);
    assert_eq!(
        journal.lifecycle_status(session.id).unwrap()["archived"],
        true
    );
    assert_eq!(
        journal
            .apply_lifecycle(&guard, &archive, 90_000, None)
            .unwrap(),
        receipt
    );
    request.expected_revision = 1;
    assert!(journal.admit_turn(&guard, &request, 1_000).is_err());
    let restore = RuntimeCommand::Archive {
        command_id: Uuid::new_v4(),
        expected_revision: 1,
        expires_at_ms: 61_000,
        archived: false,
    };
    journal
        .apply_lifecycle(&guard, &restore, 1_000, None)
        .unwrap();
    request.expected_revision = 2;
    let run = journal.admit_turn(&guard, &request, 1_000).unwrap().run;
    let clear_id = Uuid::new_v4();
    let active_clear = RuntimeCommand::Clear {
        command_id: clear_id,
        expected_revision: 3,
        expires_at_ms: 61_000,
        confirm_session_id: session.id,
    };
    assert!(
        journal
            .apply_lifecycle(&guard, &active_clear, 1_000, None)
            .is_err()
    );
    journal
        .finish(
            &guard,
            run.id,
            RunState::Failed,
            Some("fixture stopped"),
            None,
        )
        .unwrap();
    let revision = journal.load_session(session.id).unwrap().revision;
    let wrong = RuntimeCommand::Clear {
        command_id: clear_id,
        expected_revision: revision,
        expires_at_ms: 61_000,
        confirm_session_id: Uuid::new_v4(),
    };
    assert!(
        journal
            .apply_lifecycle(&guard, &wrong, 1_000, None)
            .is_err()
    );
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
    let clear = RuntimeCommand::Clear {
        command_id: clear_id,
        expected_revision: revision,
        expires_at_ms: 61_000,
        confirm_session_id: session.id,
    };
    let receipt = journal
        .apply_lifecycle(&guard, &clear, 1_000, None)
        .unwrap();
    assert_eq!(receipt["cleared"], true);
    let cleared = journal.load_session(session.id).unwrap();
    assert!(cleared.session.messages.is_empty());
    assert_eq!(cleared.revision, revision + 1);
    assert_eq!(
        journal
            .apply_lifecycle(&guard, &clear, 90_000, None)
            .unwrap(),
        receipt
    );
    assert!(
        journal
            .apply_lifecycle(&guard, &wrong, 1_000, None)
            .is_err()
    );
    assert_eq!(
        journal.lookup_command(&request).unwrap().unwrap().state,
        RunState::Failed
    );
}

#[test]
fn lifecycle_rejects_stale_expired_colliding_and_relinquished_commands_atomically() {
    use voyage_protocol::process::RuntimeCommand;
    for case in 0..7 {
        let (_root, mut journal, session, guard, request) = fixture();
        journal.initialize_process_commands(&guard).unwrap();
        journal.initialize_lifecycle(&guard).unwrap();
        let mut id = Uuid::new_v4();
        let mut revision = 0;
        let mut expiry = 61_000;
        match case {
            0 => id = Uuid::nil(),
            1 => revision = 10,
            2 => expiry = 1_000,
            3 => expiry = 301_001,
            4 => {
                let run = journal.admit_turn(&guard, &request, 1_000).unwrap().run;
                journal
                    .finish(&guard, run.id, RunState::Failed, Some("stopped"), None)
                    .unwrap();
                id = request.command_id;
                revision = journal.load_session(session.id).unwrap().revision;
            }
            5 => {
                journal
                    .connection
                    .execute(
                        "UPDATE process_lifecycle SET transfer_id=?1",
                        [Uuid::new_v4().to_string()],
                    )
                    .unwrap();
            }
            _ => {
                journal
                    .connection
                    .execute("UPDATE process_lifecycle SET deleted=1", [])
                    .unwrap();
            }
        }
        let before =
            serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap();
        let command = RuntimeCommand::Archive {
            command_id: id,
            expected_revision: revision,
            expires_at_ms: expiry,
            archived: true,
        };
        assert!(
            journal
                .apply_lifecycle(&guard, &command, 1_000, None)
                .is_err()
        );
        assert_eq!(
            serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap(),
            before
        );
        assert_eq!(
            journal.lifecycle_status(session.id).unwrap()["archived"],
            false
        );
        let receipts: i64 = journal
            .connection
            .query_row("SELECT count(*) FROM process_commands", [], |r| r.get(0))
            .unwrap();
        assert_eq!(receipts, 0);
    }
}
