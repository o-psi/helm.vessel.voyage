use super::*;
fn task(title: &str) -> NewTodo {
    NewTodo {
        title: title.into(),
        description: "fixture".into(),
        priority: Priority::Normal,
        order: None,
        assignees: BTreeSet::new(),
    }
}
#[tokio::test]
async fn dependency_blocking_evidence_and_archival_are_durable() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("todos.json");
    let scope = TodoScope::workspace(root.path().into());
    let store = TodoStore::new(path.clone(), scope.clone());
    let first = store.create(task("first")).await.unwrap();
    let second = store.create(task("second")).await.unwrap();
    store.add_dependency(second.id, first.id).await.unwrap();
    assert!(store.add_dependency(first.id, second.id).await.is_err());
    assert!(store.add_dependency(first.id, first.id).await.is_err());
    assert!(
        store
            .set_status(second.id, TodoStatus::InProgress)
            .await
            .is_err()
    );
    assert!(
        store
            .set_status(first.id, TodoStatus::Blocked)
            .await
            .is_err()
    );
    store
        .set_blockers(first.id, vec!["awaiting fixture".into()])
        .await
        .unwrap();
    assert!(
        store
            .set_status(first.id, TodoStatus::Completed)
            .await
            .is_err()
    );
    store.set_blockers(first.id, vec![]).await.unwrap();
    store
        .set_status(first.id, TodoStatus::Completed)
        .await
        .unwrap();
    store
        .set_status(second.id, TodoStatus::InProgress)
        .await
        .unwrap();
    for kind in [EntryKind::Note, EntryKind::Progress, EntryKind::Evidence] {
        store
            .append_note(
                second.id,
                kind,
                "observed fixture".into(),
                Some("tester".into()),
            )
            .await
            .unwrap();
    }
    let updated = store
        .edit(
            second.id,
            Some(" renamed ".into()),
            Some("details".into()),
            Some(Priority::High),
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "renamed");
    store
        .assign(second.id, BTreeSet::from(["worker".into()]))
        .await
        .unwrap();
    store.reorder(second.id, -5).await.unwrap();
    let reopened = TodoStore::new(path, scope).snapshot().await.unwrap();
    assert_eq!(reopened.ordered()[0].id, second.id);
    assert_eq!(reopened.items[&second.id].evidence.len(), 1);
    assert_eq!(reopened.items[&second.id].notes.len(), 1);
    store
        .set_status(second.id, TodoStatus::Completed)
        .await
        .unwrap();
    assert_eq!(store.clear_completed().await.unwrap(), 2);
    assert_eq!(store.clear_completed().await.unwrap(), 0);
    assert!(
        store
            .edit(second.id, Some("no".into()), None, None)
            .await
            .is_err()
    );
    assert!(store.remove(first.id).await.is_err());
    store.remove(second.id).await.unwrap();
    store.remove(first.id).await.unwrap();
    assert!(store.snapshot().await.unwrap().items.is_empty());
}
#[tokio::test]
async fn shared_handles_serialize_writes_and_corrupt_or_wrong_scope_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("todos.json");
    let scope = TodoScope::workspace(root.path().into());
    let a = TodoStore::new(path.clone(), scope.clone());
    let b = TodoStore::new(path.clone(), scope);
    let (x, y) = tokio::join!(a.create(task("one")), b.create(task("two")));
    assert_ne!(x.unwrap().id, y.unwrap().id);
    assert_eq!(a.snapshot().await.unwrap().items.len(), 2);
    assert!(a.create(task(" ")).await.is_err());
    assert!(a.archive(TodoId::new()).await.is_err());
    let wrong = TodoStore::new(
        path.clone(),
        TodoScope::session(root.path().into(), Uuid::new_v4()),
    );
    assert!(wrong.snapshot().await.is_err());
    std::fs::write(path, b"broken").unwrap();
    assert!(a.snapshot().await.is_err());
    assert!(a.create(task("not overwritten")).await.is_err());
}
