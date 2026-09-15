use super::*;

fn create(name: &str) -> ProfileChange {
    ProfileChange {
        operation_id: Uuid::new_v4(),
        name: name.into(),
        expected_revision: 0,
        action: Action::Create {
            rules: Builtin::Balanced.document().rules,
        },
    }
}

#[test]
fn revisions_retries_and_recreation_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::open(dir.path()).unwrap();
    let change = create("release");
    let first = store.change(&change).unwrap();
    assert!(!first.duplicate);
    assert_eq!(first.sequence, 1);
    assert_eq!(first.snapshot.identity, change.operation_id);
    drop(store);
    let store = ProfileStore::open_existing(dir.path()).unwrap();
    let retry = store.change(&change).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.snapshot, first.snapshot);
    assert_eq!(retry.sequence, first.sequence);
    let mut conflicting = change.clone();
    conflicting.name = "other".into();
    assert_eq!(
        store.change(&conflicting).unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(
        store.change(&create("release")).unwrap_err(),
        StoreError::Conflict
    );
    let replace = ProfileChange {
        operation_id: Uuid::new_v4(),
        expected_revision: 1,
        action: Action::Replace {
            rules: Builtin::Restricted.document().rules,
        },
        ..change.clone()
    };
    let second = store.change(&replace).unwrap();
    assert_eq!(second.snapshot.identity, first.snapshot.identity);
    assert_eq!(second.snapshot.revision, 2);
    let document: ProfileDocument =
        serde_json::from_slice(&store.export("release").unwrap()).unwrap();
    assert_eq!(document.rules, Builtin::Restricted.document().rules);
    assert_eq!(document.revision, 2);
    let delete = ProfileChange {
        operation_id: Uuid::new_v4(),
        expected_revision: 2,
        action: Action::Delete {},
        ..change.clone()
    };
    let tombstone = store.change(&delete).unwrap().snapshot;
    assert_eq!(tombstone.identity, first.snapshot.identity);
    assert_eq!(tombstone.revision, 3);
    assert!(tombstone.rules.is_none());
    assert_eq!(store.export("release").unwrap_err(), StoreError::Conflict);
    let recreate = ProfileChange {
        expected_revision: 3,
        ..create("release")
    };
    let recreated = store.change(&recreate).unwrap().snapshot;
    assert_eq!(recreated.revision, 4);
    assert_eq!(recreated.identity, recreate.operation_id);
    assert_ne!(recreated.identity, first.snapshot.identity);
    // A historical receipt must not replace the current snapshot.
    assert_eq!(store.change(&change).unwrap().snapshot, first.snapshot);
    assert_eq!(store.inspect("release").unwrap(), Some(recreated));
}

#[test]
fn publication_failures_require_exact_retry_and_never_duplicate_history() {
    for boundary in [
        Boundary::CandidateWritten,
        Boundary::CandidateDurable,
        Boundary::BeforePublish,
        Boundary::Published,
        Boundary::DirectorySynced,
        Boundary::Verified,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(dir.path()).unwrap();
        let change = create("release");
        assert_eq!(
            store
                .commit_with(&change, |at| {
                    if at == boundary {
                        Err(StoreError::Storage)
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err(),
            StoreError::Storage
        );
        drop(store);
        let store = ProfileStore::open_existing(dir.path()).unwrap();
        let published = matches!(
            boundary,
            Boundary::Published | Boundary::DirectorySynced | Boundary::Verified
        );
        if !published {
            assert_eq!(store.inspect("release").unwrap_err(), StoreError::Pending);
            assert_eq!(store.list(None, 10).unwrap_err(), StoreError::Pending);
            assert_eq!(
                store.change(&create("other")).unwrap_err(),
                StoreError::Pending
            );
        }
        let receipt = store.change(&change).unwrap();
        assert_eq!(receipt.duplicate, published);
        assert_eq!(receipt.sequence, 1);
        assert_eq!(store.inspect("release").unwrap(), Some(receipt.snapshot));
        assert!(store.change(&change).unwrap().duplicate);
        assert!(!dir.path().join(record_name(2)).exists());
    }
}

#[test]
fn partial_candidate_and_corrupt_committed_evidence_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::open(dir.path()).unwrap();
    let change = create("release");
    assert_eq!(
        store
            .commit_with(&change, |at| {
                if at == Boundary::CandidateCreated {
                    Err(StoreError::Storage)
                } else {
                    Ok(())
                }
            })
            .unwrap_err(),
        StoreError::Storage
    );
    assert_eq!(store.change(&change).unwrap_err(), StoreError::Evidence);
    assert_eq!(store.inspect("release").unwrap_err(), StoreError::Evidence);

    for damaged in [HEADER.to_string(), record_name(1), witness_name(1)] {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(dir.path()).unwrap();
        store.change(&create("release")).unwrap();
        std::fs::write(dir.path().join(damaged), b"{}").unwrap();
        assert_eq!(store.inspect("release").unwrap_err(), StoreError::Evidence);
        assert_eq!(
            store.change(&create("other")).unwrap_err(),
            StoreError::Evidence
        );
    }
}

#[test]
fn missing_committed_record_or_witness_is_not_empty_history() {
    for missing in [record_name(1), witness_name(1)] {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(dir.path()).unwrap();
        let change = create("release");
        store.change(&change).unwrap();
        std::fs::remove_file(dir.path().join(missing)).unwrap();
        assert_eq!(store.inspect("release").unwrap_err(), StoreError::Evidence);
        assert_eq!(store.change(&change).unwrap_err(), StoreError::Evidence);
    }
}

#[test]
fn pagination_is_stable_and_builtins_cannot_be_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::open(dir.path()).unwrap();
    for name in ["z-last", "a-first"] {
        store.change(&create(name)).unwrap();
    }
    let mut names = Vec::new();
    let mut cursor = None;
    loop {
        let page = store.list(cursor.as_deref(), 2).unwrap();
        names.extend(page.profiles.into_iter().map(|p| p.name));
        cursor = page.next_after;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        names,
        ["a-first", "autonomous", "balanced", "restricted", "z-last"]
    );
    for name in ["autonomous", "balanced", "restricted"] {
        assert!(store.inspect(name).unwrap().unwrap().builtin);
        assert_eq!(
            store.change(&create(name)).unwrap_err(),
            StoreError::Invalid
        );
    }
    for limit in [0, 101] {
        assert_eq!(store.list(None, limit).unwrap_err(), StoreError::Invalid);
    }
    assert_eq!(
        store.list(Some("../bad"), 1).unwrap_err(),
        StoreError::Invalid
    );
    assert_eq!(store.inspect("../bad").unwrap_err(), StoreError::Invalid);
    assert!(store.inspect("missing").unwrap().is_none());
    assert_eq!(store.export("missing").unwrap_err(), StoreError::Conflict);
    let invalid = ProfileChange {
        operation_id: Uuid::nil(),
        ..create("release")
    };
    assert_eq!(store.change(&invalid).unwrap_err(), StoreError::Invalid);
    let absent = dir.path().join("absent");
    assert!(matches!(
        ProfileStore::open_existing(&absent),
        Err(StoreError::Storage)
    ));
    assert!(!absent.exists());
}

#[test]
fn matching_witness_does_not_make_forged_history_valid() {
    for field in [
        "version",
        "sequence",
        "previous_hash",
        "snapshot",
        "duplicate_operation",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::open(dir.path()).unwrap();
        let first = create("first");
        store.change(&first).unwrap();
        store.change(&create("second")).unwrap();
        let path = dir.path().join(record_name(2));
        let mut record: Record = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        match field {
            "version" => record.version = 2,
            "sequence" => record.sequence = 3,
            "previous_hash" => record.previous_hash = ZERO_HASH.into(),
            "snapshot" => record.snapshot.revision += 1,
            "duplicate_operation" => {
                record.change.operation_id = first.operation_id;
                record.snapshot.identity = first.operation_id;
            }
            _ => unreachable!(),
        }
        let bytes = encoded(&record).unwrap();
        std::fs::write(path, &bytes).unwrap();
        std::fs::write(
            dir.path().join(witness_name(2)),
            witness_bytes(2, &bytes).unwrap(),
        )
        .unwrap();
        assert_eq!(
            store.inspect("second").unwrap_err(),
            StoreError::Evidence,
            "{field}"
        );
        assert_eq!(
            store.change(&create("third")).unwrap_err(),
            StoreError::Evidence,
            "{field}"
        );
    }
}

#[cfg(unix)]
#[test]
fn busy_store_and_symlink_evidence_are_refused_without_external_writes() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let store = ProfileStore::open(dir.path()).unwrap();
    let change = create("release");
    let lock = store.directory.lock().unwrap();
    assert_eq!(store.change(&change).unwrap_err(), StoreError::Busy);
    assert_eq!(store.inspect("release").unwrap_err(), StoreError::Busy);
    drop(lock);
    store.change(&change).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), b"untouched").unwrap();
    let record = dir.path().join(record_name(1));
    std::fs::remove_file(&record).unwrap();
    symlink(outside.path(), &record).unwrap();
    assert_eq!(store.inspect("release").unwrap_err(), StoreError::Storage);
    assert_eq!(store.change(&change).unwrap_err(), StoreError::Storage);
    assert_eq!(std::fs::read(outside.path()).unwrap(), b"untouched");
}
