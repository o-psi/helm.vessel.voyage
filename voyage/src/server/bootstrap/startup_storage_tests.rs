//! Ordinary startup waits for SQLite statements without retrying admission.
use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn fixture() -> (tempfile::TempDir, PathBuf, ProcessRegistration) {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("journal");
    drop(Journal::open(directory.clone()).unwrap());
    let registration = ProcessRegistration {
        executable: None,
        protocol: voyage_protocol::process::PROCESS_PROTOCOL,
        session_id: Uuid::new_v4(),
        incarnation: Uuid::new_v4(),
        command_id: Uuid::new_v4(),
        restart_from: None,
        initialize: None,
        config_path: None,
        token: "startup-contention-fixture".into(),
        peer_uids: None,
        workspace: root.path().into(),
        state: voyage_protocol::process::ProcessState::Starting,
        name: None,
    };
    (root, directory, registration)
}

#[tokio::test]
async fn startup_session_waits_for_writer_once_without_blocking_reactor() {
    let (_root, directory, registration) = fixture();
    let writer = rusqlite::Connection::open(directory.join("journal.sqlite3")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let target = directory.clone();
    let requested = registration.clone();
    let task = tokio::spawn(Journal::blocking_checkpoint(move || {
        observed.fetch_add(1, Ordering::SeqCst);
        prepare_session(
            target,
            &requested,
            requested.workspace.clone(),
            "fixture".into(),
        )
    }));
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    writer.execute_batch("ROLLBACK").unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let journal = Journal::open(directory.clone()).unwrap();
    let saved = journal.load_session(registration.session_id).unwrap();
    assert_eq!(saved.revision, 0);
    assert!(saved.session.messages.is_empty());
    assert!(
        journal
            .process_latest_run(registration.session_id)
            .unwrap()
            .is_none()
    );
    drop(journal);

    // Owner establishment uses the same bounded open, retaining OS exclusion.
    let owner = suspended::open_owner(directory.clone(), registration.session_id)
        .await
        .unwrap();
    let writer = rusqlite::Connection::open(directory.join("journal.sqlite3")).unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let incarnation = registration.incarnation;
    let task = tokio::spawn(async move {
        owner.bind_notification_incarnation(incarnation).await?;
        Ok::<_, anyhow::Error>(owner)
    });
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    assert!(!task.is_finished());
    writer.execute_batch("ROLLBACK").unwrap();
    let owner = tokio::time::timeout(std::time::Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let journal = Journal::open(directory).unwrap();
    assert!(journal.acquire_execution(registration.session_id).is_err());
    let saved = owner.snapshot().await.unwrap();
    assert_eq!(saved.revision, 0);
    assert!(saved.session.messages.is_empty());
}

#[tokio::test]
async fn startup_contention_expires_without_session_admission_or_late_replay() {
    let (_root, directory, registration) = fixture();
    let writer = rusqlite::Connection::open(directory.join("journal.sqlite3")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = attempts.clone();
    let target = directory.clone();
    let requested = registration.clone();
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        Journal::blocking_checkpoint(move || {
            observed.fetch_add(1, Ordering::SeqCst);
            prepare_session(
                target,
                &requested,
                requested.workspace.clone(),
                "fixture".into(),
            )
        }),
    )
    .await
    .unwrap();
    let error = result.unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<rusqlite::Error>()
            .unwrap()
            .sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    writer.execute_batch("ROLLBACK").unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    let journal = Journal::open(directory).unwrap();
    assert!(matches!(
        journal
            .load_session(registration.session_id)
            .err()
            .unwrap()
            .downcast_ref::<rusqlite::Error>(),
        Some(rusqlite::Error::QueryReturnedNoRows)
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
