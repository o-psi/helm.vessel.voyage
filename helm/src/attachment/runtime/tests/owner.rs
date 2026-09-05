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
