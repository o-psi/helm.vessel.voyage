use super::*;
use crate::extensions::{Archive, Content, Kind, Manifest};

fn archive(text: &str) -> Vec<u8> {
    let archive = Archive {
        manifest: Manifest {
            format: 1,
            id: "offline-guide".into(),
            version: "1.0.0".into(),
            helm: env!("CARGO_PKG_VERSION").rsplit_once('.').unwrap().0.into(),
            capabilities: vec!["model_context".into()],
            contents: vec![
                Content {
                    path: "skill.md".into(),
                    kind: Kind::Skill,
                    sha256: digest(text.as_bytes()),
                },
                Content {
                    path: "reference.txt".into(),
                    kind: Kind::Resource,
                    sha256: digest(b"offline reference"),
                },
            ],
            entrypoints: vec!["skill.md".into()],
        },
        files: BTreeMap::from([
            ("skill.md".into(), text.into()),
            ("reference.txt".into(), "offline reference".into()),
        ]),
    };
    archive.validate().unwrap();
    serde_json::to_vec(&archive).unwrap()
}

#[test]
fn exact_install_review_update_revoke_and_remove_preserve_separate_authority() {
    for scope in [Scope::Project, Scope::User] {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        let catalog = Catalog::new(&workspace, &root.path().join("private/data")).unwrap();
        assert!(catalog.list().unwrap().is_empty());
        assert!(catalog.guidance().unwrap().is_empty());
        let first = archive("Offline reviewed guidance");
        let first_hash = digest(&first);
        catalog
            .mutate(scope, "offline-guide", None, Some(&first), None)
            .unwrap();
        let installed = catalog.inspect(scope, "offline-guide").unwrap();
        assert_eq!(installed.digest, first_hash);
        assert!(!installed.active);
        assert!(installed.manifest.is_some());
        assert!(!installed.execution_reviewed);
        assert_eq!(
            catalog
                .resource(scope, "offline-guide", "reference.txt")
                .unwrap(),
            "offline reference"
        );
        assert!(
            catalog
                .resource(scope, "offline-guide", "absent.txt")
                .is_err()
        );
        assert!(
            catalog
                .resource(scope, "offline-guide", "../reference.txt")
                .is_err()
        );
        assert!(
            catalog
                .mutate(
                    scope,
                    "offline-guide",
                    Some(&"0".repeat(64)),
                    None,
                    Some(true)
                )
                .is_err()
        );
        catalog
            .mutate(scope, "offline-guide", Some(&first_hash), None, Some(true))
            .unwrap();
        assert!(catalog.inspect(scope, "offline-guide").unwrap().active);
        assert!(
            catalog
                .guidance()
                .unwrap()
                .contains("Offline reviewed guidance")
        );
        assert_eq!(
            catalog
                .activation_records()
                .unwrap()
                .get(&installed.binding),
            Some(&first_hash)
        );
        assert!(
            catalog
                .revoke_grant(&installed.binding, &"0".repeat(64))
                .is_err()
        );
        catalog
            .revoke_grant(&installed.binding, &first_hash)
            .unwrap();
        assert!(!catalog.inspect(scope, "offline-guide").unwrap().active);
        assert!(catalog.guidance().unwrap().is_empty());
        catalog
            .mutate(scope, "offline-guide", Some(&first_hash), None, Some(true))
            .unwrap();
        let second = archive("Replacement must be reviewed separately");
        let second_hash = digest(&second);
        catalog
            .mutate(
                scope,
                "offline-guide",
                Some(&first_hash),
                Some(&second),
                None,
            )
            .unwrap();
        let updated = catalog.inspect(scope, "offline-guide").unwrap();
        assert!(!updated.active);
        assert_eq!(updated.digest, second_hash);
        assert!(catalog.activation_records().unwrap().is_empty());
        assert!(
            catalog
                .mutate(scope, "offline-guide", Some(&first_hash), None, None)
                .is_err()
        );
        catalog
            .mutate(scope, "offline-guide", Some(&second_hash), None, Some(true))
            .unwrap();
        catalog
            .mutate(
                scope,
                "offline-guide",
                Some(&second_hash),
                None,
                Some(false),
            )
            .unwrap();
        assert!(catalog.guidance().unwrap().is_empty());
        catalog
            .mutate(scope, "offline-guide", Some(&second_hash), None, None)
            .unwrap();
        assert!(catalog.list().unwrap().is_empty());
        assert!(catalog.inspect(scope, "offline-guide").is_err());
    }
}

#[test]
fn project_and_user_packages_have_distinct_bindings_and_cas() {
    let root = tempfile::tempdir().unwrap();
    let catalog = Catalog::new(root.path(), &root.path().join("user")).unwrap();
    let bytes = archive("Same package, independent scopes");
    let hash = digest(&bytes);
    for scope in [Scope::Project, Scope::User] {
        catalog
            .mutate(scope, "offline-guide", None, Some(&bytes), None)
            .unwrap();
        catalog
            .mutate(scope, "offline-guide", Some(&hash), None, Some(true))
            .unwrap();
    }
    let records = catalog.list().unwrap();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0].binding, records[1].binding);
    assert_eq!(catalog.activation_records().unwrap().len(), 2);
    catalog
        .mutate(Scope::Project, "offline-guide", Some(&hash), None, None)
        .unwrap();
    assert!(
        catalog
            .inspect(Scope::User, "offline-guide")
            .unwrap()
            .active
    );
    assert_eq!(catalog.activation_records().unwrap().len(), 1);
    assert!(
        catalog
            .mutate(Scope::User, "other-id", None, Some(&bytes), None)
            .is_err()
    );
    assert!(
        catalog
            .mutate(Scope::User, "../invalid", None, Some(&bytes), None)
            .is_err()
    );
}

#[test]
fn failed_publication_revokes_prior_activation_instead_of_preserving_stale_trust() {
    let root = tempfile::tempdir().unwrap();
    let catalog = Catalog::new(root.path(), &root.path().join("data")).unwrap();
    let first = archive("Original");
    let hash = digest(&first);
    catalog
        .mutate(Scope::Project, "offline-guide", None, Some(&first), None)
        .unwrap();
    catalog
        .mutate(
            Scope::Project,
            "offline-guide",
            Some(&hash),
            None,
            Some(true),
        )
        .unwrap();
    let replacement = archive("Replacement");
    assert!(
        catalog
            .mutate_with(
                Scope::Project,
                "offline-guide",
                Some(&hash),
                Some(&replacement),
                None,
                |_, _, _| anyhow::bail!("injected publication failure")
            )
            .is_err()
    );
    let inspection = catalog.inspect(Scope::Project, "offline-guide").unwrap();
    assert_eq!(inspection.digest, hash);
    assert!(!inspection.active);
    assert!(catalog.activation_records().unwrap().is_empty());
    assert!(catalog.guidance().unwrap().is_empty());
}
