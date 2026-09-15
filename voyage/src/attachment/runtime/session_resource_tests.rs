use super::*;
use rusqlite::Connection;
use std::time::{Duration, Instant};

async fn fixture() -> (tempfile::TempDir, ManagedSessionOwner, RunOwner, Connection) {
    let (root, owner, run) = super::checkpoint_tests::fixture_authorized(None).await;
    owner.initialize_session_resources().await.unwrap();
    let db = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    db.busy_timeout(Duration::ZERO).unwrap();
    (root, owner, run, db)
}

fn counts(db: &Connection, id: Uuid) -> (i64, i64) {
    let id = id.to_string();
    (
        db.query_row("SELECT count(*) FROM process_session_resources WHERE id=?1", [&id], |r| r.get(0)).unwrap(),
        db.query_row("SELECT count(*) FROM process_observations WHERE kind='session_resource' AND entity_id=?1", [&id], |r| r.get(0)).unwrap(),
    )
}

// Wait for the worker to hold the owner mutex (not just for task scheduling).
async fn worker_started(owner: &ManagedSessionOwner) {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if owner.store.try_lock().is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn brief(reader: bool, close: bool) {
    let (_root, owner, run, db) = fixture().await;
    let id = Uuid::new_v4();
    let run_id = run.record().await.unwrap().id;
    if close {
        owner
            .session_resource_adopt(id, run_id, "root_terminals".into())
            .await
            .unwrap();
    }
    db.execute_batch(if reader {
        "BEGIN; SELECT * FROM process_session_resources;"
    } else {
        "BEGIN IMMEDIATE;"
    })
    .unwrap();
    let worker = owner.clone();
    let task = tokio::spawn(async move {
        if close {
            worker.session_resource_closed(id).await
        } else {
            worker
                .session_resource_adopt(id, run_id, "root_terminals".into())
                .await
        }
    });
    worker_started(&owner).await;
    if reader {
        // A pending COMMIT excludes new readers: observe that exact lock state
        // before releasing the original reader, rather than guessing a delay.
        let probe = Connection::open(_root.path().join("journal/journal.sqlite3")).unwrap();
        probe.busy_timeout(Duration::ZERO).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if probe
                    .query_row("SELECT count(*) FROM process_session_resources", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .is_err()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    assert!(!task.is_finished());
    db.execute_batch("ROLLBACK").unwrap();
    task.await.unwrap().unwrap();
    assert_eq!(counts(&db, id), (1, if close { 2 } else { 1 }));
}

#[tokio::test]
async fn session_resource_brief_writer_and_reader_commit_contention() {
    for reader in [false, true] {
        for close in [false, true] {
            brief(reader, close).await;
        }
    }
}

#[tokio::test]
async fn session_resource_timeout_rolls_back_and_restores_nonblocking() {
    for close in [false, true] {
        let (_root, owner, run, db) = fixture().await;
        let id = Uuid::new_v4();
        let run_id = run.record().await.unwrap().id;
        if close {
            owner
                .session_resource_adopt(id, run_id, "root_terminals".into())
                .await
                .unwrap();
        }
        db.execute_batch("BEGIN; SELECT * FROM process_session_resources;")
            .unwrap();
        let start = Instant::now();
        let result = if close {
            owner.session_resource_closed(id).await
        } else {
            owner
                .session_resource_adopt(id, run_id, "root_terminals".into())
                .await
        };
        assert!(result.is_err());
        assert!(start.elapsed() >= Duration::from_millis(1800));
        assert!(start.elapsed() < Duration::from_secs(5));
        db.execute_batch("ROLLBACK").unwrap();
        assert_eq!(counts(&db, id), if close { (1, 1) } else { (0, 0) });
        // A fresh Wait budget would make a leaked handler wait two seconds.
        // Check the connection directly while another writer holds its lock.
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        {
            let mut store = owner.store.lock().unwrap();
            let _budget = super::super::journal::checkpoint_wait::Wait::new(None, None);
            let start = Instant::now();
            let Store { journal, guard, .. } = &mut *store;
            assert!(
                journal
                    .session_resource_adopt(guard, Uuid::new_v4(), run_id, "probe")
                    .is_err()
            );
            assert!(start.elapsed() < Duration::from_millis(500));
        }
        db.execute_batch("ROLLBACK").unwrap();
        if close {
            owner.session_resource_closed(id).await.unwrap();
        } else {
            owner
                .session_resource_adopt(id, run_id, "root_terminals".into())
                .await
                .unwrap();
        }
        assert_eq!(counts(&db, id), (1, if close { 2 } else { 1 }));
    }
}

#[tokio::test]
async fn session_resource_bookkeeping_survives_cancel_and_revocation() {
    #[derive(Debug)]
    struct Revoked;
    impl crate::policy::ExecutionAuthority for Revoked {
        fn check(&self) -> anyhow::Result<()> {
            anyhow::bail!("revoked fixture")
        }
    }
    // Admit with valid authority, then revoke before recording existing resources.
    #[derive(Debug)]
    struct Authority(AtomicBool);
    impl crate::policy::ExecutionAuthority for Authority {
        fn check(&self) -> anyhow::Result<()> {
            if self.0.load(Ordering::SeqCst) {
                Revoked.check()
            } else {
                Ok(())
            }
        }
    }
    let authority = Arc::new(Authority(AtomicBool::new(false)));
    let (root, owner, run) =
        super::checkpoint_tests::fixture_authorized(Some(authority.clone())).await;
    owner.initialize_session_resources().await.unwrap();
    let run_id = run.record().await.unwrap().id;
    let cancel = CancellationToken::new();
    cancel.cancel();
    run.token.checkpoint_cancel.set(cancel).unwrap();
    authority.0.store(true, Ordering::SeqCst);
    let db = Connection::open(root.path().join("journal/journal.sqlite3")).unwrap();
    let id = Uuid::new_v4();
    for close in [false, true] {
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        let worker = owner.clone();
        let task = tokio::spawn(async move {
            if close {
                worker.session_resource_closed(id).await
            } else {
                worker
                    .session_resource_adopt(id, run_id, "root_terminals".into())
                    .await
            }
        });
        worker_started(&owner).await;
        assert!(!task.is_finished());
        db.execute_batch("ROLLBACK").unwrap();
        task.await.unwrap().unwrap();
        // Verify restoration on success too, with a live callback budget.
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        {
            let mut store = owner.store.lock().unwrap();
            let _budget = super::super::journal::checkpoint_wait::Wait::new(None, None);
            let start = Instant::now();
            let Store { journal, guard, .. } = &mut *store;
            assert!(
                journal
                    .session_resource_adopt(guard, Uuid::new_v4(), run_id, "probe")
                    .is_err()
            );
            assert!(start.elapsed() < Duration::from_millis(500));
        }
        db.execute_batch("ROLLBACK").unwrap();
    }
    assert_eq!(counts(&db, id), (1, 2));
}
