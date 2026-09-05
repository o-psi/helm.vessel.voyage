use super::*;

#[tokio::test]
async fn idle_owner_and_clones_exclude_competitors_and_expose_sqlite_revision() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let mut session = Session::new(root.path().to_owned(), "fixture".into());
    session.revision = 99; // Stored payload revision is not the SQLite authority.
    let mut journal = Journal::open(path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    let owner = ManagedSessionOwner::open(path.clone(), session.id)
        .await
        .unwrap();
    let snapshot = owner.snapshot().await.unwrap();
    assert_eq!(snapshot.revision, 0);
    assert_eq!(snapshot.session.revision, snapshot.revision);
    let mut expected = serde_json::to_value(&session).unwrap();
    expected["revision"] = serde_json::json!(0);
    assert_eq!(serde_json::to_value(&snapshot.session).unwrap(), expected);
    assert_eq!(
        journal.load_session(session.id).unwrap().session.revision,
        99
    );
    assert!(journal.acquire_execution(session.id).is_err());
    let clone = owner.clone();
    drop(owner);
    assert!(
        ManagedSessionOwner::open(path.clone(), session.id)
            .await
            .is_err()
    );
    drop(clone);
    assert!(ManagedSessionOwner::open(path, session.id).await.is_ok());
}

fn managed(run: &RunOwner, request: &TurnAdmission) -> ManagedSessionOwner {
    ManagedSessionOwner {
        store: run.store.clone(),
        session_id: request.session_id,
    }
}
fn next_request(previous: &TurnAdmission, revision: u64) -> TurnAdmission {
    TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: previous.machine_id,
        principal_id: previous.principal_id,
        session_id: previous.session_id,
        expected_revision: revision,
        expires_at_ms: 60000,
        prompt: "accepted prompt again".into(),
    }
}

#[tokio::test]
async fn next_turn_reuses_guard_only_after_run_and_callback_cleanup_and_accounts_usage_once() {
    let (root, mut first, agent, requests, effects, request) =
        setup("normal", Arc::new(SilentSink)).await;
    let owner = managed(&first, &request);
    let callback = first.checkpoint();
    first
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    let after_first = owner.snapshot().await.unwrap();
    assert_eq!(after_first.session.usage.input_tokens, 6);
    assert_eq!(after_first.session.usage.output_tokens, 4);
    assert_eq!(after_first.session.revision, after_first.revision);
    assert!(
        after_first
            .session
            .messages
            .iter()
            .any(|m| m.provider_state.is_some())
    );
    assert!(
        owner
            .admit(next_request(&request, after_first.revision), 1)
            .await
            .is_err()
    );
    assert!(owner.recover_interrupted().await.is_err());
    let Admission::Existing(existing) = owner.admit(request, 90000).await.unwrap() else {
        panic!("retry must not dispatch")
    };
    assert_eq!(existing.id, first.run_id);
    assert!(callback.partial("late stale output").await.is_err());
    drop(first);
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: existing.machine_id,
        principal_id: existing.principal_id,
        session_id: existing.session_id,
        expected_revision: after_first.revision,
        expires_at_ms: 60000,
        prompt: "accepted prompt again".into(),
    };
    assert!(
        owner
            .admit(next_request(&request, after_first.revision), 1)
            .await
            .is_err()
    );
    drop(callback);
    let Admission::New(mut second) = owner.admit(request, 1).await.unwrap() else {
        panic!()
    };
    second
        .execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(
        second
            .execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    let final_state = owner.snapshot().await.unwrap();
    assert!(final_state.revision > after_first.revision);
    assert_eq!(final_state.session.revision, final_state.revision);
    assert_eq!(final_state.session.usage.input_tokens, 9);
    assert_eq!(final_state.session.usage.output_tokens, 6);
    assert_eq!(requests.load(Ordering::SeqCst), 3);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(
        Journal::open(root.path().join("attachment"))
            .unwrap()
            .acquire_execution(final_state.session.id)
            .is_err()
    );
}

#[tokio::test]
async fn mismatched_session_revision_and_callback_identity_cannot_mutate_or_dispatch() {
    let (_root, run, _agent, requests, effects, request) =
        setup("normal", Arc::new(SilentSink)).await;
    let owner = managed(&run, &request);
    let before = owner.snapshot().await.unwrap();
    let mut wrong = next_request(&request, before.revision);
    wrong.session_id = Uuid::new_v4();
    assert!(owner.admit(wrong, 1).await.is_err());
    let mut wrong_callback = run.checkpoint();
    wrong_callback.token = Arc::new(TurnToken {
        run_id: Uuid::new_v4(),
    });
    assert!(wrong_callback.partial("must not persist").await.is_err());
    drop(wrong_callback);
    drop(run);
    owner.recover_interrupted().await.unwrap().unwrap();
    let recovered = owner.snapshot().await.unwrap();
    assert!(
        owner
            .admit(next_request(&request, recovered.revision + 1), 1)
            .await
            .is_err()
    );
    assert_eq!(owner.snapshot().await.unwrap().revision, recovered.revision);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn failed_checkpoint_retains_fence_and_recovery_requires_callback_release() {
    let (root, mut run, agent, _requests, effects, request) =
        setup("before-tool-failure", Arc::new(SilentSink)).await;
    let owner = managed(&run, &request);
    let callback = run.checkpoint();
    assert!(
        run.execute(&agent, CancellationToken::new(), None)
            .await
            .is_err()
    );
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(owner.recover_interrupted().await.is_err());
    drop(run);
    assert!(owner.recover_interrupted().await.is_err());
    assert!(
        ManagedSessionOwner::open(root.path().join("attachment"), request.session_id)
            .await
            .is_err()
    );
    drop(callback);
    let db = fixture_database(root.path().join("attachment/journal.sqlite3")).unwrap();
    db.execute_batch("DROP TRIGGER fail_checkpoint").unwrap();
    assert_eq!(
        owner.recover_interrupted().await.unwrap().unwrap().state,
        RunState::Interrupted
    );
    assert!(owner.recover_interrupted().await.unwrap().is_none());
}

#[tokio::test]
async fn aborted_storage_waiter_keeps_session_and_turn_leases_until_worker_finishes() {
    let (root, run, _agent, _requests, _effects, request) =
        setup("normal", Arc::new(SilentSink)).await;
    let callback = run.checkpoint();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let (finished, done) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        callback
            .storage(move |_| {
                let _ = started.send(());
                wait.recv_timeout(Duration::from_secs(5)).unwrap();
                let _ = finished.send(());
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    task.abort();
    let _ = task.await;
    drop(run);
    let journal = Journal::open(root.path().join("attachment")).unwrap();
    assert!(journal.acquire_execution(request.session_id).is_err());
    release.send(()).unwrap();
    done.await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(guard) = journal.acquire_execution(request.session_id) {
                drop(guard);
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn managed_owner_process_probe() {
    let Some(path) = std::env::var_os("VOYAGE_OWNER_PROBE_PATH") else {
        return;
    };
    let id = Uuid::parse_str(&std::env::var("VOYAGE_OWNER_PROBE_ID").unwrap()).unwrap();
    let journal = Journal::open(PathBuf::from(path)).unwrap();
    std::process::exit(if journal.acquire_execution(id).is_ok() {
        0
    } else {
        23
    });
}
fn process_probe(path: &std::path::Path, id: Uuid, busy: bool) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "attachment::runtime::tests::owner::managed_owner_process_probe",
        ])
        .env("VOYAGE_OWNER_PROBE_PATH", path)
        .env("VOYAGE_OWNER_PROBE_ID", id.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("ownership probe timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        child.wait_with_output().unwrap().status.code(),
        Some(if busy { 23 } else { 0 })
    );
}

#[tokio::test]
async fn independent_process_is_excluded_idle_running_post_terminal_and_callback_only() {
    let (root, mut run, agent, requests, _effects, request) =
        setup("hang", Arc::new(SilentSink)).await;
    let path = root.path().join("attachment");
    let owner = managed(&run, &request);
    let callback = run.checkpoint();
    process_probe(&path, request.session_id, true);
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let task = tokio::spawn(async move {
        let result = run.execute(&agent, worker_cancel, None).await;
        (run, result)
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    process_probe(&path, request.session_id, true);
    assert!(owner.recover_interrupted().await.is_err());
    cancel.cancel();
    let (run, result) = task.await.unwrap();
    assert!(result.is_err());
    assert_eq!(run.record().await.unwrap().state, RunState::Cancelled);
    process_probe(&path, request.session_id, true);
    drop(run);
    drop(owner);
    process_probe(&path, request.session_id, true);
    drop(callback);
    process_probe(&path, request.session_id, false);
    let idle = ManagedSessionOwner::open(path.clone(), request.session_id)
        .await
        .unwrap();
    process_probe(&path, request.session_id, true);
    drop(idle);
    process_probe(&path, request.session_id, false);
}

#[tokio::test]
async fn aborted_execution_stays_owned_and_requires_explicit_recovery_before_next_turn() {
    let (root, mut run, agent, requests, effects, request) =
        setup("hang", Arc::new(SilentSink)).await;
    let owner = managed(&run, &request);
    let task =
        tokio::spawn(async move { run.execute(&agent, CancellationToken::new(), None).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while requests.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    task.abort();
    let _ = task.await;
    let before = owner.snapshot().await.unwrap();
    assert!(
        owner
            .admit(next_request(&request, before.revision), 1)
            .await
            .is_err()
    );
    assert!(
        ManagedSessionOwner::open(root.path().join("attachment"), request.session_id)
            .await
            .is_err()
    );
    assert_eq!(
        owner.recover_interrupted().await.unwrap().unwrap().state,
        RunState::Interrupted
    );
    let snapshot = owner.snapshot().await.unwrap();
    let Admission::New(next) = owner
        .admit(next_request(&request, snapshot.revision), 1)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_ne!(next.run_id, Uuid::nil());
    assert_eq!(requests.load(Ordering::SeqCst), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stale_acceptance_callback_cannot_acknowledge_incomplete_after_terminal() {
    let (_root, mut run, agent, _requests, _effects, _request) =
        setup("normal", Arc::new(SilentSink)).await;
    run.execute(&agent, CancellationToken::new(), None)
        .await
        .unwrap();
    assert!(
        run.checkpoint()
            .accepted(
                &[],
                &Usage::default(),
                &StopReason::Incomplete {
                    reason: "late".into(),
                    readiness: None
                }
            )
            .await
            .is_err()
    );
}
