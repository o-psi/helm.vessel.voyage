use super::*;
#[tokio::test]
async fn session_store_revision_leases_branch_and_export_preserve_canonical_data() {
    let root = tempfile::tempdir().unwrap();
    let store = SessionStore::new(root.path().join("sessions"));
    let mut session = Session::new(root.path().into(), "fixture".into());
    session
        .messages
        .push(Message::new(crate::model::Role::User, "canonical prompt"));
    session.draft = "private unsent draft".into();
    store.save(&mut session).await.unwrap();
    let mut stale = store.load(session.id).await.unwrap();
    session.set_name("renamed".into());
    store.save(&mut session).await.unwrap();
    stale.set_name("stale update".into());
    assert!(store.save(&mut stale).await.is_err());
    assert_eq!(
        store
            .load_reference(&session.id.to_string())
            .await
            .unwrap()
            .display_name(),
        "renamed"
    );
    let lease = store.acquire_execution(session.id).await.unwrap();
    assert!(store.acquire_execution(session.id).await.is_err());
    store.save_with_lease(&mut session, &lease).await.unwrap();
    drop(lease);
    let branched = store.branch(&session, Some("branch".into())).await.unwrap();
    assert_ne!(branched.id, session.id);
    assert_eq!(branched.parent_id, Some(session.id));
    assert_eq!(branched.messages.len(), session.messages.len());
    let export = root.path().join("export.md");
    store.export_markdown(&session, &export).await.unwrap();
    let text = std::fs::read_to_string(export).unwrap();
    assert!(text.contains("canonical prompt"));
    assert!(!text.contains("private unsent draft"));
    assert_eq!(store.list().await.unwrap().len(), 2);
    store.delete(branched.id).await.unwrap();
    assert_eq!(store.list().await.unwrap().len(), 1);
}
#[tokio::test]
async fn malformed_or_redirected_session_files_are_preserved_and_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("sessions");
    let store = SessionStore::new(dir.clone());
    let mut session = Session::new(root.path().into(), "fixture".into());
    store.save(&mut session).await.unwrap();
    let path = dir.join(format!("{}.json", session.id));
    std::fs::write(&path, b"invalid session").unwrap();
    assert!(store.load(session.id).await.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"invalid session");
    assert!(store.load_reference("../escape").await.is_err());
    #[cfg(unix)]
    {
        std::fs::remove_file(&path).unwrap();
        let target = root.path().join("outside");
        std::fs::write(&target, b"untouched").unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(store.load(session.id).await.is_err());
        assert!(store.delete(session.id).await.is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"untouched");
    }
}
