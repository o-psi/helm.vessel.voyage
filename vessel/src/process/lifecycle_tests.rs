use super::*;
use crate::process::{database, test_support::Fixture};
use uuid::Uuid;

fn import(f: &Fixture, source: std::path::PathBuf, digest: &str) -> VesselCommand {
    VesselCommand::Import {
        command_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        source_directory: source,
        expected_revision: 7,
        source_sha256: digest.into(),
        config_path: None,
    }
}
#[tokio::test]
async fn import_rejects_noncanonical_digests_before_launch_or_recording() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    for digest in [
        "".into(),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        "é".repeat(32),
    ] {
        let error = s
            .initialize_import(import(&f, f.0.clone(), &digest))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("invalid source SHA256"));
    }
    assert!(s.registrations.lock().await.unwrap().is_empty());
}
#[tokio::test]
async fn import_requires_existing_absolute_directory() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let file = f.0.join("file");
    std::fs::write(&file, b"not directory").unwrap();
    for source in ["relative".into(), f.0.join("missing"), file] {
        let error = s
            .initialize_import(import(&f, source, &"a".repeat(64)))
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("import source must be an absolute host directory")
        );
    }
    assert!(s.registrations.lock().await.unwrap().is_empty());
}
#[tokio::test]
async fn managed_import_rejects_missing_source_without_launch() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let error = s
        .initialize_managed(VesselCommand::ManagedImport {
            command_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            workspace: f.0.clone(),
            source_directory: f.0.join("missing"),
            expected_revision: 9,
            config_path: None,
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("managed source must be an absolute host installation directory")
    );
    assert!(s.registrations.lock().await.unwrap().is_empty());
}
fn branch(r: &ProcessRegistration, branch_id: Uuid) -> VesselCommand {
    VesselCommand::Branch {
        command_id: Uuid::new_v4(),
        session_id: r.session_id,
        incarnation: r.incarnation,
        expected_revision: 2,
        expires_at_ms: u64::MAX,
        branch_id,
        name: Some("offline branch".into()),
        through_message: None,
    }
}
#[tokio::test]
async fn branch_rejects_nil_and_self_identity_before_contacting_source() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let r = f.registration();
    for id in [Uuid::nil(), r.session_id] {
        assert!(
            s.branch(branch(&r, id))
                .await
                .unwrap_err()
                .to_string()
                .contains("invalid branch identity")
        );
    }
}
#[tokio::test]
async fn branch_missing_source_does_not_create_destination() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    assert!(
        s.branch(branch(&f.registration(), Uuid::new_v4()))
            .await
            .unwrap_err()
            .to_string()
            .contains("source session not registered")
    );
    assert!(s.registrations.lock().await.unwrap().is_empty());
}
#[tokio::test]
async fn branch_stale_incarnation_preserves_source_registration() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let r = f.registration();
    database::save(&f.0, &r).await.unwrap();
    let mut stale = r.clone();
    stale.incarnation = Uuid::new_v4();
    assert!(
        s.branch(branch(&stale, Uuid::new_v4()))
            .await
            .unwrap_err()
            .to_string()
            .contains("stale source incarnation")
    );
    let registrations = s.registrations.lock().await.unwrap();
    assert_eq!(registrations.len(), 1);
    assert_eq!(registrations[&r.session_id].incarnation, r.incarnation);
}
