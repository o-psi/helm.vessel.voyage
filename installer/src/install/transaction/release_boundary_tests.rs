//! Ordinary releases are pinned before staging; partial failures never publish them.
use super::*;
use crate::install::release::UpdateCompatibility;
use std::os::unix::fs::symlink;

const ASSETS: [&str; 5] = [
    "worker.mjs",
    "guardian.py",
    "package.json",
    "package-lock.json",
    "node_modules/playwright-core/package.json",
];
fn browser_source(f: &Fixture, version: &str) -> (PathBuf, Manifest) {
    let (source, mut manifest) = f.source(version);
    for name in ASSETS {
        let relative = format!("share/voyage/browser/{name}");
        let path = source.parent().unwrap().join(&relative);
        files::directory(path.parent().unwrap()).unwrap();
        files::write_new(&path, format!("fixture-{name}").as_bytes()).unwrap();
        manifest.assets.insert(
            relative,
            Binary {
                sha256: files::hash(&path).unwrap(),
            },
        );
    }
    fs::write(
        source.parent().unwrap().join("release.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    (source, manifest)
}
fn compatibility() -> UpdateCompatibility {
    UpdateCompatibility {
        schema_version: 1,
        formats: [
            "catalogue_read",
            "catalogue_write",
            "journal_read",
            "journal_write",
            "process_protocol",
            "vessel_protocol",
            "execution_identity",
        ]
        .into_iter()
        .map(|key| (key.into(), vec![1]))
        .collect(),
        implementation_sha256: "a".repeat(64),
        build_inputs_sha256: "b".repeat(64),
    }
}

#[test]
fn manifest_validation_refuses_wrong_identity_target_hash_and_browser_inventory() {
    let f = Fixture::new();
    let (_source, base) = browser_source(&f, "validation");
    assert!(base.validate().is_ok());
    for variant in 0..17 {
        let mut invalid = base.clone();
        match variant {
            0 => invalid.schema_version = 2,
            1 => invalid.version.clear(),
            2 => invalid.version = "x".repeat(129),
            3 => invalid.version = "private\nversion".into(),
            4 => {
                invalid.binaries.remove("helm");
            }
            5 => {
                invalid.binaries.insert(
                    "unexpected".into(),
                    Binary {
                        sha256: "a".repeat(64),
                    },
                );
            }
            6 => invalid.target = "wrong-host-target".into(),
            7 => invalid.binaries.get_mut("helm").unwrap().sha256 = "g".repeat(64),
            8 => invalid.binaries.get_mut("helm").unwrap().sha256 = "a".repeat(63),
            9 => {
                invalid.assets.remove("share/voyage/browser/worker.mjs");
            }
            10 => {
                invalid.assets.insert(
                    "share/voyage/browser/../escape".into(),
                    Binary {
                        sha256: "a".repeat(64),
                    },
                );
            }
            11 => {
                invalid.assets.insert(
                    "share/voyage/browser//escape".into(),
                    Binary {
                        sha256: "a".repeat(64),
                    },
                );
            }
            12 => {
                invalid.assets.insert(
                    "share/voyage/browser/escape\\name".into(),
                    Binary {
                        sha256: "a".repeat(64),
                    },
                );
            }
            13 => {
                invalid.assets.insert(
                    "share/voyage/browser/private\nname".into(),
                    Binary {
                        sha256: "a".repeat(64),
                    },
                );
            }
            14 => {
                invalid
                    .assets
                    .get_mut("share/voyage/browser/worker.mjs")
                    .unwrap()
                    .sha256 = "not-a-hash".into()
            }
            15 => {
                invalid.assets.insert(
                    format!("share/voyage/browser/{}", "x".repeat(513)),
                    Binary {
                        sha256: "a".repeat(64),
                    },
                );
            }
            _ => {
                for index in 0..1801 {
                    invalid.assets.insert(
                        format!("share/voyage/browser/extra-{index}"),
                        Binary {
                            sha256: "a".repeat(64),
                        },
                    );
                }
            }
        }
        assert!(invalid.validate().is_err(), "variant {variant}");
    }
}

#[test]
fn compatibility_requires_sorted_bounded_readable_formats_and_lowercase_digests() {
    let valid = compatibility();
    valid.validate().unwrap();
    for variant in 0..10 {
        let mut invalid = valid.clone();
        match variant {
            0 => invalid.schema_version = 2,
            1 => {
                invalid.formats.remove("execution_identity");
            }
            2 => {
                invalid.formats.insert("unknown".into(), vec![1]);
            }
            3 => {
                invalid.formats.insert("journal_read".into(), vec![]);
            }
            4 => {
                invalid.formats.insert("journal_read".into(), vec![1, 1]);
            }
            5 => {
                invalid.formats.insert("journal_read".into(), vec![2, 1]);
            }
            6 => {
                invalid.formats.insert("journal_read".into(), vec![0]);
            }
            7 => {
                invalid.formats.insert("journal_read".into(), vec![10001]);
            }
            8 => {
                invalid.formats.insert("journal_write".into(), vec![2]);
            }
            _ => invalid.implementation_sha256 = "A".repeat(64),
        }
        assert!(invalid.validate().is_err(), "variant {variant}");
    }
    let f = Fixture::new();
    let (_, mut manifest) = f.source("compatibility");
    let original = manifest.id().unwrap();
    manifest.update_compatibility = Some(valid);
    assert_eq!(manifest.id().unwrap(), original);
    manifest
        .update_compatibility
        .as_mut()
        .unwrap()
        .build_inputs_sha256 = "c".repeat(64);
    assert_eq!(manifest.id().unwrap(), original);
    manifest.version = "different-version".into();
    assert_ne!(manifest.id().unwrap(), original);
}

#[test]
fn browser_assets_stage_and_upgrade_reuse_exact_verified_files() {
    let f = Fixture::new();
    let (source, manifest) = browser_source(&f, "browser");
    let id = manifest.id().unwrap();
    let root = f.layout.release(&id);
    manifest.stage(&source, &root).unwrap();
    manifest.verify(&root).unwrap();
    for name in manifest.assets.keys() {
        assert_eq!(
            fs::read(root.join(name)).unwrap(),
            fs::read(source.parent().unwrap().join(name)).unwrap()
        );
    }
    manifest.stage(&source, &root).unwrap();
    assert!(!root.with_extension("staging").exists());
    let mut journal = f.layout.journal().unwrap();
    publish(&f.layout, &mut journal, &id).unwrap();
    assert_eq!(f.layout.pointer().unwrap(), Some(id.clone()));
    assert_eq!(f.layout.verify(&id).unwrap().assets.len(), 5);
}

#[test]
fn resume_reuses_matching_partial_binary_and_assets_without_publishing_changed_stage() {
    let f = Fixture::new();
    let (source, manifest) = browser_source(&f, "partial");
    let root = f.layout.release(&manifest.id().unwrap());
    let staging = root.with_extension("staging");
    files::directory(&staging.join("bin")).unwrap();
    fs::copy(source.join("helm"), staging.join("bin/helm")).unwrap();
    fs::set_permissions(staging.join("bin/helm"), fs::Permissions::from_mode(0o700)).unwrap();
    let relative = "share/voyage/browser/worker.mjs";
    files::directory(staging.join(relative).parent().unwrap()).unwrap();
    fs::copy(
        source.parent().unwrap().join(relative),
        staging.join(relative),
    )
    .unwrap();
    fs::set_permissions(staging.join(relative), fs::Permissions::from_mode(0o600)).unwrap();
    manifest.stage(&source, &root).unwrap();
    manifest.verify(&root).unwrap();
    assert!(!staging.exists());
}

#[test]
fn changed_binary_source_never_publishes_a_release() {
    let f = Fixture::new();
    let (source, manifest) = f.source("changed");
    let root = f.layout.release(&manifest.id().unwrap());
    fs::write(source.join("helm"), b"changed-unreviewed-binary").unwrap();
    assert!(manifest.stage(&source, &root).is_err());
    assert!(!root.exists());
    assert!(f.layout.pointer().unwrap().is_none());
}

#[test]
fn changed_browser_sources_partial_assets_and_staged_manifest_do_not_publish() {
    for variant in 0..2 {
        let f = Fixture::new();
        let (source, manifest) = browser_source(&f, "changed-assets");
        let root = f.layout.release(&manifest.id().unwrap());
        let staging = root.with_extension("staging");
        match variant {
            0 => {
                fs::write(
                    source
                        .parent()
                        .unwrap()
                        .join("share/voyage/browser/worker.mjs"),
                    b"changed",
                )
                .unwrap();
            }
            _ => {
                let dest = staging.join("share/voyage/browser/worker.mjs");
                files::directory(dest.parent().unwrap()).unwrap();
                fs::write(dest, b"changed").unwrap();
            }
        }
        assert!(manifest.stage(&source, &root).is_err());
        assert!(!root.exists());
        assert!(f.layout.pointer().unwrap().is_none());
    }
}

#[test]
fn installed_binary_and_browser_permission_hash_or_symlink_changes_refuse_upgrade() {
    for variant in 0..7 {
        let f = Fixture::new();
        let (source, manifest) = browser_source(&f, "installed");
        let root = f.layout.release(&manifest.id().unwrap());
        manifest.stage(&source, &root).unwrap();
        let asset = root.join("share/voyage/browser/worker.mjs");
        let binary = root.join("bin/helm");
        match variant {
            0 => fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap(),
            1 => fs::set_permissions(&binary, fs::Permissions::from_mode(0o600)).unwrap(),
            2 => fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap(),
            3 => fs::write(&binary, b"changed").unwrap(),
            4 => fs::write(&asset, b"changed").unwrap(),
            5 => fs::set_permissions(&asset, fs::Permissions::from_mode(0o644)).unwrap(),
            _ => {
                fs::remove_file(&asset).unwrap();
                symlink(
                    source
                        .parent()
                        .unwrap()
                        .join("share/voyage/browser/worker.mjs"),
                    &asset,
                )
                .unwrap();
            }
        }
        assert!(manifest.verify(&root).is_err(), "variant {variant}");
        assert!(manifest.stage(&source, &root).is_err());
        assert!(root.exists());
        assert!(f.layout.pointer().unwrap().is_none());
    }
}

#[test]
fn sparse_oversized_browser_asset_refuses_before_allocation_or_publication() {
    let f = Fixture::new();
    let (source, manifest) = browser_source(&f, "oversize");
    let root = f.layout.release(&manifest.id().unwrap());
    fs::File::options()
        .write(true)
        .open(
            source
                .parent()
                .unwrap()
                .join("share/voyage/browser/worker.mjs"),
        )
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    assert!(manifest.stage(&source, &root).is_err());
    assert!(!root.exists());
    assert!(f.layout.pointer().unwrap().is_none());
}

#[test]
fn inspect_refuses_changed_manifest_binary_membership_and_unsafe_source_modes() {
    for variant in 0..4 {
        let f = Fixture::new();
        let (source, manifest) = f.source("inspect");
        match variant {
            0 => {
                fs::write(source.join("helm"), b"changed").unwrap();
            }
            1 => {
                fs::set_permissions(source.join("helm"), fs::Permissions::from_mode(0o600)).unwrap()
            }
            2 => {
                fs::set_permissions(source.join("helm"), fs::Permissions::from_mode(0o722)).unwrap()
            }
            _ => {
                fs::remove_file(source.join("helm")).unwrap();
                symlink(source.join("vessel"), source.join("helm")).unwrap();
            }
        }
        assert!(Manifest::inspect(&source).is_err());
        assert!(f.layout.pointer().unwrap().is_none());
        assert_eq!(manifest.binaries.len(), 4);
    }
}
