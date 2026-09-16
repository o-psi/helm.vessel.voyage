use super::super::{database, test_support::Fixture};
use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

fn evidence(f: &Fixture, r: &ProcessRegistration) -> serde_json::Value {
    serde_json::json!({"session_id": r.session_id, "incarnation": r.incarnation, "cleanup_observed": true, "suspended": true, "fixture": f.0})
}
fn save(f: &Fixture, value: &serde_json::Value) {
    let path = f.0.join("stopped.json");
    std::fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn cleanup_evidence_fences_identity_reason_and_resource_existence() {
    let f = Fixture::new();
    let mut r = f.registration();
    r.state = ProcessState::Live;
    assert!(!clean_stop(&f.0, &r));
    assert!(!suspended(&f.0, &r));
    let original = evidence(&f, &r);
    save(&f, &original);
    assert!(clean_stop(&f.0, &r));
    assert!(suspended(&f.0, &r));
    assert!(archived(&f.0, &r).is_none());
    assert!(deletion(&f.0, &r).is_none());
    for (field, value) in [
        ("session_id", serde_json::json!(uuid::Uuid::new_v4())),
        ("incarnation", serde_json::json!(uuid::Uuid::new_v4())),
        ("cleanup_observed", serde_json::json!(false)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        save(&f, &changed);
        assert!(!clean_stop(&f.0, &r), "{field}");
        assert!(!suspended(&f.0, &r), "{field}");
    }
    let mut changed = original.clone();
    changed["suspended"] = serde_json::json!(false);
    save(&f, &changed);
    assert!(clean_stop(&f.0, &r));
    assert!(!suspended(&f.0, &r));
    save(&f, &original);
    std::fs::write(f.0.join("runtime.sock"), b"not a runtime").unwrap();
    assert!(!suspended(&f.0, &r));
    std::fs::remove_file(f.0.join("runtime.sock")).unwrap();
    r.state = ProcessState::Relinquished;
    assert!(!suspended(&f.0, &r));
    r.state = ProcessState::Suspended;
    std::fs::remove_file(f.0.join("stopped.json")).unwrap();
    assert!(!suspended(&f.0, &r));
}

#[test]
fn stop_evidence_refuses_unsafe_files_links_oversize_and_malformed_json() {
    let f = Fixture::new();
    let r = f.registration();
    let value = evidence(&f, &r);
    save(&f, &value);
    std::fs::set_permissions(
        f.0.join("stopped.json"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(!clean_stop(&f.0, &r));
    save(&f, &value);
    std::fs::hard_link(f.0.join("stopped.json"), f.0.join("linked")).unwrap();
    assert!(!clean_stop(&f.0, &r));
    std::fs::remove_file(f.0.join("linked")).unwrap();
    assert!(clean_stop(&f.0, &r));
    std::fs::rename(f.0.join("stopped.json"), f.0.join("actual")).unwrap();
    symlink(f.0.join("actual"), f.0.join("stopped.json")).unwrap();
    assert!(!clean_stop(&f.0, &r));
    std::fs::remove_file(f.0.join("stopped.json")).unwrap();
    save(&f, &value);
    std::fs::write(f.0.join("stopped.json"), vec![b' '; 4096]).unwrap();
    assert!(!clean_stop(&f.0, &r));
    std::fs::write(f.0.join("stopped.json"), b"{bad json}").unwrap();
    assert!(!clean_stop(&f.0, &r));
}

#[tokio::test]
async fn startup_gate_requires_private_single_link_regular_file_and_releases_lock() {
    let f = Fixture::new();
    assert!(startup_gate(&f.0).await.is_err());
    let path = f.0.join("startup.lock");
    std::fs::write(&path, b"").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(startup_gate(&f.0).await.is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::hard_link(&path, f.0.join("alias")).unwrap();
    assert!(startup_gate(&f.0).await.is_err());
    std::fs::remove_file(f.0.join("alias")).unwrap();
    let lock = startup_gate(&f.0).await.unwrap();
    let competitor = std::fs::File::open(&path).unwrap();
    assert!(matches!(
        competitor.try_lock(),
        Err(std::fs::TryLockError::WouldBlock)
    ));
    drop(lock);
    let lock = startup_gate(&f.0).await.unwrap();
    drop(lock);
    competitor.try_lock().unwrap();
}

#[tokio::test]
async fn restart_rejects_nil_and_unknown_session_without_creating_registration() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let r = f.registration();
    assert!(
        s.restart(uuid::Uuid::nil(), r.session_id, r.incarnation)
            .await
            .unwrap_err()
            .to_string()
            .contains("nonnil")
    );
    let directory = registry::directory(&f.0, r.session_id);
    registry::private_directory(&directory).unwrap();
    let path = directory.join("startup.lock");
    std::fs::write(&path, b"").unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        s.restart(uuid::Uuid::new_v4(), r.session_id, r.incarnation)
            .await
            .unwrap_err()
            .to_string()
            .contains("unknown session")
    );
    assert!(database::registration(&f.0, r.session_id).await.is_err());
    assert!(s.registrations.lock().await.unwrap().is_empty());
}
