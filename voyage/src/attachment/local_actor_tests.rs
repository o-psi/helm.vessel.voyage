use super::*;
use std::fs;

fn private_write(path: &Path, bytes: &[u8]) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

#[test]
fn publication_recovers_every_durable_boundary_without_rotating_identity() {
    for stop in [
        Boundary::CandidateWritten,
        Boundary::CandidateDurable,
        Boundary::BeforePublish,
        Boundary::Published,
        Boundary::Durable,
        Boundary::Verified,
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("identity");
        assert!(
            LocalActorStore::open_with(&path, |at| {
                if at == stop {
                    anyhow::bail!("synthetic interruption")
                }
                Ok(())
            })
            .is_err()
        );
        let candidate = fs::read(path.join(CANDIDATE)).unwrap();
        let expected = decode(&candidate).unwrap();
        let recovered = LocalActorStore::open(&path).unwrap();
        assert_eq!(recovered.identity().unwrap(), expected);
        assert_eq!(fs::read(path.join(PUBLISHED)).unwrap(), candidate);
        assert_eq!(
            LocalActorStore::open(&path).unwrap().identity().unwrap(),
            expected
        );
    }
}

#[test]
fn interruption_before_creation_is_retryable_but_empty_candidate_is_not_replaced() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("identity");
    assert!(LocalActorStore::open_with(&path, |_| anyhow::bail!("stop")).is_err());
    assert!(!path.join(CANDIDATE).exists());
    assert!(
        LocalActorStore::open_with(&path, |at| {
            if at == Boundary::CandidateCreated {
                anyhow::bail!("stop")
            }
            Ok(())
        })
        .is_err()
    );
    assert!(LocalActorStore::open(&path).is_err());
    assert_eq!(fs::read(path.join(CANDIDATE)).unwrap(), b"");
    assert!(!path.join(PUBLISHED).exists());
}

#[test]
fn published_identity_survives_missing_candidate_but_conflicting_candidate_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("identity");
    let store = LocalActorStore::open(&path).unwrap();
    let expected = store.identity().unwrap();
    let published = fs::read(path.join(PUBLISHED)).unwrap();
    fs::remove_file(path.join(CANDIDATE)).unwrap();
    assert_eq!(
        LocalActorStore::open(&path).unwrap().identity().unwrap(),
        expected
    );
    let other = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    private_write(
        &path.join(CANDIDATE),
        &serde_json::to_vec(&Record {
            version: 1,
            actor: other,
        })
        .unwrap(),
    );
    assert!(LocalActorStore::open(&path).is_err());
    assert_eq!(fs::read(path.join(PUBLISHED)).unwrap(), published);
    // Equal decoded identity is insufficient: the exact persisted bytes are pinned.
    let mut padded = published.clone();
    padded.push(b'\n');
    private_write(&path.join(CANDIDATE), &padded);
    assert!(LocalActorStore::open(&path).is_err());
    assert_eq!(store.identity().unwrap(), expected);
}

#[test]
fn identity_revalidation_refuses_missing_changed_and_unsafe_publication() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("identity");
    let store = LocalActorStore::open(&path).unwrap();
    let published = path.join(PUBLISHED);
    let original = fs::read(&published).unwrap();
    fs::remove_file(&published).unwrap();
    assert!(store.identity().is_err());
    let other = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    private_write(
        &published,
        &serde_json::to_vec(&Record {
            version: 1,
            actor: other,
        })
        .unwrap(),
    );
    assert!(store.identity().is_err());
    private_write(&published, &original);
    fs::set_permissions(&published, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.identity().is_err());
}

#[test]
fn candidate_mutation_at_publication_boundaries_is_detected() {
    for stop in [Boundary::BeforePublish, Boundary::Published] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("identity");
        assert!(
            LocalActorStore::open_with(&path, |at| {
                if at == stop {
                    private_write(&path.join(CANDIDATE), b"{}");
                }
                Ok(())
            })
            .is_err()
        );
        assert!(LocalActorStore::open(&path).is_err());
    }
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("identity");
    assert!(
        LocalActorStore::open_with(&path, |at| {
            if at == Boundary::Published {
                private_write(&path.join(PUBLISHED), b"{}");
            }
            Ok(())
        })
        .is_err()
    );
}

#[test]
fn malformed_records_and_non_unique_identifiers_never_become_attribution() {
    let id = Uuid::new_v4();
    for actor in [
        LocalActor {
            installation_id: Uuid::nil(),
            principal_id: id,
        },
        LocalActor {
            installation_id: id,
            principal_id: Uuid::nil(),
        },
        LocalActor {
            installation_id: id,
            principal_id: id,
        },
    ] {
        assert!(decode(&serde_json::to_vec(&Record { version: 1, actor }).unwrap()).is_err());
    }
    let actor = LocalActor {
        installation_id: id,
        principal_id: Uuid::new_v4(),
    };
    assert!(decode(&serde_json::to_vec(&Record { version: 2, actor }).unwrap()).is_err());
    assert!(decode(&vec![b' '; MAX_BYTES + 1]).is_err());
    assert!(decode(b"not json").is_err());
    let mut value = serde_json::to_value(Record { version: 1, actor }).unwrap();
    value["unexpected"] = true.into();
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn private_identity_traverses_execute_only_ancestors_without_listing_them() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("traverse");
    let owned = parent.join("owned");
    fs::create_dir_all(&owned).unwrap();
    fs::set_permissions(&owned, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o111)).unwrap();
    let path = owned.join("identity");
    let observed = (|| -> Result<LocalActor> {
        if unsafe { libc::geteuid() } != 0 {
            ensure!(
                fs::read_dir(&parent).is_err(),
                "fixture parent unexpectedly listable"
            );
        }
        let store = LocalActorStore::open(&path)?;
        let actor = store.identity()?;
        let existing = storage::Directory::open_existing(&path)?;
        ensure!(
            decode(&existing.read(PUBLISHED)?.context("identity missing")?)? == actor,
            "identity changed"
        );
        Ok(actor)
    })();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let actor = observed.unwrap();
    assert_eq!(
        LocalActorStore::open(&path).unwrap().identity().unwrap(),
        actor
    );
    std::os::unix::fs::symlink(&parent, root.path().join("alias")).unwrap();
    assert!(LocalActorStore::open(&root.path().join("alias/owned/identity")).is_err());
    if unsafe { libc::geteuid() } != 0 {
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o000)).unwrap();
        let denied = LocalActorStore::open(&path);
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(denied.is_err());
    }
}
