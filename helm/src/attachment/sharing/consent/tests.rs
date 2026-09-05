use super::*;

fn setup() -> (tempfile::TempDir, ConsentStore, ConsentChange) {
    let temp = tempfile::tempdir().unwrap();
    let actor = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    let root = temp.path().canonicalize().unwrap().join("consent");
    let store = ConsentStore::open(&root, actor).unwrap();
    let change = ConsentChange {
        operation_id: Uuid::new_v4(),
        actor,
        session_id: Uuid::new_v4(),
        expected_revision: 0,
        destination: Some(
            ConsentDestination::new(
                "https://vessel.example",
                Uuid::new_v4(),
                Uuid::new_v4(),
                1,
                false,
            )
            .unwrap(),
        ),
        settings: SharingSettings::new(Disclosure::Live, false),
    };
    (temp, store, change)
}
fn commit(store: &ConsentStore, change: &ConsentChange) -> ConsentReceipt {
    let preview = store.preview_local(change).unwrap();
    store
        .commit_local(change, &preview.confirmation_digest)
        .unwrap()
}
#[test]
fn absent_default_preview_confirmation_and_restart_tombstone_are_explicit() {
    let (temp, store, mut change) = setup();
    assert!(store.inspect_local(change.session_id).unwrap().is_none());
    let preview = store.preview_local(&change).unwrap();
    assert!(store.inspect_local(change.session_id).unwrap().is_none());
    assert!(store.commit_local(&change, "not the preview").is_err());
    let first = store
        .commit_local(&change, &preview.confirmation_digest)
        .unwrap();
    assert_eq!(first.snapshot.revision, 1);
    assert!(!first.duplicate);
    let mut disable = change.clone();
    disable.operation_id = Uuid::new_v4();
    disable.expected_revision = 1;
    disable.destination = None;
    disable.settings = SharingSettings::new(Disclosure::None, false);
    let disabled = commit(&store, &disable);
    assert_eq!(disabled.snapshot.revision, 2);
    let actor = change.actor;
    drop(store);
    let store =
        ConsentStore::open(&temp.path().canonicalize().unwrap().join("consent"), actor).unwrap();
    let tombstone = store.inspect_local(change.session_id).unwrap().unwrap();
    assert_eq!(tombstone.settings.disclosure, Disclosure::None);
    assert!(tombstone.destination.is_none());
    assert!(
        store
            .commit_local(&change, &preview.confirmation_digest)
            .unwrap()
            .duplicate
    );
    change.expected_revision = 2;
    assert!(
        store
            .commit_local(&change, &preview.confirmation_digest)
            .is_err()
    );
    assert_eq!(store.audit_local(0, 100).unwrap().entries.len(), 2);
}
#[test]
fn destination_epoch_actor_and_cas_changes_need_fresh_confirmed_consent() {
    let (_temp, store, change) = setup();
    let first = commit(&store, &change);
    let mut next = change.clone();
    next.operation_id = Uuid::new_v4();
    next.expected_revision = first.snapshot.revision;
    let preview = store.preview_local(&next).unwrap();
    next.destination.as_mut().unwrap().epoch = 2;
    assert!(
        store
            .commit_local(&next, &preview.confirmation_digest)
            .is_err()
    );
    commit(&store, &next);
    assert!(store.preview_local(&change).unwrap().already_committed);
    for field in 0..4 {
        let mut wrong = next.clone();
        wrong.operation_id = Uuid::new_v4();
        wrong.expected_revision = 2;
        match field {
            0 => wrong.actor.installation_id = Uuid::new_v4(),
            1 => wrong.actor.principal_id = Uuid::new_v4(),
            2 => {
                wrong.destination.as_mut().unwrap().origin =
                    "https://user:secret@vessel.example/path".into()
            }
            _ => wrong.destination.as_mut().unwrap().epoch = 0,
        }
        assert!(store.preview_local(&wrong).is_err());
    }
    let mut stale = next;
    stale.operation_id = Uuid::new_v4();
    assert!(store.preview_local(&stale).is_err());
}
#[test]
fn pages_and_audit_are_bounded_metadata_only_and_exact_retries_are_immutable() {
    let (_temp, store, change) = setup();
    let first = commit(&store, &change);
    for _ in 0..2 {
        let mut other = change.clone();
        other.operation_id = Uuid::new_v4();
        other.session_id = Uuid::new_v4();
        commit(&store, &other);
    }
    assert!(store.list_local(None, 0).is_err());
    assert!(store.audit_local(0, 101).is_err());
    let page = store.list_local(None, 2).unwrap();
    assert_eq!(page.sessions.len(), 2);
    assert!(page.next_after.is_some());
    assert_eq!(
        store.list_local(page.next_after, 2).unwrap().sessions.len(),
        1
    );
    let audit = store.audit_local(0, 2).unwrap();
    assert_eq!(audit.entries.len(), 2);
    assert!(audit.next_after.is_some());
    assert_eq!(
        store
            .audit_local(audit.next_after.unwrap(), 2)
            .unwrap()
            .entries
            .len(),
        1
    );
    let encoded = serde_json::to_string(&page).unwrap();
    assert!(
        !encoded.contains("messages")
            && !encoded.contains("provider_state")
            && !encoded.contains("private_key")
    );
    let preview = store.preview_local(&change).unwrap();
    let repeated = store
        .commit_local(&change, &preview.confirmation_digest)
        .unwrap();
    assert!(repeated.duplicate);
    assert_eq!(repeated.sequence, first.sequence);
}

#[test]
fn missing_last_record_never_restores_an_older_grant() {
    let (temp, store, change) = setup();
    commit(&store, &change);
    let mut disable = change.clone();
    disable.operation_id = Uuid::new_v4();
    disable.expected_revision = 1;
    disable.destination = None;
    disable.settings = SharingSettings::new(Disclosure::None, false);
    commit(&store, &disable);
    std::fs::remove_file(temp.path().join("consent").join(record_name(2))).unwrap();
    assert!(store.inspect_local(change.session_id).is_err());
}

#[test]
fn publication_failures_never_activate_staged_intent_and_exact_retry_recovers() {
    for failure in [
        Boundary::CandidateCreated,
        Boundary::CandidateWritten,
        Boundary::CandidateDurable,
        Boundary::BeforePublish,
        Boundary::Published,
        Boundary::DirectorySynced,
        Boundary::Verified,
    ] {
        let (temp, store, change) = setup();
        let preview = store.preview_local(&change).unwrap();
        assert!(
            store
                .commit_with(&change, &preview.confirmation_digest, |boundary| {
                    if boundary == failure {
                        Err(ConsentError::Storage)
                    } else {
                        Ok(())
                    }
                })
                .is_err()
        );
        drop(store);
        let store = ConsentStore::open(
            &temp.path().canonicalize().unwrap().join("consent"),
            change.actor,
        )
        .unwrap();
        if failure == Boundary::CandidateCreated {
            assert!(store.inspect_local(change.session_id).is_err());
            assert!(
                store
                    .commit_local(&change, &preview.confirmation_digest)
                    .is_err()
            );
            continue;
        }
        if matches!(
            failure,
            Boundary::CandidateWritten | Boundary::CandidateDurable | Boundary::BeforePublish
        ) {
            assert_eq!(
                store.inspect_local(change.session_id).unwrap_err(),
                ConsentError::Pending
            );
            let mut other = change.clone();
            other.operation_id = Uuid::new_v4();
            assert!(store.preview_local(&other).is_err());
        }
        let receipt = store
            .commit_local(&change, &preview.confirmation_digest)
            .unwrap();
        assert_eq!(
            receipt.duplicate,
            matches!(
                failure,
                Boundary::Published | Boundary::DirectorySynced | Boundary::Verified
            )
        );
        assert_eq!(
            store
                .inspect_local(change.session_id)
                .unwrap()
                .unwrap()
                .revision,
            1
        );
        assert_eq!(store.audit_local(0, 100).unwrap().entries.len(), 1);
    }
}

#[test]
fn malformed_gapped_and_mismatched_witness_evidence_fails_closed_without_raw_data() {
    for variant in 0..6 {
        let (temp, store, change) = setup();
        commit(&store, &change);
        let mut next = change.clone();
        next.operation_id = Uuid::new_v4();
        next.expected_revision = 1;
        commit(&store, &next);
        let path = temp.path().join("consent");
        match variant {
            0 => std::fs::write(path.join(record_name(1)), b"SECRET_BAD_JSON").unwrap(),
            1 => std::fs::remove_file(path.join(record_name(1))).unwrap(),
            2 => std::fs::write(path.join(witness_name(2)), b"SECRET_BAD_WITNESS").unwrap(),
            3 => std::fs::write(path.join(record_name(2)), vec![b'x'; MAX_RECORD + 1]).unwrap(),
            4 => std::fs::remove_file(path.join(witness_name(2))).unwrap(),
            _ => std::fs::write(
                path.join(storage::publication_name(&record_name(3)).unwrap()),
                b"SECRET_PARTIAL",
            )
            .unwrap(),
        }
        let error = store.list_local(None, 100).unwrap_err();
        assert!(!format!("{error:?}: {error}").contains("SECRET"));
        assert!(store.preview_local(&change).is_err());
    }
}

#[test]
fn held_lock_and_two_handles_never_commit_competing_revisions() {
    let (temp, store, change) = setup();
    let other = ConsentStore::open(
        &temp.path().canonicalize().unwrap().join("consent"),
        change.actor,
    )
    .unwrap();
    let first = store.preview_local(&change).unwrap();
    let mut competing = change.clone();
    competing.operation_id = Uuid::new_v4();
    competing.settings.disclosure = Disclosure::Transcript;
    let second = other.preview_local(&competing).unwrap();
    let lock = store.directory.lock().unwrap();
    assert_eq!(
        other
            .commit_local(&competing, &second.confirmation_digest)
            .unwrap_err(),
        ConsentError::Busy
    );
    drop(lock);
    store
        .commit_local(&change, &first.confirmation_digest)
        .unwrap();
    assert_eq!(
        other
            .commit_local(&competing, &second.confirmation_digest)
            .unwrap_err(),
        ConsentError::Conflict
    );
    assert_eq!(
        other
            .inspect_local(change.session_id)
            .unwrap()
            .unwrap()
            .settings
            .disclosure,
        Disclosure::Live
    );
}

#[test]
fn immutable_operation_id_cannot_change_scope_settings_or_original_revision() {
    let (_temp, store, change) = setup();
    let receipt = commit(&store, &change);
    for field in 0..6 {
        let mut wrong = change.clone();
        match field {
            0 => wrong.session_id = Uuid::new_v4(),
            1 => wrong.expected_revision = 1,
            2 => wrong.settings.disclosure = Disclosure::Transcript,
            3 => wrong.destination.as_mut().unwrap().machine_id = Uuid::new_v4(),
            4 => wrong.destination.as_mut().unwrap().owner_id = Uuid::new_v4(),
            _ => wrong.destination.as_mut().unwrap().origin = "https://elsewhere.example".into(),
        }
        assert!(
            store
                .commit_local(&wrong, &receipt.confirmation_digest)
                .is_err()
        );
    }
    let mut disabled = change.clone();
    disabled.operation_id = Uuid::new_v4();
    disabled.expected_revision = 1;
    disabled.destination = None;
    disabled.settings = SharingSettings::new(Disclosure::None, false).with_approvals(true, false);
    assert!(store.preview_local(&disabled).is_err());
}

#[cfg(unix)]
#[test]
fn private_storage_rejects_links_replacement_and_unsafe_names_without_changing_actor_behavior() {
    use std::os::unix::fs::symlink;
    let (temp, store, change) = setup();
    let path = temp.path().join("consent");
    for name in [
        "../escape",
        "/absolute",
        "nested/file",
        "nested\\file",
        "name:stream",
        ".",
        "..",
        "trailing.",
        "bad\nname",
    ] {
        assert!(store.directory.create(name).is_err());
        assert!(store.directory.read_bounded(name, MAX_RECORD).is_err());
        assert!(store.directory.publish_new(name, b"safe").is_err());
    }
    assert!(store.directory.read_bounded(HEADER, 0).is_err());
    assert!(store.directory.read_bounded(HEADER, 65_537).is_err());
    assert!(
        store
            .directory
            .publish_new("too-large", &vec![0; 65_537])
            .is_err()
    );
    let candidate = path.join(storage::publication_name(&record_name(1)).unwrap());
    symlink(temp.path().join("missing"), &candidate).unwrap();
    assert!(store.preview_local(&change).is_err());
    std::fs::remove_file(&candidate).unwrap();
    commit(&store, &change);
    let linked = temp.path().join("extra-link");
    std::fs::hard_link(path.join(record_name(1)), &linked).unwrap();
    assert!(store.inspect_local(change.session_id).is_err());
    std::fs::remove_file(linked).unwrap();
    let moved = temp.path().join("moved");
    std::fs::rename(&path, &moved).unwrap();
    assert!(store.list_local(None, 1).is_err());
    let unicode = temp
        .path()
        .canonicalize()
        .unwrap()
        .join("private actor λ space");
    let actor = crate::attachment::local_actor::LocalActorStore::open(&unicode).unwrap();
    assert!(actor.identity().is_ok());
}

#[test]
fn actual_capacity_keeps_receipts_and_rejects_new_mutations() {
    let (_temp, store, mut change) = setup();
    let mut previous = None;
    let mut prior_hash = ZERO_HASH.to_owned();
    let mut first = None;
    for sequence in 1..=MAX_RECORDS {
        change.operation_id = Uuid::new_v4();
        change.expected_revision = (sequence - 1) as u64;
        let (proposed, confirmation_digest) = store.proposal(&change, previous.as_ref()).unwrap();
        let record = Record {
            version: VERSION,
            sequence: sequence as u64,
            previous_hash: prior_hash,
            change: change.clone(),
            previous,
            proposed: proposed.clone(),
            confirmation_digest,
        };
        let bytes = encoded(&record).unwrap();
        // Construct a complete bounded fixture efficiently; production publication
        // durability is exercised separately at every boundary.
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
        prior_hash = digest(&bytes);
        previous = Some(proposed);
        if first.is_none() {
            first = Some(record);
        }
    }
    let first = first.unwrap();
    assert!(
        store
            .commit_local(&first.change, &first.confirmation_digest)
            .unwrap()
            .duplicate
    );
    change.operation_id = Uuid::new_v4();
    change.expected_revision = MAX_RECORDS as u64;
    assert_eq!(
        store.preview_local(&change).unwrap_err(),
        ConsentError::Capacity
    );
    assert_eq!(
        store
            .inspect_local(change.session_id)
            .unwrap()
            .unwrap()
            .revision,
        MAX_RECORDS as u64
    );
}

#[test]
fn consent_process_fixture() {
    let Some(root) = std::env::var_os("HELM_CONSENT_FIXTURE") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let change: ConsentChange =
        serde_json::from_slice(&std::fs::read(root.join("request.json")).unwrap()).unwrap();
    let mode = std::env::var("HELM_CONSENT_MODE").unwrap();
    let opened = ConsentStore::open(&root.join("consent"), change.actor);
    if mode == "blocked" {
        assert!(matches!(opened, Err(ConsentError::Busy)));
        return;
    }
    let store = opened.unwrap();
    let preview = store.preview_local(&change).unwrap();
    store
        .commit_with(&change, &preview.confirmation_digest, |boundary| {
            if mode == format!("{boundary:?}") {
                std::process::exit(42);
            }
            Ok(())
        })
        .unwrap();
    std::process::exit(0);
}

#[test]
fn independent_process_locking_and_crash_recovery_keep_exact_intent() {
    for mode in ["CandidateWritten", "BeforePublish", "Published", "complete"] {
        let (temp, store, change) = setup();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(
            root.join("request.json"),
            serde_json::to_vec(&change).unwrap(),
        )
        .unwrap();
        let spawn = |mode: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "attachment::sharing::consent::tests::consent_process_fixture",
                    "--nocapture",
                ])
                .env("HELM_CONSENT_FIXTURE", &root)
                .env("HELM_CONSENT_MODE", mode)
                .output()
                .unwrap()
        };
        let lock = store.directory.lock().unwrap();
        let blocked = spawn("blocked");
        assert!(
            blocked.status.success(),
            "{}",
            String::from_utf8_lossy(&blocked.stderr)
        );
        drop(lock);
        let preview = store.preview_local(&change).unwrap();
        let crashed = spawn(mode);
        assert_eq!(
            crashed.status.code(),
            Some(if mode == "complete" { 0 } else { 42 }),
            "{}",
            String::from_utf8_lossy(&crashed.stderr)
        );
        if matches!(mode, "CandidateWritten" | "BeforePublish") {
            assert_eq!(
                store.inspect_local(change.session_id).unwrap_err(),
                ConsentError::Pending
            );
        }
        let receipt = store
            .commit_local(&change, &preview.confirmation_digest)
            .unwrap();
        assert_eq!(receipt.duplicate, matches!(mode, "Published" | "complete"));
        assert_eq!(store.audit_local(0, 100).unwrap().entries.len(), 1);
        assert_eq!(
            store
                .inspect_local(change.session_id)
                .unwrap()
                .unwrap()
                .settings
                .disclosure,
            Disclosure::Live
        );
    }
}
