use super::*;
fn anchor(dir: &Path) -> DefaultsSource {
    DefaultsSource {
        directory: dir.to_owned(),
        store_id: Uuid::new_v4(),
    }
}
fn preference() -> DefaultsChange {
    DefaultsChange {
        operation_id: Uuid::new_v4(),
        key: DefaultKey::Preference {
            scope: DefaultScope::Global {},
        },
        expected_revision: 0,
        value: DefaultValue::Preference { profile: None },
    }
}
#[test]
fn revisions_receipts_and_history_survive_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let source = anchor(dir.path());
    let store = DefaultsStore::create(&source).unwrap();
    let first = preference();
    let receipt = store.change(&first).unwrap();
    assert!(!receipt.duplicate);
    drop(store);
    let store = DefaultsStore::open_existing(&source).unwrap();
    assert!(store.change(&first).unwrap().duplicate);
    assert_eq!(
        store.inspect(&first.key).unwrap(),
        Some(receipt.snapshot.clone())
    );
    let stale = DefaultsChange {
        operation_id: Uuid::new_v4(),
        ..first.clone()
    };
    assert_eq!(store.change(&stale).unwrap_err(), StoreError::Conflict);
    let second = DefaultsChange {
        operation_id: Uuid::new_v4(),
        expected_revision: 1,
        ..first.clone()
    };
    assert_eq!(store.change(&second).unwrap().snapshot.revision, 2);
    let conflicting = DefaultsChange {
        expected_revision: 2,
        ..first.clone()
    };
    assert_eq!(
        store.change(&conflicting).unwrap_err(),
        StoreError::Conflict
    );
    assert_eq!(store.change(&first).unwrap().snapshot, receipt.snapshot);
    assert_eq!(store.inspect(&first.key).unwrap().unwrap().revision, 2);
    let history = store.history().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].0, 1);
    assert_eq!(history[1].0, 2);
    let wrong = DefaultsSource {
        store_id: Uuid::new_v4(),
        ..source
    };
    assert!(DefaultsStore::open_existing(&wrong).is_err());
    assert!(DefaultsStore::create(&wrong).is_err());
}
#[test]
fn exact_retry_recovers_each_complete_publication_boundary() {
    for boundary in [
        Boundary::CandidateWritten,
        Boundary::CandidateDurable,
        Boundary::BeforePublish,
        Boundary::Published,
        Boundary::DirectorySynced,
        Boundary::Verified,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let source = anchor(dir.path());
        let store = DefaultsStore::create(&source).unwrap();
        let change = preference();
        assert_eq!(
            store
                .commit_with(&change, |at| if at == boundary {
                    Err(StoreError::Storage)
                } else {
                    Ok(())
                })
                .unwrap_err(),
            StoreError::Storage
        );
        drop(store);
        let store = DefaultsStore::open_existing(&source).unwrap();
        let published = matches!(
            boundary,
            Boundary::Published | Boundary::DirectorySynced | Boundary::Verified
        );
        if !published {
            assert_eq!(store.history().unwrap_err(), StoreError::Pending);
            assert_eq!(store.inspect(&change.key).unwrap_err(), StoreError::Pending);
            assert_eq!(store.list(None, 1).unwrap_err(), StoreError::Pending);
            assert_eq!(
                store.change(&preference()).unwrap_err(),
                StoreError::Pending
            );
        }
        let receipt = store.change(&change).unwrap();
        assert_eq!(receipt.duplicate, published);
        assert_eq!(receipt.sequence, 1);
        assert!(store.change(&change).unwrap().duplicate);
        assert_eq!(store.history().unwrap().len(), 1);
    }
}
#[test]
fn corrupt_or_missing_evidence_never_becomes_empty_history() {
    for name in [HEADER.to_string(), record_name(1), witness_name(1)] {
        for remove in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let source = anchor(dir.path());
            let store = DefaultsStore::create(&source).unwrap();
            let change = preference();
            store.change(&change).unwrap();
            let path = dir.path().join(&name);
            if remove {
                std::fs::remove_file(path).unwrap();
            } else {
                std::fs::write(path, b"{}").unwrap();
            }
            assert_eq!(
                store.inspect(&change.key).unwrap_err(),
                StoreError::Evidence
            );
            assert_eq!(store.change(&change).unwrap_err(), StoreError::Evidence);
        }
    }
}
#[test]
fn activation_and_preferences_are_distinct_paginated_keys() {
    let dir = tempfile::tempdir().unwrap();
    let store = DefaultsStore::create(&anchor(dir.path())).unwrap();
    let mut changes = vec![preference()];
    changes.push(DefaultsChange {
        key: DefaultKey::Preference {
            scope: DefaultScope::Workspace {
                workspace: dir.path().to_owned(),
            },
        },
        ..preference()
    });
    let mut effective = crate::policy_profile::Builtin::Restricted.document().rules;
    effective.read_roots = vec![dir.path().to_str().unwrap().into()];
    effective.write_roots = effective.read_roots.clone();
    changes.push(DefaultsChange {
        key: DefaultKey::Activation {
            workspace: dir.path().to_owned(),
        },
        value: DefaultValue::Activation {
            candidate_digest: "a".repeat(64),
            transition_digest: "b".repeat(64),
            context_digest: "c".repeat(64),
            effective,
        },
        ..preference()
    });
    for change in &changes {
        store.change(change).unwrap();
    }
    let mut seen = Vec::new();
    let mut cursor = None;
    loop {
        let page = store.list(cursor.as_deref(), 1).unwrap();
        seen.extend(page.profiles.into_iter().map(|p| p.key.digest().unwrap()));
        cursor = page.next_after;
        if cursor.is_none() {
            break;
        }
    }
    let mut expected: Vec<_> = changes.iter().map(|c| c.key.digest().unwrap()).collect();
    expected.sort();
    assert_eq!(seen, expected);
    for limit in [0, 101] {
        assert_eq!(store.list(None, limit).unwrap_err(), StoreError::Invalid);
    }
    assert_eq!(
        store.list(Some("invalid"), 1).unwrap_err(),
        StoreError::Invalid
    );
    let invalid = DefaultsChange {
        value: changes[2].value.clone(),
        ..preference()
    };
    assert_eq!(store.change(&invalid).unwrap_err(), StoreError::Invalid);
    let invalid = DefaultsChange {
        operation_id: Uuid::nil(),
        ..preference()
    };
    assert_eq!(store.change(&invalid).unwrap_err(), StoreError::Invalid);
    let invalid = DefaultsChange {
        key: DefaultKey::Preference {
            scope: DefaultScope::Workspace {
                workspace: "relative".into(),
            },
        },
        ..preference()
    };
    assert_eq!(store.change(&invalid).unwrap_err(), StoreError::Invalid);
}
