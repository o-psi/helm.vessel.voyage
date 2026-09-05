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
            .admit_at(next_request(&request, after_first.revision), 1)
            .await
            .is_err()
    );
    assert!(owner.recover_interrupted().await.is_err());
    let Admission::Existing(existing) = owner.admit_at(request, 90000).await.unwrap() else {
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
            .admit_at(next_request(&request, after_first.revision), 1)
            .await
            .is_err()
    );
    drop(callback);
    let Admission::New(mut second) = owner.admit_at(request, 1).await.unwrap() else {
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
    assert!(owner.admit_at(wrong, 1).await.is_err());
    let mut wrong_callback = run.checkpoint();
    wrong_callback.token = Arc::new(TurnToken::new(Uuid::new_v4()));
    assert!(wrong_callback.partial("must not persist").await.is_err());
    drop(wrong_callback);
    drop(run);
    owner.recover_interrupted().await.unwrap().unwrap();
    let recovered = owner.snapshot().await.unwrap();
    assert!(
        owner
            .admit_at(next_request(&request, recovered.revision + 1), 1)
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
                wait.recv_timeout(Duration::from_secs(20)).unwrap();
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
            .admit_at(next_request(&request, before.revision), 1)
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
        .admit_at(next_request(&request, snapshot.revision), 1)
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

#[tokio::test]
async fn aborted_admission_keeps_fence_until_commit_and_never_dispatches_on_retry() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let session = Session::new(root.path().to_owned(), "fixture".into());
    let mut journal = Journal::open(path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    let owner = ManagedSessionOwner::open(path.clone(), session.id)
        .await
        .unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "accepted prompt after cancelled waiter".into(),
    };
    let mut retry = next_request(&request, 0);
    retry.command_id = request.command_id;
    retry.prompt = request.prompt.clone();
    let (started, ready) = tokio::sync::oneshot::channel();
    let (release, wait) = std::sync::mpsc::channel();
    let lifetime = Arc::downgrade(&owner.store);
    let admitting = owner.clone();
    let task = tokio::spawn(async move {
        admitting
            .admit_after(request, Arc::new(|| Ok(1)), move || {
                let _ = started.send(());
                wait.recv_timeout(Duration::from_secs(20)).unwrap();
                Ok(())
            })
            .await
    });
    ready.await.unwrap();
    task.abort();
    let _ = task.await;
    drop(owner);
    process_probe(&path, session.id, true); // Only the blocking admission worker owns the fence.
    assert!(journal.lookup_command(&retry).unwrap().is_none());
    release.send(()).unwrap();
    // Opening another Journal starts a transaction, so wait without touching SQLite
    // until the abandoned worker has completed its commit and dropped its owner.
    tokio::time::timeout(Duration::from_secs(2), async {
        while lifetime.strong_count() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let recovered_owner = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(owner) = ManagedSessionOwner::open(path.clone(), session.id).await {
                break owner;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let durable = journal.lookup_command(&retry).unwrap().unwrap();
    assert_eq!(durable.state, RunState::Accepted);
    let snapshot = recovered_owner.snapshot().await.unwrap();
    assert_eq!(snapshot.session.messages.len(), 1);
    assert_eq!(snapshot.session.messages[0].content, retry.prompt);
    assert_eq!(snapshot.session.usage.input_tokens, 0);
    assert!(
        recovered_owner
            .admit_at(next_request(&retry, snapshot.revision), 1)
            .await
            .is_err()
    );
    let next = next_request(&retry, snapshot.revision);
    assert!(
        matches!(recovered_owner.admit_at(retry, 90000).await.unwrap(), Admission::Existing(record) if record.id == durable.id && record.state == RunState::Accepted)
    );
    assert_eq!(
        recovered_owner
            .recover_interrupted()
            .await
            .unwrap()
            .unwrap()
            .state,
        RunState::Interrupted
    );
    let revision = recovered_owner.snapshot().await.unwrap().revision;
    assert!(matches!(
        recovered_owner
            .admit_at(next_request(&next, revision), 1)
            .await
            .unwrap(),
        Admission::New(_)
    ));
    assert_eq!(
        journal.run(durable.id).unwrap().state,
        RunState::Interrupted
    );
}

#[tokio::test]
async fn command_expiring_while_admission_waits_is_not_committed() {
    use std::sync::atomic::{AtomicI64, Ordering};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let session = Session::new(root.path().to_owned(), "fixture".into());
    let mut journal = Journal::open(path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    let owner = ManagedSessionOwner::open(path, session.id).await.unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 10,
        prompt: "must expire before acceptance".into(),
    };
    let mut retry = next_request(&request, 0);
    retry.command_id = request.command_id;
    retry.expires_at_ms = request.expires_at_ms;
    retry.prompt = request.prompt.clone();
    let clock = Arc::new(AtomicI64::new(1));
    let sampled = clock.clone();
    let result = owner
        .admit_after(
            request,
            Arc::new(move || Ok(sampled.load(Ordering::SeqCst))),
            move || {
                clock.store(10, Ordering::SeqCst);
                Ok(())
            },
        )
        .await;
    assert!(result.is_err(), "expired queued command was accepted");
    assert!(journal.lookup_command(&retry).unwrap().is_none());
    assert_eq!(journal.load_session(session.id).unwrap().revision, 0);
    assert!(
        journal
            .load_session(session.id)
            .unwrap()
            .session
            .messages
            .is_empty()
    );
}

fn clock_request(id: Uuid) -> TurnAdmission {
    TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
        session_id: id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "clock fixture".into(),
    }
}
fn copy_request(r: &TurnAdmission) -> TurnAdmission {
    TurnAdmission {
        command_id: r.command_id,
        machine_id: r.machine_id,
        principal_id: r.principal_id,
        session_id: r.session_id,
        expected_revision: r.expected_revision,
        expires_at_ms: r.expires_at_ms,
        prompt: r.prompt.clone(),
    }
}
#[tokio::test]
async fn failed_or_invalid_clocks_leave_no_admission_but_duplicates_remain_observable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let session = Session::new(root.path().to_owned(), "fixture".into());
    let mut journal = Journal::open(path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    let owner = ManagedSessionOwner::open(path.clone(), session.id)
        .await
        .unwrap();
    let request = clock_request(session.id);
    let failed: Arc<dyn RuntimeClock> = Arc::new(|| anyhow::bail!("clock unavailable"));
    for clock in [
        failed.clone(),
        Arc::new(|| Ok(-1_i64)) as Arc<dyn RuntimeClock>,
        Arc::new(|| Ok(i64::MAX)),
    ] {
        assert!(
            owner
                .admit_with_clock(copy_request(&request), clock)
                .await
                .is_err()
        );
        assert!(journal.lookup_command(&request).unwrap().is_none());
        assert_eq!(owner.snapshot().await.unwrap().revision, 0);
    }
    let Admission::New(run) = owner.admit_at(copy_request(&request), 1).await.unwrap() else {
        panic!("fresh admission")
    };
    let revision = owner.snapshot().await.unwrap().revision;
    assert!(
        matches!(owner.admit_with_clock(copy_request(&request), failed.clone()).await.unwrap(), Admission::Existing(record) if record.id == run.run_id)
    );
    assert!(
        matches!(RunOwner::admit_with_clock(path, request, failed).await.unwrap(), Admission::Existing(record) if record.id == run.run_id)
    );
    assert_eq!(owner.snapshot().await.unwrap().revision, revision);
}
#[test]
fn journal_samples_new_command_clock_inside_write_transaction_only() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let session = Session::new(root.path().to_owned(), "fixture".into());
    let mut journal = Journal::open(path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = clock_request(session.id);
    let other = fixture_database(path.join("journal.sqlite3")).unwrap();
    let admitted = journal
        .admit_turn_with_clock(&guard, &request, || {
            let result = other.execute_batch("BEGIN IMMEDIATE");
            if result.is_ok() {
                other.execute_batch("ROLLBACK")?;
                anyhow::bail!("clock sampled outside the admission transaction");
            }
            assert_eq!(
                result.unwrap_err().sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
            Ok(1)
        })
        .unwrap();
    assert!(!admitted.duplicate);
    assert!(
        journal
            .admit_turn_with_clock(&guard, &request, || panic!(
                "duplicate must not sample clock"
            ))
            .unwrap()
            .duplicate
    );
}

#[derive(Debug)]
struct ForegroundAuthority(std::sync::atomic::AtomicBool);
impl crate::policy::ExecutionAuthority for ForegroundAuthority {
    fn check(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.0.load(Ordering::SeqCst),
            "foreground authority revoked"
        );
        Ok(())
    }
}
#[tokio::test]
async fn foreground_authority_is_checked_before_existing_receipt_observation() {
    let (_root, run, _agent, _requests, _effects, request) =
        setup("normal", Arc::new(SilentSink)).await;
    let owner = managed(&run, &request);
    let authority = Arc::new(ForegroundAuthority(std::sync::atomic::AtomicBool::new(
        true,
    )));
    assert!(matches!(
        owner
            .admit_authorized(
                TurnAdmission {
                    prompt: request.prompt.clone(),
                    ..request
                },
                authority.clone()
            )
            .await
            .unwrap(),
        Admission::Existing(_)
    ));
    authority.0.store(false, Ordering::SeqCst);
    assert!(owner.admit_authorized(request, authority).await.is_err());
    assert_eq!(run.record().await.unwrap().state, RunState::Accepted);
    assert_eq!(owner.snapshot().await.unwrap().revision, 1);
}
#[tokio::test]
async fn foreground_authority_rechecked_after_admission_wait_before_any_durable_acceptance() {
    let (_root, run, _agent, _requests, _effects, request) =
        setup("normal", Arc::new(SilentSink)).await;
    let owner = managed(&run, &request);
    drop(run);
    owner.recover_interrupted().await.unwrap();
    let revision = owner.snapshot().await.unwrap().revision;
    let request = next_request(&request, revision);
    let authority = Arc::new(ForegroundAuthority(std::sync::atomic::AtomicBool::new(
        true,
    )));
    let revoke = authority.clone();
    assert!(
        owner
            .admit_authorized_after(
                request,
                Arc::new(|| Ok(1)),
                move || {
                    revoke.0.store(false, Ordering::SeqCst);
                    Ok(())
                },
                Some(authority)
            )
            .await
            .is_err()
    );
    assert_eq!(owner.snapshot().await.unwrap().revision, revision);
}

#[tokio::test]
async fn remote_cancel_clock_waits_for_owner_lock_and_duplicate_still_requires_authority() {
    use crate::attachment::journal::RemoteBinding;
    use std::sync::atomic::{AtomicI64, AtomicUsize};
    use voyage_protocol::attachment::{Command, Operation, VERSION};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("journal");
    let mut journal = Journal::open(path.clone()).unwrap();
    let session = Session::new(root.path().into(), "fixture".into());
    let binding = RemoteBinding {
        origin: "http://127.0.0.1:9480".into(),
        machine_id: Uuid::new_v4(),
        owner_id: Uuid::new_v4(),
        epoch: 1,
        local_installation_id: Uuid::new_v4(),
        local_principal_id: Uuid::new_v4(),
    };
    journal.create_remote_session(&session, &binding).unwrap();
    let owner = ManagedSessionOwner::open(path, session.id).await.unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "task".into(),
    };
    let Admission::New(run) = owner.admit_at(request, 1).await.unwrap() else {
        panic!("new run required")
    };
    let command = Command {
        version: VERSION,
        connection_id: Uuid::new_v4(),
        machine_id: binding.machine_id,
        principal_id: binding.owner_id,
        command_id: Uuid::new_v4(),
        expires_at_ms: 60000,
        operation: Operation::Cancel {
            session_id: session.id,
            run_id: run.run_id,
        },
    };
    let authority = Arc::new(ForegroundAuthority(std::sync::atomic::AtomicBool::new(
        true,
    )));
    let current = Arc::new(AtomicI64::new(1));
    let calls = Arc::new(AtomicUsize::new(0));
    let clock = {
        let current = current.clone();
        let calls = calls.clone();
        Arc::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(current.load(Ordering::SeqCst))
        })
    };
    let shared = owner.store.clone();
    let (entered, entry) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let locker = tokio::task::spawn_blocking(move || {
        let _guard = shared.lock().unwrap();
        entered.send(()).unwrap();
        released
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    });
    entry
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let count = Arc::strong_count(&owner.store);
    let task = {
        let owner = owner.clone();
        let binding = binding.clone();
        let command = command.clone();
        let authority = authority.clone();
        let clock = clock.clone();
        tokio::spawn(async move {
            owner
                .remote_cancel_clock(binding, command, authority, clock)
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while Arc::strong_count(&owner.store) < count + 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "clock sampled before the owned storage wait"
    );
    current.store(60000, Ordering::SeqCst);
    release.send(()).unwrap();
    locker.await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!owner.local_cancel_requested(run.run_id).await.unwrap());
    current.store(1, Ordering::SeqCst);
    assert!(
        !owner
            .remote_cancel_clock(binding.clone(), command.clone(), authority.clone(), clock)
            .await
            .unwrap()
            .duplicate
    );
    assert!(
        owner
            .remote_cancel_clock(
                binding.clone(),
                command.clone(),
                authority.clone(),
                Arc::new(|| panic!("duplicate sampled clock"))
            )
            .await
            .unwrap()
            .duplicate
    );
    authority.0.store(false, Ordering::SeqCst);
    assert!(
        owner
            .remote_cancel_clock(
                binding,
                command,
                authority,
                Arc::new(|| panic!("unauthorized sampled clock"))
            )
            .await
            .is_err()
    );
}
