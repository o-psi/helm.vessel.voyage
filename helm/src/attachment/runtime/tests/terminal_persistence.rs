use super::*;

/// A real external rollback-journal reader holds COMMIT away from the owner.
/// This is terminal persistence only: the model/tool work has already finished.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_terminal_reader_does_not_leave_cancelled_run_running() {
    terminal_contention("BEGIN; SELECT * FROM runs;", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_terminal_writer_does_not_leave_cancelled_run_running() {
    terminal_contention("BEGIN IMMEDIATE", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transient_terminal_exclusive_lock_retries_preflight() {
    terminal_contention("BEGIN EXCLUSIVE", false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_retry_resamples_in_memory_cancellation() {
    terminal_contention("BEGIN; SELECT * FROM runs;", true).await;
}

async fn terminal_contention(lock_sql: &str, token_only: bool) {
    let cancel = CancellationToken::new();
    let (dir, mut owner, agent, requests, effects, request) =
        setup("success", Arc::new(SilentSink)).await;
    owner.register_local_cleanup().await.unwrap();
    let run_id = owner.record().await.unwrap().id;
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let path = dir.path().join("attachment/journal.sqlite3");
    let mut reader = None;
    let retry_ready = Arc::new(tokio::sync::Notify::new());
    let (retry_release_tx, retry_release_rx) = std::sync::mpsc::channel();
    if token_only {
        let ready = retry_ready.clone();
        let release = std::sync::Mutex::new(retry_release_rx);
        owner.terminal_retry_hook = Some(Arc::new(move || {
            ready.notify_one();
            release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }));
    }
    let mut execution = Box::pin(
        owner.execute_before_finish(&agent, cancel.clone(), None, || {
            let mut journal = Journal::open(dir.path().join("attachment")).unwrap();
            if !token_only {
                journal
                    .request_cancel_local(&super::super::super::journal::LocalCancelRequest {
                        session_id: request.session_id,
                        run_id,
                        installation_id: request.machine_id,
                        principal_id: request.principal_id,
                        expires_at_ms: SystemClock.now_ms().unwrap() + 60000,
                    })
                    .unwrap();
            }
            let db = fixture_database(&path).unwrap();
            db.execute_batch(lock_sql).unwrap();
            reader = Some(std::thread::spawn(move || {
                ready_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                db.execute_batch("ROLLBACK").unwrap();
            }));
            Ok(())
        }),
    );
    let early = if token_only {
        tokio::select! {
            biased;
            _ = retry_ready.notified() => None,
            result = &mut execution => Some(result),
        }
    } else {
        tokio::select! {
            biased;
            ready = ready_rx => {
                ready.unwrap();
                tokio::time::timeout(Duration::from_millis(150), &mut execution).await.ok()
            },
            result = &mut execution => Some(result),
        }
    };
    // Token cancellation happens after a confirmed rolled-back BUSY, while the
    // retry worker is paused. Releasing only after cancellation guarantees the
    // next attempt samples it; a timeout cannot establish that ordering.
    // An early failure still releases the real reader and reports durable state.
    if token_only {
        cancel.cancel();
    }
    let _ = release_tx.send(());
    let _ = retry_release_tx.send(());
    let result = match early {
        Some(result) => result,
        None => execution.as_mut().await,
    };
    drop(execution);
    reader.take().unwrap().join().unwrap();
    let record = owner.record().await.unwrap();
    assert!(
        matches!(result, Err(AgentError::Cancelled)) && record.state == RunState::Cancelled,
        "terminal-only commit must settle cancellation after reader release: result={result:?}, record={record:?}"
    );
    owner.confirm_local_cleanup_observed().await.unwrap();
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_busy_deadline_preserves_running_and_unconfirmed_cleanup() {
    let (dir, mut owner, agent, requests, effects, _) =
        setup("success", Arc::new(SilentSink)).await;
    owner.register_local_cleanup().await.unwrap();
    let mut reader = None;
    let started = std::time::Instant::now();
    let result = owner
        .execute_before_finish(&agent, CancellationToken::new(), None, || {
            let db = fixture_database(dir.path().join("attachment/journal.sqlite3")).unwrap();
            db.execute_batch("BEGIN; SELECT * FROM runs").unwrap();
            reader = Some(db);
            Ok(())
        })
        .await;
    assert!(matches!(result, Err(AgentError::Checkpoint(_))));
    assert!(started.elapsed() >= Duration::from_secs(2));
    assert!(started.elapsed() < Duration::from_secs(5));
    reader.take().unwrap().execute_batch("ROLLBACK").unwrap();
    assert_eq!(owner.record().await.unwrap().state, RunState::Running);
    assert!(owner.confirm_local_cleanup_observed().await.is_err());
    assert_eq!(requests.load(Ordering::SeqCst), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}
