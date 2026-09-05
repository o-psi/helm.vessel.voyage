use super::*;

#[test]
fn identity_survives_reopen_without_rewriting_published_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("local-actor");
    let first = LocalActorStore::open(&path).unwrap().identity().unwrap();
    assert!(!first.installation_id.is_nil());
    assert!(!first.principal_id.is_nil());
    assert_ne!(first.installation_id, first.principal_id);
    let bytes = std::fs::read(path.join("actor.json")).unwrap();
    let second = LocalActorStore::open(&path).unwrap().identity().unwrap();
    assert_eq!(first, second);
    assert_eq!(bytes, std::fs::read(path.join("actor.json")).unwrap());
    assert_eq!(bytes, std::fs::read(path.join("candidate.json")).unwrap());
}

#[test]
fn independent_installations_have_independent_local_actors() {
    let temp = tempfile::tempdir().unwrap();
    let a = LocalActorStore::open(&root(&temp).join("a"))
        .unwrap()
        .identity()
        .unwrap();
    let b = LocalActorStore::open(&root(&temp).join("b"))
        .unwrap()
        .identity()
        .unwrap();
    assert_ne!(a.installation_id, b.installation_id);
    assert_ne!(a.principal_id, b.principal_id);
}

#[test]
fn a_corrupt_published_identity_never_regenerates() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("local-actor");
    drop(LocalActorStore::open(&path).unwrap());
    std::fs::write(path.join("actor.json"), b"broken").unwrap();
    assert!(LocalActorStore::open(&path).is_err());
    assert_eq!(std::fs::read(path.join("actor.json")).unwrap(), b"broken");
}

#[test]
fn malformed_records_fail_closed_with_bytes_preserved() {
    for invalid in [
        b"{}".to_vec(), b"null".to_vec(), b"[]".to_vec(), vec![b'x'; MAX_BYTES + 1],
        serde_json::to_vec(&serde_json::json!({"version":2,"actor":{"installation_id":Uuid::new_v4(),"principal_id":Uuid::new_v4()}})).unwrap(),
        serde_json::to_vec(&serde_json::json!({"version":1,"actor":{"installation_id":Uuid::nil(),"principal_id":Uuid::new_v4()}})).unwrap(),
        serde_json::to_vec(&serde_json::json!({"version":1,"extra":true,"actor":{"installation_id":Uuid::new_v4(),"principal_id":Uuid::new_v4()}})).unwrap(),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let path = root(&temp).join("actor");
        drop(LocalActorStore::open(&path).unwrap());
        std::fs::write(path.join(PUBLISHED), &invalid).unwrap();
        assert!(LocalActorStore::open(&path).is_err());
        assert_eq!(std::fs::read(path.join(PUBLISHED)).unwrap(), invalid);
    }
}

fn root(temp: &tempfile::TempDir) -> std::path::PathBuf {
    std::fs::canonicalize(temp.path()).unwrap()
}

const BOUNDARIES: [Boundary; 8] = [
    Boundary::DirectoryReady,
    Boundary::CandidateCreated,
    Boundary::CandidateWritten,
    Boundary::CandidateDurable,
    Boundary::BeforePublish,
    Boundary::Published,
    Boundary::Durable,
    Boundary::Verified,
];

fn assert_recovery(path: &Path, boundary: Boundary) {
    let before = std::fs::read(path.join(CANDIDATE)).ok();
    #[cfg(unix)]
    let original_inode = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path.join(PUBLISHED))
            .ok()
            .map(|m| (m.dev(), m.ino()))
    };
    let result = LocalActorStore::open(path);
    if boundary == Boundary::CandidateCreated {
        assert!(
            result.is_err(),
            "partial candidate must require explicit recovery"
        );
        assert_eq!(before.as_deref(), Some(b"".as_slice()));
        assert_eq!(
            std::fs::read(path.join(CANDIDATE)).unwrap(),
            before.unwrap()
        );
        assert!(!path.join(PUBLISHED).exists());
    } else {
        let store = result.unwrap();
        let actor = store.identity().unwrap();
        if let Some(before) = before {
            assert_eq!(decode(&before).unwrap(), actor);
            assert_eq!(std::fs::read(path.join(CANDIDATE)).unwrap(), before);
        }
        assert_eq!(
            std::fs::read(path.join(CANDIDATE)).unwrap(),
            std::fs::read(path.join(PUBLISHED)).unwrap()
        );
    }
    #[cfg(unix)]
    if let Some(original) = original_inode {
        use std::os::unix::fs::MetadataExt;
        let current = std::fs::metadata(path.join(PUBLISHED)).unwrap();
        assert_eq!(original, (current.dev(), current.ino()));
    }
    assert!(std::fs::read_dir(path).unwrap().count() <= 4);
}

#[test]
fn every_outer_write_boundary_preserves_truthful_retry_and_candidate_evidence() {
    for stop in BOUNDARIES {
        let temp = tempfile::tempdir().unwrap();
        let path = root(&temp).join("actor");
        let result = LocalActorStore::open_with(&path, |boundary| {
            anyhow::ensure!(boundary != stop, "injected storage fault");
            Ok(())
        });
        assert!(result.is_err(), "{stop:?}");
        assert_recovery(&path, stop);
    }
}

#[test]
fn competing_published_identity_is_never_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let other = LocalActor {
        installation_id: Uuid::new_v4(),
        principal_id: Uuid::new_v4(),
    };
    let bytes = serde_json::to_vec(&Record {
        version: 1,
        actor: other,
    })
    .unwrap();
    let result = LocalActorStore::open_with(&path, |boundary| {
        if boundary == Boundary::BeforePublish {
            let directory = storage::Directory::open(&path)?;
            use std::io::Write;
            let mut file = directory.create(PUBLISHED)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(path.join(PUBLISHED)).unwrap(), bytes);
    assert_ne!(std::fs::read(path.join(CANDIDATE)).unwrap(), bytes);
    assert!(LocalActorStore::open(&path).is_err());
}

#[test]
fn immutable_handle_detects_changed_or_missing_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let store = LocalActorStore::open(&path).unwrap();
    let actor = store.identity().unwrap();
    let changed = serde_json::to_vec(&Record {
        version: 1,
        actor: LocalActor {
            installation_id: Uuid::new_v4(),
            ..actor
        },
    })
    .unwrap();
    std::fs::write(path.join(PUBLISHED), changed).unwrap();
    assert!(store.identity().is_err());
    std::fs::remove_file(path.join(PUBLISHED)).unwrap();
    assert!(store.identity().is_err());
}

#[test]
fn initialization_lock_is_bounded_and_store_does_not_hold_it_idle() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let store = LocalActorStore::open(&path).unwrap();
    let directory = storage::Directory::open(&path).unwrap();
    let lock = directory.lock().unwrap();
    let start = std::time::Instant::now();
    assert!(LocalActorStore::open(&path).is_err());
    assert!(start.elapsed() < std::time::Duration::from_secs(1));
    drop(lock);
    assert_eq!(
        LocalActorStore::open(&path).unwrap().identity().unwrap(),
        store.identity().unwrap()
    );
}

#[test]
fn actor_process_probe() {
    let Some(path) = std::env::var_os("HELM_ACTOR_TEST_PATH") else {
        return;
    };
    let path = std::path::PathBuf::from(path);
    if let Ok(stop) = std::env::var("HELM_ACTOR_TEST_CRASH") {
        let _ = LocalActorStore::open_with(&path, |boundary| {
            if format!("{boundary:?}") == stop {
                std::process::exit(77);
            }
            Ok(())
        });
        panic!("crash boundary did not execute");
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Ok(store) = LocalActorStore::open(&path) {
            let actor = store.identity().unwrap();
            assert!(!actor.installation_id.is_nil());
            if let Some(observation) = std::env::var_os("HELM_ACTOR_TEST_OBSERVATION") {
                std::fs::write(observation, serde_json::to_vec(&actor).unwrap()).unwrap();
            }
            std::process::exit(0);
        }
        assert!(
            std::time::Instant::now() < deadline,
            "actor initialization timed out"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn child(path: &Path, crash: Option<Boundary>, observation: Option<&Path>) -> std::process::Child {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "attachment::local_actor::tests::actor_process_probe",
            "--nocapture",
        ])
        .env("HELM_ACTOR_TEST_PATH", path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(crash) = crash {
        command.env("HELM_ACTOR_TEST_CRASH", format!("{crash:?}"));
    }
    if let Some(observation) = observation {
        command.env("HELM_ACTOR_TEST_OBSERVATION", observation);
    }
    command.spawn().unwrap()
}
fn wait(mut child: std::process::Child) -> i32 {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().unwrap();
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("actor subprocess timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn real_process_crashes_preserve_candidate_and_committed_identity() {
    for boundary in BOUNDARIES {
        let temp = tempfile::tempdir().unwrap();
        let path = root(&temp).join("actor");
        assert_eq!(wait(child(&path, Some(boundary), None)), 77);
        assert_recovery(&path, boundary);
    }
}

#[test]
fn concurrent_first_use_and_restart_converge_without_rewriting_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let observations: Vec<_> = (0..4)
        .map(|i| root(&temp).join(format!("observation-{i}")))
        .collect();
    let children: Vec<_> = observations
        .iter()
        .map(|observation| child(&path, None, Some(observation)))
        .collect();
    for child in children {
        assert_eq!(wait(child), 0);
    }
    let bytes = std::fs::read(path.join(PUBLISHED)).unwrap();
    let actor = decode(&bytes).unwrap();
    for observation in observations {
        let observed: LocalActor =
            serde_json::from_slice(&std::fs::read(observation).unwrap()).unwrap();
        assert_eq!(observed, actor);
    }
    assert_eq!(wait(child(&path, None, None)), 0);
    assert_eq!(std::fs::read(path.join(PUBLISHED)).unwrap(), bytes);
    assert_eq!(
        decode(&bytes).unwrap(),
        LocalActorStore::open(&path).unwrap().identity().unwrap()
    );
}

#[cfg(unix)]
#[test]
fn unix_private_paths_reject_symlinks_hardlinks_modes_and_swapped_directories() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let store = LocalActorStore::open(&path).unwrap();
    symlink(&path, root(&temp).join("alias")).unwrap();
    assert!(LocalActorStore::open(&root(&temp).join("alias")).is_err());
    std::fs::hard_link(path.join(PUBLISHED), path.join("hardlink")).unwrap();
    assert!(store.identity().is_err());
    std::fs::remove_file(path.join("hardlink")).unwrap();
    std::fs::set_permissions(path.join(PUBLISHED), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(store.identity().is_err());
    std::fs::set_permissions(path.join(PUBLISHED), std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(store.identity().is_ok());
    let moved = root(&temp).join("moved");
    std::fs::rename(&path, &moved).unwrap();
    let replacement = LocalActorStore::open(&path).unwrap();
    assert!(store.identity().is_err());
    assert!(replacement.identity().is_ok());
    assert!(LocalActorStore::open(&moved).is_ok());
}

#[test]
fn reopened_actor_preserves_journal_command_scope_without_duplicate_admission() {
    use crate::{
        attachment::journal::{Journal, TurnAdmission},
        session::Session,
    };
    let temp = tempfile::tempdir().unwrap();
    let actor_path = root(&temp).join("actor");
    let actor = LocalActorStore::open(&actor_path)
        .unwrap()
        .identity()
        .unwrap();
    let session = Session::new(root(&temp), "fixture".into());
    let journal_path = root(&temp).join("journal");
    let mut journal = Journal::open(journal_path.clone()).unwrap();
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let request = TurnAdmission {
        command_id: Uuid::new_v4(),
        machine_id: actor.installation_id,
        principal_id: actor.principal_id,
        session_id: session.id,
        expected_revision: 0,
        expires_at_ms: 60000,
        prompt: "accepted local prompt".into(),
    };
    let admitted = journal.admit_turn(&guard, &request, 1).unwrap();
    assert!(!admitted.duplicate);
    drop(guard);
    drop(journal);
    let restarted = LocalActorStore::open(&actor_path)
        .unwrap()
        .identity()
        .unwrap();
    let mut retry = request;
    retry.machine_id = restarted.installation_id;
    retry.principal_id = restarted.principal_id;
    let mut journal = Journal::open(journal_path).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    let duplicate = journal.admit_turn(&guard, &retry, 90000).unwrap();
    assert!(duplicate.duplicate);
    assert_eq!(admitted.run.id, duplicate.run.id);
    let snapshot = journal.load_session(session.id).unwrap();
    assert_eq!(snapshot.session.messages.len(), 1);
    assert_eq!(snapshot.session.usage.input_tokens, 0);
    retry.principal_id = Uuid::new_v4();
    assert!(journal.lookup_command(&retry).is_err());
}

#[cfg(unix)]
#[test]
fn directory_swap_during_creation_cannot_publish_in_replacement() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let moved = root(&temp).join("moved");
    assert!(
        LocalActorStore::open_with(&path, |boundary| {
            if boundary == Boundary::CandidateDurable {
                std::fs::rename(&path, &moved)?;
                std::fs::create_dir(&path)?;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            }
            Ok(())
        })
        .is_err()
    );
    assert!(!path.join(PUBLISHED).exists());
    assert!(!path.join(CANDIDATE).exists());
    assert!(moved.join(CANDIDATE).exists());
    assert!(!moved.join(PUBLISHED).exists());
}

#[cfg(unix)]
#[test]
fn rejects_unsafe_directory_candidate_lock_and_symlink_ancestor() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    for name in [CANDIDATE, PUBLISHED, "actor.lock", "publication.json"] {
        let path = root(&temp).join(name.replace('.', "-"));
        let directory = storage::Directory::open(&path).unwrap();
        let outside = root(&temp).join(format!("outside-{name}"));
        std::fs::write(&outside, b"untouched").unwrap();
        symlink(&outside, path.join(name)).unwrap();
        assert!(LocalActorStore::open(&path).is_err(), "{name}");
        assert_eq!(std::fs::read(&outside).unwrap(), b"untouched");
        drop(directory);
    }
    let path = root(&temp).join("public");
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(LocalActorStore::open(&path).is_err());
    let real = root(&temp).join("real");
    std::fs::create_dir(&real).unwrap();
    let alias = root(&temp).join("alias");
    symlink(&real, &alias).unwrap();
    assert!(LocalActorStore::open(&alias.join("actor")).is_err());
    assert!(!real.join("actor").exists());
}

#[cfg(windows)]
#[test]
fn windows_private_actor_handles_pin_directory_and_reject_linked_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    let store = LocalActorStore::open(&path).unwrap();
    assert!(std::fs::rename(&path, root(&temp).join("moved")).is_err());
    std::fs::hard_link(path.join(PUBLISHED), path.join("alias")).unwrap();
    assert!(store.identity().is_err());
    assert!(LocalActorStore::open(&path).is_err());
}

#[test]
fn changed_candidate_is_preserved_and_cannot_publish_cached_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    assert!(
        LocalActorStore::open_with(&path, |boundary| {
            if boundary == Boundary::BeforePublish {
                std::fs::write(path.join(CANDIDATE), b"changed")?;
            }
            Ok(())
        })
        .is_err()
    );
    assert_eq!(std::fs::read(path.join(CANDIDATE)).unwrap(), b"changed");
    assert!(!path.join(PUBLISHED).exists());
}

#[test]
fn partial_publication_slot_is_bounded_and_never_discarded_on_retry() {
    let temp = tempfile::tempdir().unwrap();
    let path = root(&temp).join("actor");
    #[cfg(unix)]
    let slot = "publication.json";
    #[cfg(windows)]
    let slot = ".new-actor.json";
    assert!(
        LocalActorStore::open_with(&path, |boundary| {
            if boundary == Boundary::BeforePublish {
                use std::io::Write;
                storage::Directory::open(&path)?
                    .create(slot)?
                    .write_all(b"partial")?;
            }
            Ok(())
        })
        .is_err()
    );
    let candidate = std::fs::read(path.join(CANDIDATE)).unwrap();
    for _ in 0..8 {
        assert!(LocalActorStore::open(&path).is_err());
    }
    assert_eq!(std::fs::read(path.join(slot)).unwrap(), b"partial");
    assert_eq!(std::fs::read(path.join(CANDIDATE)).unwrap(), candidate);
    assert_eq!(std::fs::read_dir(&path).unwrap().count(), 3);
    assert!(!path.join(PUBLISHED).exists());
}
