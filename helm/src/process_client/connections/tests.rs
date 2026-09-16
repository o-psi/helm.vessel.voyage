// Serialize fork-based fixtures against short-lived advisory locks.
pub(super) fn process_guard() -> std::sync::MutexGuard<'static, ()> {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}
use super::access::WorkspaceCredential;
use super::*;

fn connection() -> Connection {
    Connection {
        id: Uuid::new_v4(),
        alias: "fixture".into(),
        endpoint: "http://127.0.0.1:12345".into(),
        vessel_id: Uuid::new_v4(),
        principal_id: Some(Uuid::new_v4()),
        grant_id: Uuid::new_v4(),
        scope: Scope::Workspaces {
            workspace_ids: vec![],
        },
        credential_ref: Uuid::new_v4(),
        autoconnect: false,
        workspace_preference: None,
        revision: 1,
        forgotten: false,
        legacy_route: None,
        metadata: Metadata::default(),
    }
}
fn credential(c: &Connection) -> Credential {
    Credential::Workspace(WorkspaceCredential {
        schema_version: 1,
        kind: "workspace".into(),
        endpoint: c.endpoint.clone(),
        grant_id: c.grant_id,
        principal_id: c.principal_id.unwrap(),
        vessel_id: c.vessel_id,
        token: "offline-fixture-token".into(),
    })
}
fn seed(registry: &Registry, connections: Vec<Connection>) {
    let mut snapshot = Snapshot {
        connections,
        ..Default::default()
    };
    Registry::commit(&registry.directory().unwrap(), &mut snapshot).unwrap();
}
fn preview(registry: &Registry, c: Connection) -> ConnectionPreview {
    let dir = registry.directory().unwrap();
    let preview = ConnectionPreview {
        connection: c.clone(),
        preview_id: Uuid::new_v4(),
    };
    dir.write(
        &format!("{}.preview", preview.preview_id),
        &serde_json::to_vec(&c).unwrap(),
        private::CREDENTIAL_LIMIT * 4,
    )
    .unwrap();
    // Legacy plaintext is an explicitly supported read format. Seed only the
    // disposable fixture; avoid host key provisioning and environment changes.
    let path = registry.credential_path(c.credential_ref);
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .unwrap();
    file.write_all(&serde_json::to_vec(&credential(&c)).unwrap())
        .unwrap();
    preview
}

#[test]
fn registry_roundtrip_cas_forget_restore_preserves_credentials() {
    let _process = process_guard();
    let temp = tempfile::tempdir().unwrap();
    let registry = Registry::open(temp.path().join("registry")).unwrap();
    let empty = registry.load().unwrap();
    assert_eq!(empty.revision, 0);
    assert!(empty.connections.is_empty());
    let identity = retry_busy(|| registry.principal_id()).unwrap();
    assert_eq!(identity, retry_busy(|| registry.principal_id()).unwrap());
    let c = connection();
    let p = preview(&registry, c.clone());
    let saved = retry_busy(|| registry.save_preview(&p, "reviewed".into(), true)).unwrap();
    assert_eq!(saved.alias, "reviewed");
    assert_eq!(saved.revision, 1);
    assert_eq!(
        retry_busy(|| registry.save_preview(&p, "ignored".into(), false)).unwrap(),
        saved
    );
    let changed = retry_busy(|| {
        registry.update(
            saved.id,
            1,
            Preferences {
                alias: "edited".into(),
                autoconnect: false,
                workspace_preference: None,
            },
        )
    })
    .unwrap();
    assert_eq!(changed.revision, 2);
    assert!(registry.forget(saved.id, 1).is_err());
    assert!(registry.forget(Uuid::new_v4(), 1).is_err());
    assert!(registry.restore(saved.id, 2).is_err());
    let forgotten = retry_busy(|| registry.forget(saved.id, 2)).unwrap();
    assert!(forgotten.forgotten);
    assert!(!forgotten.autoconnect);
    assert!(registry.credential_path(c.credential_ref).exists());
    assert!(
        registry
            .update(
                saved.id,
                3,
                Preferences {
                    alias: "no".into(),
                    autoconnect: true,
                    workspace_preference: None
                }
            )
            .is_err()
    );
    assert!(registry.save_preview(&p, "no".into(), false).is_err());
    let restored = retry_busy(|| registry.restore(saved.id, 3)).unwrap();
    assert!(!restored.forgotten);
    assert!(!restored.autoconnect);
    assert_eq!(restored.credential_ref, c.credential_ref);
    assert_eq!(restored.revision, 4);
}

#[test]
fn independent_record_updates_merge_and_failed_edits_leave_bytes_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let registry = Registry::open(temp.path().join("registry")).unwrap();
    let a = connection();
    let b = connection();
    seed(&registry, vec![a.clone(), b.clone()]);
    retry_busy(|| registry.forget(a.id, 1)).unwrap();
    retry_busy(|| registry.forget(b.id, 1)).unwrap();
    assert!(
        registry
            .load()
            .unwrap()
            .connections
            .iter()
            .all(|c| c.forgotten)
    );
    let before = std::fs::read(registry.root.join("registry.json")).unwrap();
    assert!(registry.restore(a.id, 2).is_err());
    assert_eq!(
        std::fs::read(registry.root.join("registry.json")).unwrap(),
        before
    );
    let mut exhausted = connection();
    exhausted.revision = u64::MAX;
    seed(&registry, vec![exhausted.clone()]);
    assert!(registry.forget(exhausted.id, u64::MAX).is_err());
    assert_eq!(registry.load().unwrap().connections[0], exhausted);
}

#[test]
fn previews_and_exact_grants_cannot_be_substituted() {
    let temp = tempfile::tempdir().unwrap();
    let registry = Registry::open(temp.path().join("registry")).unwrap();
    let c = connection();
    let mut p = preview(&registry, c.clone());
    p.connection.endpoint = "https://example.invalid".into();
    assert!(registry.save_preview(&p, "x".into(), false).is_err());
    p.connection = c.clone();
    retry_busy(|| registry.save_preview(&p, "x".into(), false)).unwrap();
    let mut duplicate = c.clone();
    duplicate.id = Uuid::new_v4();
    duplicate.credential_ref = Uuid::new_v4();
    let duplicate = preview(&registry, duplicate);
    assert!(
        registry
            .save_preview(&duplicate, "x".into(), false)
            .is_err()
    );
    let missing = ConnectionPreview {
        connection: c,
        preview_id: Uuid::new_v4(),
    };
    assert!(registry.save_preview(&missing, "x".into(), false).is_err());
}

#[test]
fn snapshot_validation_rejects_bad_identity_version_and_preferences() {
    assert!(Registry::open(PathBuf::from("relative")).is_err());
    let mut snapshot = Snapshot::default();
    snapshot.schema_version = 2;
    assert!(Registry::validate(&snapshot).is_err());
    snapshot.schema_version = 1;
    let c = connection();
    snapshot.connections = vec![c.clone(), c.clone()];
    assert!(Registry::validate(&snapshot).is_err());
    for field in 0..5 {
        let mut bad = c.clone();
        match field {
            0 => bad.id = Uuid::nil(),
            1 => bad.vessel_id = Uuid::nil(),
            2 => bad.credential_ref = Uuid::nil(),
            3 => bad.alias = "a\nb".into(),
            _ => bad.endpoint = "file:///tmp/fixture".into(),
        }
        snapshot.connections = vec![bad];
        assert!(Registry::validate(&snapshot).is_err());
    }
    snapshot.connections = vec![c.clone(); 4097];
    assert!(Registry::validate(&snapshot).is_err());
    for alias in ["x".repeat(257), "x\0".into()] {
        assert!(
            preferences(
                &c,
                &Preferences {
                    alias,
                    autoconnect: false,
                    workspace_preference: None
                }
            )
            .is_err()
        );
    }
    let workspace = Uuid::new_v4();
    let p = Preferences {
        alias: "ok".into(),
        autoconnect: true,
        workspace_preference: Some(workspace),
    };
    assert!(preferences(&c, &p).is_err());
    let mut c = c;
    c.metadata.workspaces.push(Workspace {
        id: workspace,
        name: "fixture".into(),
        path: "/fixture".into(),
        provider_ready: Some(false),
    });
    assert!(preferences(&c, &p).is_ok());
}

#[test]
fn credential_pin_checks_every_identity_and_scope() {
    let c = connection();
    let secret = credential(&c);
    assert!(validate_credential(&c, &secret).is_ok());
    for field in 0..5 {
        let mut changed = c.clone();
        match field {
            0 => changed.endpoint.push('/'),
            1 => changed.grant_id = Uuid::new_v4(),
            2 => changed.principal_id = None,
            3 => changed.vessel_id = Uuid::new_v4(),
            _ => {
                changed.scope = Scope::Session {
                    session_id: Uuid::new_v4(),
                }
            }
        }
        let error = validate_credential(&changed, &secret).unwrap_err();
        assert_eq!(
            error.downcast_ref::<ConnectionFailure>(),
            Some(&ConnectionFailure::Identity)
        );
    }
}

#[test]
fn legacy_route_identity_is_deterministic_and_separates_optional_paths() {
    let a = LegacyRoute {
        directory: "/fixture".into(),
        access_file: None,
    };
    let b = LegacyRoute {
        directory: "/fixture".into(),
        access_file: Some("/access".into()),
    };
    assert_eq!(a.id(), a.clone().id());
    assert_ne!(a.id(), b.id());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&a.key().unwrap()).unwrap(),
        serde_json::json!(["/fixture", null, null])
    );
    assert_eq!(a.id().get_version_num(), 8);
}

// Concurrent subprocess fixtures may briefly inherit an advisory lock before exec.
// Retry only an explicit non-admission busy refusal, never uncertain mutations.
fn retry_busy<T>(mut operation: impl FnMut() -> anyhow::Result<T>) -> anyhow::Result<T> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match operation() {
            Err(error)
                if error.to_string().contains("connection registry busy")
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            result => return result,
        }
    }
}
