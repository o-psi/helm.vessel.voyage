use super::*;

fn setup() -> (tempfile::TempDir, Journal, Session, TurnAdmission) {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path().join("attachment")).unwrap();
    let session = Session::new(dir.path().into(), "offline-test".into());
    journal.create_session(&session).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60_000,
        prompt: "preserve the accepted turn".into(),
    };
    (dir, journal, session, request)
}

#[test]
fn admission_is_atomic_and_duplicate_is_bound_to_every_authority_field() {
    let (_dir, mut journal, session, mut request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let first = journal.admit_turn(&guard, &request, 1).unwrap();
    assert!(!first.duplicate);
    let retry = journal.admit_turn(&guard, &request, 90_000).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.run.id, first.run.id);
    let loaded = journal.load_session(session.id).unwrap();
    assert_eq!(loaded.revision, 1);
    assert_eq!(loaded.session.messages.len(), 1);
    assert_eq!(loaded.session.messages[0].content, request.prompt);
    for field in ["prompt", "principal", "machine", "revision", "deadline"] {
        let original = serde_json::to_value(&request).unwrap();
        match field {
            "prompt" => request.prompt.push('!'),
            "principal" => request.principal_id = Uuid::new_v4(),
            "machine" => request.machine_id = Uuid::new_v4(),
            "revision" => request.expected_revision = 1,
            "deadline" => request.expires_at_ms += 1,
            _ => unreachable!(),
        }
        assert!(journal.admit_turn(&guard, &request, 1).is_err());
        request.prompt = original["prompt"].as_str().unwrap().into();
        request.principal_id = Uuid::parse_str(original["principal_id"].as_str().unwrap()).unwrap();
        request.machine_id = Uuid::parse_str(original["machine_id"].as_str().unwrap()).unwrap();
        request.expected_revision = original["expected_revision"].as_u64().unwrap();
        request.expires_at_ms = original["expires_at_ms"].as_i64().unwrap();
    }
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
}

#[test]
fn invalid_admissions_leave_no_turn_or_run_and_do_not_consume_command_id() {
    let (_dir, mut journal, session, mut request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    for now in [-1, 60_000, i64::MAX] {
        assert!(journal.admit_turn(&guard, &request, now).is_err());
    }
    request.expected_revision = 99;
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    request.expected_revision = 0;
    request.prompt = " ".into();
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    request.prompt = "a".repeat(MAX_PROMPT + 1);
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    request.prompt = "valid".into();
    request.machine_id = Uuid::nil();
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 0);
    assert_eq!(
        journal
            .connection
            .query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    request.machine_id = Uuid::new_v4();
    assert!(!journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
}

#[test]
fn injected_failure_after_session_update_rolls_back_the_entire_admission() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    journal.connection.execute_batch("CREATE TRIGGER fail_admission BEFORE INSERT ON commands BEGIN SELECT RAISE(ABORT,'injected command failure'); END;").unwrap();
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    let loaded = journal.load_session(session.id).unwrap();
    assert_eq!(loaded.revision, 0);
    assert!(loaded.session.messages.is_empty());
    assert_eq!(
        journal
            .connection
            .query_row("SELECT count(*) FROM runs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(matches!(journal.replay(session.id, 0).unwrap(), Replay::Events(v) if v.is_empty()));
    journal
        .connection
        .execute_batch("DROP TRIGGER fail_admission;")
        .unwrap();
    assert!(!journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
}

#[test]
fn independent_handles_fence_execution_without_blocking_other_sessions() {
    let (dir, journal, session, _) = setup();
    let mut other = Journal::open(dir.path().join("attachment")).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    assert!(other.acquire_execution(session.id).is_err());
    let separate = Session::new(dir.path().into(), "offline".into());
    other.create_session(&separate).unwrap();
    let other_guard = other.acquire_execution(separate.id).unwrap();
    assert!(journal.check_guard(&other_guard, session.id).is_err());
    drop(guard);
    assert!(other.acquire_execution(session.id).is_ok());
}

#[test]
fn terminal_outcomes_are_immutable_and_partial_output_is_not_a_false_completion() {
    for state in [
        RunState::Completed,
        RunState::Cancelled,
        RunState::Failed,
        RunState::Interrupted,
    ] {
        let (_dir, mut journal, session, mut request) = setup();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        journal.append_text(&guard, run.id, "partial é").unwrap();
        let final_text = (state == RunState::Completed).then_some("verified final");
        journal
            .finish(
                &guard,
                run.id,
                state.clone(),
                Some("test outcome"),
                final_text,
            )
            .unwrap();
        assert!(journal.append_text(&guard, run.id, "late").is_err());
        assert!(
            journal
                .finish(
                    &guard,
                    run.id,
                    RunState::Completed,
                    None,
                    Some("late success")
                )
                .is_err()
        );
        let stored = journal.run(run.id).unwrap();
        assert_eq!(stored.state, state);
        assert_eq!(stored.partial_text, "partial é");
        let current = journal.load_session(session.id).unwrap();
        assert_eq!(current.revision, 2);
        assert_eq!(
            current.session.messages.len(),
            if state == RunState::Completed { 2 } else { 1 }
        );
        assert!(journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
        request.command_id = Uuid::new_v4();
        assert!(journal.admit_turn(&guard, &request, 1).is_err()); // stale revision
        request.expected_revision = current.revision;
        assert!(!journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
    }
}

#[test]
fn restart_preserves_accepted_and_partial_output_without_readmission() {
    let (dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    journal
        .append_text(&guard, run.id, "persist before publishing")
        .unwrap();
    drop(guard);
    drop(journal);
    let mut reopened = Journal::open(dir.path().join("attachment")).unwrap();
    let guard = reopened.acquire_execution(session.id).unwrap();
    assert_eq!(
        reopened.recover_interrupted(&guard).unwrap().unwrap().state,
        RunState::Interrupted
    );
    assert!(reopened.recover_interrupted(&guard).unwrap().is_none());
    let retry = reopened.admit_turn(&guard, &request, 90_000).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.run.id, run.id);
    assert_eq!(retry.run.state, RunState::Interrupted);
    assert_eq!(retry.run.partial_text, "persist before publishing");
    assert_eq!(
        reopened
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
}

#[test]
fn replay_eviction_requires_snapshot_but_keeps_dedup_and_partial_text() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    for _ in 0..REPLAY_LIMIT {
        journal.append_text(&guard, run.id, "x").unwrap();
    }
    assert!(
        matches!(journal.replay(session.id, 0).unwrap(), Replay::SnapshotRequired { latest_sequence } if latest_sequence == (REPLAY_LIMIT + 2) as u64)
    );
    let Replay::Events(events) = journal.replay(session.id, 2).unwrap() else {
        panic!("cursor should still be retained")
    };
    assert_eq!(events.len(), REPLAY_LIMIT as usize);
    assert_eq!(events[0].sequence, 3);
    assert!(journal.replay(session.id, u64::MAX).is_err());
    assert!(
        journal
            .replay(session.id, (REPLAY_LIMIT + 3) as u64)
            .is_err()
    );
    assert_eq!(
        journal.run(run.id).unwrap().partial_text.len(),
        REPLAY_LIMIT as usize
    );
    assert!(journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
}

#[test]
fn partial_capacity_and_failed_terminal_commit_preserve_prior_state() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    for _ in 0..MAX_PARTIAL / MAX_PROMPT {
        journal
            .append_text(&guard, run.id, &"x".repeat(MAX_PROMPT))
            .unwrap();
    }
    assert!(journal.append_text(&guard, run.id, "!").is_err());
    assert_eq!(journal.run(run.id).unwrap().partial_text.len(), MAX_PARTIAL);
    journal.connection.execute_batch("CREATE TRIGGER fail_finish BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'injected event failure'); END;").unwrap();
    assert!(
        journal
            .finish(&guard, run.id, RunState::Completed, None, Some("final"))
            .is_err()
    );
    assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
    assert_eq!(journal.load_session(session.id).unwrap().revision, 1);
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
}

#[test]
fn wrong_schema_corrupt_snapshot_and_overflow_fail_closed() {
    let (dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    journal
        .connection
        .execute("UPDATE sessions SET revision=?1", [i64::MAX])
        .unwrap();
    let mut request = request;
    request.expected_revision = i64::MAX as u64;
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    journal
        .connection
        .execute(
            "UPDATE sessions SET revision=0,next_sequence=?1",
            [i64::MAX],
        )
        .unwrap();
    request.expected_revision = 0;
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    assert!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .is_empty()
    );
    journal
        .connection
        .execute("UPDATE sessions SET state='malformed'", [])
        .unwrap();
    assert!(journal.load_session(session.id).is_err());
    journal
        .connection
        .execute("UPDATE attachment_schema SET version=999", [])
        .unwrap();
    assert!(Journal::open(dir.path().join("attachment")).is_err());
    assert_eq!(
        journal
            .connection
            .query_row("SELECT version FROM attachment_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        999
    );
}

#[test]
fn create_is_explicit_never_overwrites_and_excludes_runtime_guidance() {
    let (_dir, mut journal, session, _) = setup();
    let mut same = session.clone();
    same.messages
        .push(Message::new(Role::User, "overwrite attempt"));
    assert!(journal.create_session(&same).is_err());
    assert!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .is_empty()
    );
    same.id = Uuid::new_v4();
    same.messages
        .push(Message::new(Role::System, "runtime-only instruction"));
    journal.create_session(&same).unwrap();
    assert_eq!(
        journal
            .load_session(same.id)
            .unwrap()
            .session
            .messages
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn private_storage_rejects_symlinks_and_world_accessible_paths() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let public = dir.path().join("public");
    fs::create_dir(&public).unwrap();
    fs::set_permissions(&public, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Journal::open(public).is_err());
    let real = dir.path().join("real");
    let mut journal = Journal::open(real.clone()).unwrap();
    let alias = dir.path().join("alias");
    symlink(&real, &alias).unwrap();
    assert!(Journal::open(alias).is_err());
    let session = Session::new(dir.path().into(), "test".into());
    journal.create_session(&session).unwrap();
    let sentinel = dir.path().join("sentinel");
    fs::write(&sentinel, "untouched").unwrap();
    symlink(
        &sentinel,
        real.join(format!("{}.execution.lock", session.id)),
    )
    .unwrap();
    assert!(journal.acquire_execution(session.id).is_err());
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "untouched");
    assert_eq!(
        fs::metadata(real.join("journal.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

// Spawn this same test binary so the OS—not a Tokio mutex—proves exclusion and
// ownership release on abrupt process termination. Child sends no secrets.
#[test]
fn multiprocess_execution_lock_and_crash_recovery() {
    const ENV: &str = "HELM_JOURNAL_LOCK_CHILD";
    if let Some(path) = std::env::var_os(ENV) {
        let session_id =
            Uuid::parse_str(&std::env::var("HELM_JOURNAL_LOCK_SESSION").unwrap()).unwrap();
        let journal = Journal::open(PathBuf::from(path)).unwrap();
        let _guard = journal.acquire_execution(session_id).unwrap();
        fs::write(
            std::env::var_os("HELM_JOURNAL_LOCK_READY").unwrap(),
            "ready",
        )
        .unwrap();
        std::thread::sleep(Duration::from_secs(30));
        return;
    }
    let (dir, mut journal, session, request) = setup();
    {
        let guard = journal.acquire_execution(session.id).unwrap();
        journal.admit_turn(&guard, &request, 1).unwrap();
    }
    let ready = dir.path().join("ready");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "attachment::journal::tests::multiprocess_execution_lock_and_crash_recovery",
            "--nocapture",
        ])
        .env(ENV, dir.path().join("attachment"))
        .env("HELM_JOURNAL_LOCK_SESSION", session.id.to_string())
        .env("HELM_JOURNAL_LOCK_READY", &ready)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !ready.exists()
        && std::time::Instant::now() < deadline
        && child.try_wait().unwrap().is_none()
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let was_ready = ready.exists();
    let excluded = was_ready && journal.acquire_execution(session.id).is_err();
    let _ = child.kill();
    child.wait().unwrap();
    assert!(was_ready, "child did not acquire execution guard");
    assert!(
        excluded,
        "other process obtained active execution ownership"
    );
    let guard = journal.acquire_execution(session.id).unwrap();
    assert_eq!(
        journal.recover_interrupted(&guard).unwrap().unwrap().state,
        RunState::Interrupted
    );
    assert!(journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
}

#[test]
fn sqlite_full_and_busy_are_bounded_failures_without_admission() {
    let (dir, mut journal, session, mut request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let other = Journal::open(dir.path().join("attachment")).unwrap();
    other.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = std::time::Instant::now();
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    other.connection.execute_batch("ROLLBACK").unwrap();
    let pages: i64 = journal
        .connection
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    journal
        .connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    request.prompt = "x".repeat(MAX_PROMPT);
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 0);
    assert!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .is_empty()
    );
    assert_eq!(
        journal
            .connection
            .query_row("SELECT count(*) FROM commands", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn accepted_is_not_running_and_dispatch_intent_cannot_be_replayed() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    assert_eq!(run.state, RunState::Accepted);
    assert_eq!(run.machine_id, request.machine_id);
    assert_eq!(run.principal_id, request.principal_id);
    assert!(journal.append_text(&guard, run.id, "not started").is_err());
    assert!(
        journal
            .finish(
                &guard,
                run.id,
                RunState::Completed,
                None,
                Some("not executed")
            )
            .is_err()
    );
    journal.mark_running(&guard, run.id).unwrap();
    assert_eq!(journal.run(run.id).unwrap().state, RunState::Running);
    assert!(journal.mark_running(&guard, run.id).is_err());
    let retry = journal.admit_turn(&guard, &request, 1).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.run.state, RunState::Running);
}

#[test]
fn corrupt_identity_lifecycle_and_missing_replay_never_look_current() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    let original = serde_json::to_string(&run).unwrap();
    let mut altered = run.clone();
    altered.session_id = Uuid::new_v4();
    journal
        .connection
        .execute(
            "UPDATE runs SET record=?1",
            [serde_json::to_string(&altered).unwrap()],
        )
        .unwrap();
    assert!(journal.run(run.id).is_err());
    journal
        .connection
        .execute("UPDATE runs SET record=?1,active=0", [&original])
        .unwrap();
    assert!(journal.run(run.id).is_err());
    journal
        .connection
        .execute("UPDATE runs SET active=1", [])
        .unwrap();
    journal
        .connection
        .execute("DELETE FROM events", [])
        .unwrap();
    assert!(matches!(
        journal.replay(session.id, 0).unwrap(),
        Replay::SnapshotRequired { latest_sequence: 1 }
    ));
    assert!(journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
}

#[test]
fn missing_middle_or_tail_event_requires_snapshot_and_debug_omits_content() {
    for deleted in [2, 3] {
        let (_dir, mut journal, session, request) = setup();
        let guard = journal.acquire_execution(session.id).unwrap();
        let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
        journal.mark_running(&guard, run.id).unwrap();
        journal
            .append_text(&guard, run.id, "private-output-sentinel")
            .unwrap();
        assert!(!format!("{:?}", journal.run(run.id).unwrap()).contains("private-output-sentinel"));
        journal
            .connection
            .execute("DELETE FROM events WHERE sequence=?1", [deleted])
            .unwrap();
        assert!(matches!(
            journal.replay(session.id, 0).unwrap(),
            Replay::SnapshotRequired { latest_sequence: 3 }
        ));
    }
}

#[test]
fn unknown_sessions_do_not_allocate_execution_lock_files() {
    let (_dir, journal, _, _) = setup();
    let missing = Uuid::new_v4();
    assert!(journal.acquire_execution(missing).is_err());
    assert!(
        !journal
            .directory
            .join(format!("{missing}.execution.lock"))
            .exists()
    );
}

#[test]
fn canonical_checkpoints_preserve_prefix_usage_and_rollback_and_never_duplicate_final() {
    let (_dir, mut journal, session, request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let base = journal.load_session(session.id).unwrap().session.messages;
    let mut history = base.clone();
    history.push(Message::new(Role::Assistant, "final checkpoint"));
    let usage = Usage {
        input_tokens: 7,
        output_tokens: 3,
    };
    journal.connection.execute_batch("CREATE TRIGGER fail_event BEFORE INSERT ON events BEGIN SELECT RAISE(ABORT,'event write failed'); END;").unwrap();
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &history, &usage)
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
    assert_eq!(journal.run(run.id).unwrap().usage.input_tokens, 0);
    journal
        .connection
        .execute_batch("DROP TRIGGER fail_event;")
        .unwrap();
    journal
        .checkpoint_canonical(&guard, run.id, &history, &usage)
        .unwrap();
    let revision = journal.load_session(session.id).unwrap().revision;
    journal
        .checkpoint_canonical(&guard, run.id, &history, &usage)
        .unwrap();
    assert_eq!(journal.load_session(session.id).unwrap().revision, revision);
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &base, &usage)
            .is_err()
    );
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &history, &Usage::default())
            .is_err()
    );
    let mut edited = history.clone();
    edited[0].content = "rewritten accepted input".into();
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &edited, &usage)
            .is_err()
    );
    let mut system = history.clone();
    system.push(Message::new(Role::System, "guidance"));
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &system, &usage)
            .is_err()
    );
    assert!(
        journal
            .finish(
                &guard,
                run.id,
                RunState::Completed,
                None,
                Some("final checkpoint")
            )
            .is_err()
    );
    assert!(!journal.run(run.id).unwrap().final_checkpointed);
    assert!(
        journal
            .finish(&guard, run.id, RunState::Completed, None, None)
            .is_err()
    );
    journal
        .accept_checkpoint(&guard, run.id, &history, &usage)
        .unwrap();
    assert!(
        journal
            .accept_checkpoint(&guard, run.id, &history, &usage)
            .is_err()
    );
    journal
        .finish(&guard, run.id, RunState::Completed, None, None)
        .unwrap();
    let stored = journal.load_session(session.id).unwrap().session;
    assert_eq!(stored.messages.len(), 2);
    assert_eq!(stored.usage.input_tokens, 7);
    assert_eq!(stored.usage.output_tokens, 3);
    assert!(
        journal
            .checkpoint_canonical(&guard, run.id, &history, &usage)
            .is_err()
    );
}

#[test]
fn old_journal_schema_is_rejected_without_migrating_or_erasing_data() {
    let (dir, journal, session, _) = setup();
    journal
        .connection
        .execute("UPDATE attachment_schema SET version=1", [])
        .unwrap();
    drop(journal);
    assert!(Journal::open(dir.path().join("attachment")).is_err());
    let db = Connection::open(dir.path().join("attachment/journal.sqlite3")).unwrap();
    assert_eq!(
        db.query_row("SELECT version FROM attachment_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT id FROM sessions", [], |r| r.get::<_, String>(0))
            .unwrap(),
        session.id.to_string()
    );
}

#[test]
fn uncertain_tool_intent_cannot_be_completed_or_dispatched_on_a_new_turn() {
    let (_dir, mut journal, session, mut request) = setup();
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal.mark_running(&guard, run.id).unwrap();
    let mut history = journal.load_session(session.id).unwrap().session.messages;
    let mut intent = Message::new(Role::Assistant, "tool intent");
    intent.tool_calls.push(crate::model::ToolCall {
        id: "uncertain-call".into(),
        name: "shell".into(),
        arguments: serde_json::json!({"command":"synthetic effect"}),
    });
    history.push(intent);
    journal
        .checkpoint_canonical(&guard, run.id, &history, &crate::model::Usage::default())
        .unwrap();
    assert!(
        journal
            .finish(
                &guard,
                run.id,
                RunState::Completed,
                None,
                Some("false success")
            )
            .is_err()
    );
    journal
        .finish(
            &guard,
            run.id,
            RunState::Interrupted,
            Some("owner exited"),
            None,
        )
        .unwrap();
    assert!(journal.admit_turn(&guard, &request, 1).unwrap().duplicate);
    request.command_id = Uuid::new_v4();
    request.expected_revision = journal.load_session(session.id).unwrap().revision;
    assert!(journal.admit_turn(&guard, &request, 1).is_err());
    assert_eq!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .len(),
        history.len()
    );
    assert!(journal.lookup_command(&request).unwrap().is_none());
}

fn legacy_v2(journal: &mut Journal) {
    journal
        .connection
        .execute_batch("DROP TABLE imports; UPDATE attachment_schema SET version=2 WHERE id=1;")
        .unwrap();
    journal.opened_schema = 2;
}

#[test]
fn explicit_upgrade_preserves_canonical_runs_replay_and_dedup_and_fences_writers() {
    let (_dir, mut journal, session, request) = setup();
    legacy_v2(&mut journal);
    let guard = journal.acquire_execution(session.id).unwrap();
    let run = journal.admit_turn(&guard, &request, 1).unwrap().run;
    journal
        .finish(
            &guard,
            run.id,
            RunState::Failed,
            Some("test terminal"),
            None,
        )
        .unwrap();
    let before = serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap();
    assert!(journal.upgrade_quiescent().is_err()); // effects guard retained after terminal persistence
    drop(guard);
    let mut stale = Journal::open(journal.directory.clone()).unwrap();
    assert_eq!(stale.opened_schema, 2); // open never upgrades
    journal
        .upgrade_with(|| {
            assert!(
                stale
                    .create_session(&Session::new(session.workspace.clone(), "new".into()))
                    .is_err()
            );
            assert!(stale.acquire_execution(session.id).is_err());
            Ok(())
        })
        .unwrap();
    assert_eq!(journal.opened_schema, 3);
    assert_eq!(
        serde_json::to_value(journal.load_session(session.id).unwrap().session).unwrap(),
        before
    );
    assert_eq!(
        journal.lookup_command(&request).unwrap().unwrap().id,
        run.id
    );
    assert!(
        matches!(journal.replay(session.id, 0).unwrap(), Replay::Events(events) if events.len() == 2)
    );
    assert!(
        stale
            .create_session(&Session::new(session.workspace.clone(), "stale".into()))
            .is_err()
    );
    assert!(stale.acquire_execution(session.id).is_err());
    assert_eq!(
        Journal::open(journal.directory.clone())
            .unwrap()
            .opened_schema,
        3
    );
}

#[test]
fn active_run_or_upgrade_failure_keeps_supported_v2_unchanged() {
    let (_dir, mut journal, session, request) = setup();
    legacy_v2(&mut journal);
    let guard = journal.acquire_execution(session.id).unwrap();
    journal.admit_turn(&guard, &request, 1).unwrap();
    drop(guard);
    assert!(journal.upgrade_quiescent().is_err());
    let guard = journal.acquire_execution(session.id).unwrap();
    journal.recover_interrupted(&guard).unwrap();
    drop(guard);
    assert!(
        journal
            .upgrade_with(|| anyhow::bail!("injected upgrade failure"))
            .is_err()
    );
    assert_eq!(
        journal
            .connection
            .query_row("SELECT version FROM attachment_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        journal
            .connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='imports'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    journal.upgrade_quiescent().unwrap();
}

fn import_provenance(session: &Session, directory: &Path) -> super::super::migration::Provenance {
    super::super::migration::Provenance {
        transfer_id: Uuid::new_v4(),
        session_id: session.id,
        source_revision: session.revision,
        source_sha256: "a".repeat(64),
        source: directory.join("source.json"),
        destination: directory.to_owned(),
        backup: directory.join("backup.json"),
        workspace: session.workspace.clone(),
    }
}

#[test]
fn provenance_insert_is_atomic_create_only_and_retains_source_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path().join("journal")).unwrap();
    let mut session = Session::new(dir.path().into(), "original".into());
    session.revision = 17;
    session.messages.push(Message::new(Role::User, "preserve"));
    let provenance = import_provenance(&session, &journal.directory);
    journal.connection.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON imports BEGIN SELECT RAISE(ABORT, 'injected disk failure'); END;").unwrap();
    assert!(journal.import_session(&session, &provenance).is_err());
    assert!(journal.load_session(session.id).is_err());
    assert!(journal.preflight_import(&provenance).unwrap().is_none());
    journal
        .connection
        .execute_batch("DROP TRIGGER fail_import")
        .unwrap();
    assert_eq!(
        journal.import_session(&session, &provenance).unwrap(),
        (17, false)
    );
    assert_eq!(
        journal.import_session(&session, &provenance).unwrap(),
        (17, true)
    );
    let mut conflict = provenance.clone();
    conflict.source_sha256 = "b".repeat(64);
    assert!(journal.import_session(&session, &conflict).is_err());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 17);
    let mut collision = Session::new(dir.path().into(), "other".into());
    collision.revision = 17;
    conflict = provenance.clone();
    conflict.session_id = collision.id;
    assert!(journal.import_session(&collision, &conflict).is_err());
    assert!(journal.load_session(collision.id).is_err());
}

#[test]
fn import_sqlite_full_rolls_back_snapshot_and_provenance_together() {
    let dir = tempfile::tempdir().unwrap();
    let mut journal = Journal::open(dir.path().join("journal")).unwrap();
    let mut session = Session::new(dir.path().into(), "original".into());
    session
        .messages
        .push(Message::new(Role::User, "x".repeat(128 * 1024)));
    let provenance = import_provenance(&session, &journal.directory);
    let pages: i64 = journal
        .connection
        .pragma_query_value(None, "page_count", |r| r.get(0))
        .unwrap();
    journal
        .connection
        .pragma_update(None, "max_page_count", pages)
        .unwrap();
    assert!(journal.import_session(&session, &provenance).is_err());
    assert!(journal.load_session(session.id).is_err());
    assert!(journal.preflight_import(&provenance).unwrap().is_none());
}
