use super::*;
use tempfile::TempDir;
fn setup() -> (TempDir, DefaultsSource, DefaultsStore, DefaultsChange) {
    let temp = TempDir::new().unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let change = DefaultsChange {
        operation_id: Uuid::new_v4(),
        key: DefaultKey::Preference {
            scope: DefaultScope::Global {},
        },
        expected_revision: 0,
        value: DefaultValue::Preference { profile: None },
    };
    (temp, anchor, store, change)
}
#[test]
fn publication_failures_do_not_activate_staged_intent() {
    for point in [
        Boundary::CandidateCreated,
        Boundary::CandidateWritten,
        Boundary::CandidateDurable,
        Boundary::BeforePublish,
        Boundary::Published,
        Boundary::DirectorySynced,
        Boundary::Verified,
    ] {
        let (_temp, anchor, store, change) = setup();
        assert!(
            store
                .commit_with(&change, |b| if b == point {
                    Err(StoreError::Storage)
                } else {
                    Ok(())
                })
                .is_err()
        );
        drop(store);
        let store = DefaultsStore::open_existing(&anchor).unwrap();
        if point == Boundary::CandidateCreated {
            assert!(store.inspect(&change.key).is_err());
            assert!(store.change(&change).is_err());
        } else {
            if matches!(
                point,
                Boundary::CandidateWritten | Boundary::CandidateDurable | Boundary::BeforePublish
            ) {
                assert!(store.inspect(&change.key).is_err());
            }
            assert_eq!(store.change(&change).unwrap().snapshot.revision, 1);
            assert_eq!(store.inspect(&change.key).unwrap().unwrap().revision, 1);
        }
    }
}
#[test]
fn missing_last_record_and_cross_store_identity_fail_closed() {
    let (_temp, anchor, store, change) = setup();
    store.change(&change).unwrap();
    std::fs::remove_file(anchor.directory.join(record_name(1))).unwrap();
    assert!(store.inspect(&change.key).is_err());
    assert!(store.change(&change).is_err());
}
#[test]
fn corrupt_witness_gap_symlink_and_extra_link_refuse_reads() {
    for mode in 0..5 {
        let (temp, anchor, store, change) = setup();
        store.change(&change).unwrap();
        let record = anchor.directory.join(record_name(1));
        match mode {
            0 => std::fs::write(anchor.directory.join(witness_name(1)), b"bad").unwrap(),
            1 => {
                let bytes = std::fs::read(&record).unwrap();
                std::fs::remove_file(&record).unwrap();
                std::fs::write(anchor.directory.join(record_name(2)), bytes).unwrap();
            }
            2 => std::fs::hard_link(&record, temp.path().join("extra-link")).unwrap(),
            3 => {
                #[cfg(unix)]
                {
                    std::fs::remove_file(&record).unwrap();
                    std::os::unix::fs::symlink(temp.path().join("missing"), &record).unwrap();
                }
            }
            4 => std::fs::write(anchor.directory.join(HEADER), b"{\"version\":99}").unwrap(),
            _ => unreachable!(),
        }
        #[cfg(not(unix))]
        if mode == 3 {
            continue;
        }
        assert!(store.inspect(&change.key).is_err(), "mode {mode}");
        assert!(store.change(&change).is_err(), "mode {mode}");
    }
}
#[test]
fn shared_readers_exclude_writer_without_bootstrapping_or_rewriting() {
    let (_temp, anchor, store, change) = setup();
    store.change(&change).unwrap();
    let before = std::fs::read(anchor.directory.join(HEADER)).unwrap();
    let reader = store.directory.read_lock().unwrap();
    let another = DefaultsStore::open_existing(&anchor).unwrap();
    assert!(another.inspect(&change.key).unwrap().is_some());
    assert!(matches!(another.change(&change), Err(StoreError::Busy)));
    drop(reader);
    let writer = store.directory.lock().unwrap();
    assert!(matches!(
        another.inspect(&change.key),
        Err(StoreError::Busy)
    ));
    drop(writer);
    assert_eq!(
        before,
        std::fs::read(anchor.directory.join(HEADER)).unwrap()
    );
    assert!(another.change(&change).unwrap().duplicate);
}
#[test]
fn malformed_or_unbounded_requests_are_rejected_before_publication() {
    let (_temp, anchor, store, change) = setup();
    let invalid = [
        DefaultsChange {
            operation_id: Uuid::nil(),
            ..change.clone()
        },
        DefaultsChange {
            expected_revision: u64::MAX,
            ..change.clone()
        },
        DefaultsChange {
            key: DefaultKey::Activation {
                workspace: "relative".into(),
            },
            ..change.clone()
        },
    ];
    for request in invalid {
        assert!(store.change(&request).is_err());
    }
    assert!(!anchor.directory.join(record_name(1)).exists());
    assert!(store.list(None, 0).is_err());
    assert!(store.list(None, 101).is_err());
    assert!(store.list(Some("bad"), 1).is_err());
}
#[test]
fn wrong_private_store_kind_is_rejected_before_defaults_header_creation() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("shared");
    crate::policy_profile::store::ProfileStore::open(&path).unwrap();
    let anchor = DefaultsSource {
        directory: path.clone(),
        store_id: Uuid::new_v4(),
    };
    assert!(DefaultsStore::create(&anchor).is_err());
    assert!(!path.join(HEADER).exists());
}
#[test]
fn full_history_refuses_new_mutations_but_preserves_exact_retry_and_pagination() {
    let (_temp, _anchor, store, first) = setup();
    let mut previous = None;
    let mut previous_hash = ZERO_HASH.to_owned();
    for sequence in 1..=MAX_RECORDS {
        let change = if sequence == 1 {
            first.clone()
        } else {
            DefaultsChange {
                operation_id: Uuid::new_v4(),
                expected_revision: (sequence - 1) as u64,
                ..first.clone()
            }
        };
        let snapshot = proposal(&change, previous.as_ref()).unwrap();
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
        previous = Some(snapshot);
    }
    assert!(store.change(&first).unwrap().duplicate);
    assert!(matches!(
        store.change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: MAX_RECORDS as u64,
            ..first
        }),
        Err(StoreError::Capacity)
    ));
    let page = store.list(None, 1).unwrap();
    assert_eq!(page.profiles.len(), 1);
    assert!(page.next_after.is_none());
}

#[test]
fn self_consistent_but_invalid_profile_snapshot_is_not_committed() {
    let (temp, anchor, store, mut change) = setup();
    let directory = temp.path().join("profiles");
    let snapshot = crate::policy_profile::store::ProfileStore::open(&directory)
        .unwrap()
        .inspect("restricted")
        .unwrap()
        .unwrap();
    change.value = DefaultValue::Preference {
        profile: Some(ProfileRef {
            directory,
            name: snapshot.name.clone(),
            revision: snapshot.revision,
            digest: snapshot.digest().unwrap(),
            snapshot,
        }),
    };
    let DefaultValue::Preference {
        profile: Some(profile),
    } = &mut change.value
    else {
        panic!("profile fixture")
    };
    profile.snapshot.rules.as_mut().unwrap().inherit_env = vec!["not-a-profile-name".into()];
    profile.digest = profile.snapshot.digest().unwrap();
    assert!(store.change(&change).is_err());
    assert!(!anchor.directory.join(record_name(1)).exists());
}
