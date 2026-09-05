use super::*;
use crate::policy_profile::Builtin;
use tempfile::TempDir;
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
fn lifecycle_cas_tombstone_recreation_and_immutable_retry() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::open(&temp.path().join("profiles")).unwrap();
    let original = create("review");
    let first = store.change(&original).unwrap();
    assert_eq!(first.snapshot.revision, 1);
    let mut edit = original.clone();
    edit.operation_id = Uuid::new_v4();
    edit.expected_revision = 1;
    edit.action = Action::Replace {
        rules: Builtin::Restricted.document().rules,
    };
    let second = store.change(&edit).unwrap();
    assert_eq!(second.snapshot.identity, first.snapshot.identity);
    assert!(
        store
            .change(&ProfileChange {
                operation_id: Uuid::new_v4(),
                ..edit.clone()
            })
            .is_err()
    );
    let delete = ProfileChange {
        operation_id: Uuid::new_v4(),
        name: "review".into(),
        expected_revision: 2,
        action: Action::Delete {},
    };
    assert!(store.change(&delete).unwrap().snapshot.rules.is_none());
    let mut recreated = create("review");
    recreated.expected_revision = 3;
    let fourth = store.change(&recreated).unwrap();
    assert_eq!(fourth.snapshot.revision, 4);
    assert_ne!(fourth.snapshot.identity, first.snapshot.identity);
    let retry = store.change(&original).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.snapshot, first.snapshot);
    assert_eq!(store.inspect("review").unwrap().unwrap(), fourth.snapshot);
    drop(store);
    assert_eq!(
        ProfileStore::open(&temp.path().join("profiles"))
            .unwrap()
            .inspect("review")
            .unwrap()
            .unwrap(),
        fourth.snapshot
    );
}
#[test]
fn builtin_cannot_be_replaced_and_import_is_inert() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::open(&temp.path().join("profiles")).unwrap();
    for builtin in ["restricted", "balanced", "autonomous"] {
        assert!(store.change(&create(builtin)).is_err());
    }
    let exported = Builtin::Autonomous.document().encode().unwrap();
    let document = ProfileDocument::decode(&exported).unwrap();
    assert!(store.inspect("imported").unwrap().is_none());
    let mut request = create("imported");
    request.action = Action::Create {
        rules: document.rules,
    };
    store.change(&request).unwrap();
    assert_eq!(store.list(None, 100).unwrap().profiles.len(), 4);
    assert!(store.list(None, 0).is_err());
}
#[test]
fn every_publication_boundary_is_fail_closed_and_exact_retry_recovers() {
    for failure in [
        Boundary::CandidateCreated,
        Boundary::CandidateWritten,
        Boundary::CandidateDurable,
        Boundary::BeforePublish,
        Boundary::Published,
        Boundary::DirectorySynced,
        Boundary::Verified,
    ] {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("profiles");
        let store = ProfileStore::open(&path).unwrap();
        let change = create("review");
        assert!(
            store
                .commit_with(&change, |boundary| if boundary == failure {
                    Err(StoreError::Storage)
                } else {
                    Ok(())
                })
                .is_err()
        );
        drop(store);
        let store = ProfileStore::open(&path).unwrap();
        if failure == Boundary::CandidateCreated {
            assert!(store.inspect("review").is_err());
            assert!(store.change(&change).is_err()); // partial evidence is never guessed or discarded
        } else {
            if matches!(
                failure,
                Boundary::CandidateWritten | Boundary::CandidateDurable | Boundary::BeforePublish
            ) {
                assert!(matches!(store.inspect("review"), Err(StoreError::Pending)));
            }
            let receipt = store.change(&change).unwrap();
            assert_eq!(receipt.snapshot.revision, 1);
            assert_eq!(store.inspect("review").unwrap().unwrap(), receipt.snapshot);
        }
    }
}
#[test]
fn historical_retry_does_not_activate_unrelated_pending_edit() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::open(&temp.path().join("profiles")).unwrap();
    let first = create("review");
    store.change(&first).unwrap();
    let edit = ProfileChange {
        operation_id: Uuid::new_v4(),
        name: "review".into(),
        expected_revision: 1,
        action: Action::Replace {
            rules: Builtin::Autonomous.document().rules,
        },
    };
    assert!(
        store
            .commit_with(&edit, |b| if b == Boundary::BeforePublish {
                Err(StoreError::Storage)
            } else {
                Ok(())
            })
            .is_err()
    );
    assert!(store.change(&first).unwrap().duplicate);
    assert!(matches!(store.inspect("review"), Err(StoreError::Pending)));
    assert!(matches!(
        store.change(&create("other")),
        Err(StoreError::Pending)
    ));
    store.change(&edit).unwrap();
    assert_eq!(store.inspect("review").unwrap().unwrap().revision, 2);
}
#[test]
fn missing_record_witness_mismatch_gap_and_unknown_fields_refuse_old_profile() {
    for mutation in 0..4 {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("profiles");
        let store = ProfileStore::open(&path).unwrap();
        let first = create("review");
        store.change(&first).unwrap();
        let edit = ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 1,
            action: Action::Delete {},
        };
        store.change(&edit).unwrap();
        match mutation {
            0 => std::fs::remove_file(path.join(record_name(2))).unwrap(),
            1 => std::fs::write(path.join(witness_name(2)), b"{}").unwrap(),
            2 => {
                std::fs::remove_file(path.join(record_name(1))).unwrap();
                std::fs::remove_file(path.join(witness_name(1))).unwrap();
            }
            _ => {
                let file = path.join(record_name(2));
                let mut value: serde_json::Value =
                    serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                value["future"] = true.into();
                std::fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
            }
        }
        assert!(store.inspect("review").is_err());
        assert!(store.list(None, 100).is_err());
    }
}
#[test]
fn operation_collision_invalid_names_and_bounded_records_are_rejected() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::open(&temp.path().join("profiles")).unwrap();
    let first = create("review");
    store.change(&first).unwrap();
    let mut wrong = first.clone();
    wrong.name = "different".into();
    assert!(matches!(store.change(&wrong), Err(StoreError::Conflict)));
    for name in ["", "../escape", "a\nsecret", ".", ".."] {
        assert!(store.change(&create(name)).is_err());
    }
    let mut huge = create("large");
    if let Action::Create { rules } = &mut huge.action {
        rules.inherit_env = vec!["X".repeat(128); 129];
    }
    assert!(store.change(&huge).is_err());
    let firstpage = store.list(None, 2).unwrap();
    let second = store.list(firstpage.next_after.as_deref(), 2).unwrap();
    assert_eq!(firstpage.profiles.len() + second.profiles.len(), 4);
}
#[cfg(unix)]
#[test]
fn private_lock_symlink_and_extra_link_refuse_unsafe_paths() {
    use std::os::unix::fs::symlink;
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("profiles");
    let store = ProfileStore::open(&path).unwrap();
    let guard = store.directory.lock().unwrap();
    assert!(matches!(
        store.change(&create("review")),
        Err(StoreError::Busy)
    ));
    drop(guard);
    store.change(&create("review")).unwrap();
    std::fs::hard_link(path.join(record_name(1)), temp.path().join("extra")).unwrap();
    assert!(store.inspect("review").is_err());
    std::fs::remove_file(temp.path().join("extra")).unwrap();
    let bytes = std::fs::read(path.join(record_name(1))).unwrap();
    std::fs::remove_file(path.join(record_name(1))).unwrap();
    std::fs::write(temp.path().join("elsewhere"), bytes).unwrap();
    symlink(temp.path().join("elsewhere"), path.join(record_name(1))).unwrap();
    assert!(store.inspect("review").is_err());
    symlink(&path, temp.path().join("linked")).unwrap();
    assert!(ProfileStore::open(&temp.path().join("linked")).is_err());
}
#[test]
fn history_capacity_is_bounded_and_does_not_prevent_exact_retry() {
    let temp = TempDir::new().unwrap();
    let store = ProfileStore::open(&temp.path().join("profiles")).unwrap();
    let first = create("review");
    let mut last = None;
    let mut previous_hash = ZERO_HASH.to_owned();
    for sequence in 1..=MAX_RECORDS {
        let change = if sequence == 1 {
            first.clone()
        } else {
            ProfileChange {
                operation_id: Uuid::new_v4(),
                name: "review".into(),
                expected_revision: (sequence - 1) as u64,
                action: Action::Replace {
                    rules: Builtin::Restricted.document().rules,
                },
            }
        };
        let snapshot = proposal(&change, last.as_ref()).unwrap();
        let record = Record {
            version: 1,
            sequence: sequence as u64,
            previous_hash,
            change,
            snapshot: snapshot.clone(),
        };
        let bytes = encoded(&record).unwrap();
        store
            .directory
            .create(&record_name(sequence))
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        store
            .directory
            .create(&witness_name(sequence))
            .unwrap()
            .write_all(&witness_bytes(sequence as u64, &bytes).unwrap())
            .unwrap();
        previous_hash = digest(&bytes);
        last = Some(snapshot);
    }
    assert!(matches!(
        store.change(&create("new")),
        Err(StoreError::Capacity)
    ));
    assert!(store.change(&first).unwrap().duplicate);
}
#[test]
fn concurrent_private_open_handles_allow_only_one_cas_winner() {
    use std::sync::{Arc, Barrier};
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("profiles");
    let first = ProfileStore::open(&path).unwrap();
    let second = ProfileStore::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let other = barrier.clone();
    let a = std::thread::spawn(move || {
        other.wait();
        first.change(&create("review"))
    });
    let b = std::thread::spawn(move || {
        barrier.wait();
        second.change(&create("review"))
    });
    let results = [a.join().unwrap(), b.join().unwrap()];
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        ProfileStore::open(&path)
            .unwrap()
            .inspect("review")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
}
#[test]
fn initialized_shared_readers_coexist_and_exclude_mutation_without_bootstrap() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("profiles");
    assert!(ProfileStore::open_existing(&path).is_err());
    assert!(!path.exists());
    let store = ProfileStore::open(&path).unwrap();
    store.change(&create("review")).unwrap();
    let reader = store.directory.read_lock().unwrap();
    let another = ProfileStore::open_existing(&path).unwrap();
    assert_eq!(another.inspect("review").unwrap().unwrap().revision, 1);
    assert!(matches!(
        another.change(&create("other")),
        Err(StoreError::Busy)
    ));
    drop(reader);
    another.change(&create("other")).unwrap();
    let writer = store.directory.lock().unwrap();
    assert!(matches!(another.inspect("review"), Err(StoreError::Busy)));
    drop(writer);
    std::fs::remove_file(path.join("actor.lock")).unwrap();
    assert!(ProfileStore::open_existing(&path).is_err());
    assert!(!path.join("actor.lock").exists());
}
