use super::*;

fn sample() -> StoredDraft {
    let helm = Uuid::new_v4();
    StoredDraft::new(
        Draft {
            name: "Research α".into(),
            purpose: "Compare tools\nDocument findings".into(),
            participants: vec![helm],
            coordinator: Some(helm),
        },
        Some("https://vessel.example".into()),
        BTreeMap::from([(helm, "Desk".into())]),
    )
}

fn setup() -> (tempfile::TempDir, Store) {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().canonicalize().unwrap().join("drafts")).unwrap();
    (temp, store)
}

#[test]
fn restart_roundtrip_and_retry_preserve_one_revision() {
    let (temp, store) = setup();
    let proposed = sample();
    let saved = store.save(&proposed).unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(store.save(&proposed).unwrap(), saved);
    drop(store);
    let store = Store::new(temp.path().canonicalize().unwrap().join("drafts")).unwrap();
    assert_eq!(store.load(saved.id).unwrap(), Some(saved.clone()));
    assert_eq!(store.list().unwrap(), vec![saved]);
    assert_eq!(store.load(Uuid::new_v4()).unwrap(), None);
}

#[test]
fn stale_edit_cannot_overwrite_newer_revision_or_allocate_duplicate() {
    let (_temp, store) = setup();
    let initial = store.save(&sample()).unwrap();
    let second_store = Store::new(&store.path).unwrap();
    let mut changed = initial.clone();
    changed.draft.purpose = "Updated research".into();
    let newest = second_store.save(&changed).unwrap();
    assert_eq!(newest.revision, 2);
    let mut stale = initial;
    stale.draft.name = "Conflicting edit".into();
    assert!(
        store
            .save(&stale)
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    assert_eq!(store.list().unwrap(), vec![newest]);
}

#[test]
fn competing_process_handles_fail_busy_without_partial_writes_then_retry() {
    let (_temp, store) = setup();
    let other = Store::new(&store.path).unwrap();
    let proposed = sample();
    let lock = store.directory.lock().unwrap();
    assert!(other.save(&proposed).is_err());
    assert!(other.list().is_err());
    drop(lock);
    assert_eq!(other.save(&proposed).unwrap().revision, 1);
}

#[test]
fn validation_rejects_incomplete_scope_controls_bounds_and_credentials() {
    let (_temp, store) = setup();
    let good = sample();
    let mut invalid = vec![];
    let mut value = good.clone();
    value.draft.coordinator = None;
    invalid.push(value);
    let mut value = good.clone();
    value.draft.coordinator = Some(Uuid::new_v4());
    invalid.push(value);
    let mut value = good.clone();
    value.draft.participants.push(value.draft.participants[0]);
    invalid.push(value);
    let mut value = good.clone();
    value.draft.participants.clear();
    invalid.push(value);
    let mut value = good.clone();
    value.draft.name = "é".repeat(129);
    invalid.push(value);
    let mut value = good.clone();
    value.draft.name = "hidden\u{202e}".into();
    invalid.push(value);
    let mut value = good.clone();
    value.draft.purpose = "\x1b[2J".into();
    invalid.push(value);
    let mut value = good.clone();
    value.draft.purpose = "a".repeat(8193);
    invalid.push(value);
    let mut value = good.clone();
    value.origin = Some("https://user:secret@vessel.example".into());
    invalid.push(value);
    let mut value = good.clone();
    value.origin = Some("https://vessel.example?token=secret".into());
    invalid.push(value);
    let mut value = good.clone();
    value.helm_labels.insert(Uuid::new_v4(), "Other".into());
    invalid.push(value);
    let mut value = good.clone();
    value.revision = u64::MAX;
    invalid.push(value);
    for value in invalid {
        assert!(store.save(&value).is_err());
    }
    assert!(store.list().unwrap().is_empty());
    let mut boundary = good;
    boundary.draft.name = "é".repeat(128);
    boundary.draft.purpose = "a".repeat(8192);
    boundary.draft.participants = (0..64).map(|_| Uuid::new_v4()).collect();
    boundary.draft.coordinator = Some(boundary.draft.participants[0]);
    boundary.helm_labels.clear();
    assert!(store.save(&boundary).is_ok());
    boundary.id = Uuid::new_v4();
    boundary.draft.participants.push(Uuid::new_v4());
    assert!(store.save(&boundary).is_err());
}

#[test]
fn corrupt_future_schema_oversized_and_wrong_identity_records_fail_closed() {
    let (_temp, store) = setup();
    let saved = store.save(&sample()).unwrap();
    let path = store.path.join(filename(saved.id));
    let original = std::fs::read(&path).unwrap();
    for bytes in [
        b"{broken secret contents".to_vec(),
        vec![b' '; MAX_BYTES + 1],
        {
            let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
            json["schema"] = 99.into();
            serde_json::to_vec(&json).unwrap()
        },
        {
            let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
            json["value"]["id"] = Uuid::new_v4().to_string().into();
            serde_json::to_vec(&json).unwrap()
        },
        {
            let mut json: serde_json::Value = serde_json::from_slice(&original).unwrap();
            json["token"] = "secret".into();
            serde_json::to_vec(&json).unwrap()
        },
    ] {
        std::fs::write(&path, &bytes).unwrap();
        let error = store.load(saved.id).unwrap_err().to_string();
        assert!(!error.contains("secret"));
        assert!(store.save(&saved).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn abandoned_temporary_file_does_not_publish_a_draft_or_block_retry() {
    let (_temp, store) = setup();
    let proposed = sample();
    let _abandoned = store.directory.create(".tmp-abandoned").unwrap();
    assert!(store.list().unwrap().is_empty());
    assert!(store.save(&proposed).is_ok());
}

#[test]
fn capacity_blocks_new_records_but_allows_existing_edits_and_recent_order() {
    let (_temp, store) = setup();
    let mut first = None;
    for index in 0..MAX_DRAFTS {
        let mut value = sample();
        value.revision = 1;
        value.updated_at = DateTime::from_timestamp(index as i64, 0).unwrap();
        store
            .directory
            .publish(
                &filename(value.id),
                &serde_json::to_vec(&Record {
                    schema: SCHEMA,
                    value: value.clone(),
                })
                .unwrap(),
            )
            .unwrap();
        first.get_or_insert(value);
    }
    assert!(
        store
            .save(&sample())
            .unwrap_err()
            .to_string()
            .contains("capacity")
    );
    let mut edit = first.unwrap();
    edit.draft.name = "Updated existing plan".into();
    let saved = store.save(&edit).unwrap();
    let listed = store.list().unwrap();
    assert_eq!(listed.len(), MAX_DRAFTS);
    assert_eq!(listed[0], saved);
}

#[cfg(unix)]
#[test]
fn unix_private_modes_and_link_attacks_fail_without_modifying_target() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    let (temp, store) = setup();
    let proposed = sample();
    assert_eq!(
        std::fs::metadata(&store.path).unwrap().mode() & 0o777,
        0o700
    );
    let saved = store.save(&proposed).unwrap();
    let path = store.path.join(filename(saved.id));
    assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    let target = temp.path().join("target");
    std::fs::write(&target, b"private target").unwrap();
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(store.load(saved.id).is_err());
    assert!(store.save(&saved).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"private target");
    std::fs::remove_file(&path).unwrap();
    std::fs::hard_link(&target, &path).unwrap();
    assert!(store.load(saved.id).is_err());
    assert!(store.save(&saved).is_err());
    assert_eq!(std::fs::read(&target).unwrap(), b"private target");
    let alias = temp.path().join("alias");
    symlink(&store.path, &alias).unwrap();
    assert!(Store::new(alias).is_err());
}

#[cfg(unix)]
#[test]
fn unix_replaced_directory_and_public_records_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let (temp, store) = setup();
    let saved = store.save(&sample()).unwrap();
    let path = store.path.join(filename(saved.id));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.load(saved.id).is_err());
    std::fs::rename(&store.path, temp.path().join("moved")).unwrap();
    let _replacement = Store::new(&store.path).unwrap();
    assert!(store.list().is_err());
    assert!(store.save(&saved).is_err());
}
