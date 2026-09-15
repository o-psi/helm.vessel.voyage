use super::*;
use crate::install::release::{BINARIES, Binary};
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture {
    path: PathBuf,
    layout: Layout,
}
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "voyage-installer-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        files::private_directory(&path).unwrap();
        let layout = Layout {
            root: path.join("install"),
            bin: path.join("bin"),
        };
        files::private_directory(&layout.root).unwrap();
        files::directory(&layout.root.join("releases")).unwrap();
        Self { path, layout }
    }
    fn source(&self, version: &str) -> (PathBuf, Manifest) {
        let source = self.path.join(version).join("bin");
        files::directory(&source).unwrap();
        let mut binaries = BTreeMap::new();
        for name in BINARIES {
            let p = source.join(name);
            files::write_new(&p, format!("fixture-{version}-{name}").as_bytes()).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            binaries.insert(
                name.into(),
                Binary {
                    sha256: files::hash(&p).unwrap(),
                },
            );
        }
        let manifest = Manifest {
            schema_version: 1,
            version: version.into(),
            target: format!("linux-{}", std::env::consts::ARCH),
            binaries,
        };
        files::write_new(
            &source.parent().unwrap().join("release.json"),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        (source, manifest)
    }
    fn stage(&self, version: &str) -> String {
        let (source, manifest) = self.source(version);
        let id = manifest.id().unwrap();
        manifest.stage(&source, &self.layout.release(&id)).unwrap();
        id
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).unwrap();
    }
}

#[test]
fn publication_upgrade_and_rollback_preserve_previous_release() {
    let f = Fixture::new();
    let l = &f.layout;
    let first = f.stage("one");
    let second = f.stage("two");
    let mut j = l.journal().unwrap();
    assert!(j.current.is_none());
    publish(l, &mut j, &first).unwrap();
    assert_eq!(l.pointer().unwrap(), Some(first.clone()));
    assert!(j.previous.is_none());
    publish(l, &mut j, &second).unwrap();
    assert_eq!(l.journal().unwrap().previous, Some(first.clone()));
    assert_eq!(l.pointer().unwrap(), Some(second.clone()));
    publish(l, &mut j, &second).unwrap();
    assert_eq!(j.previous, Some(first.clone()));
    publish(l, &mut j, &first).unwrap();
    assert_eq!(j.previous, Some(second));
    assert_eq!(j.current, Some(first.clone()));
    assert!(!l.report(&first, "one", &j).unwrap().changed);
    assert!(j.pending.is_none());
    assert!(!l.root.join("next").exists());
}

#[test]
fn recovery_before_and_after_pointer_exchange_is_idempotent() {
    for exchanged in [false, true] {
        let f = Fixture::new();
        let l = &f.layout;
        let first = f.stage("one");
        let second = f.stage("two");
        let mut j = l.journal().unwrap();
        publish(l, &mut j, &first).unwrap();
        j.pending = Some(second.clone());
        l.save(&j).unwrap();
        symlink(l.release(&second), l.root.join("next")).unwrap();
        if exchanged {
            files::exchange(&l.root.join("next"), &l.root.join("current")).unwrap();
        }
        let mut reopened = l.journal().unwrap();
        recover(l, &mut reopened).unwrap();
        assert!(reopened.pending.is_none());
        assert_eq!(
            reopened.current,
            Some(if exchanged {
                second.clone()
            } else {
                first.clone()
            })
        );
        assert_eq!(
            reopened.previous,
            if exchanged { Some(first) } else { None }
        );
        recover(l, &mut reopened).unwrap();
        publish(l, &mut reopened, &second).unwrap();
        assert_eq!(l.pointer().unwrap(), Some(second));
        assert!(!l.root.join("next").exists());
    }
}

#[test]
fn unexpected_recovery_pointers_and_corrupt_releases_are_preserved() {
    for mode in ["current", "next", "binary"] {
        let f = Fixture::new();
        let l = &f.layout;
        let first = f.stage("one");
        let second = f.stage("two");
        let mut j = l.journal().unwrap();
        publish(l, &mut j, &first).unwrap();
        j.pending = Some(second.clone());
        l.save(&j).unwrap();
        match mode {
            "current" => {
                fs::remove_file(l.root.join("current")).unwrap();
                symlink(f.path.join("unrelated"), l.root.join("current")).unwrap();
            }
            "next" => {
                fs::remove_file(l.root.join("current")).unwrap();
                symlink(l.release(&second), l.root.join("current")).unwrap();
                symlink(f.path.join("unrelated"), l.root.join("next")).unwrap();
            }
            "binary" => {
                fs::write(l.release(&second).join("bin/helm"), b"tampered").unwrap();
            }
            _ => unreachable!(),
        }
        let before = fs::read(l.root.join("transaction.json")).unwrap();
        assert!(recover(l, &mut j).is_err(), "{mode}");
        assert_eq!(fs::read(l.root.join("transaction.json")).unwrap(), before);
        assert_eq!(l.journal().unwrap().pending, Some(second));
        if mode == "next" {
            assert_eq!(
                fs::read_link(l.root.join("next")).unwrap(),
                f.path.join("unrelated")
            );
        }
    }
}

#[test]
fn manifest_staging_retries_and_source_tampering() {
    let f = Fixture::new();
    let (source, manifest) = f.source("one");
    assert_eq!(
        Manifest::inspect(&source).unwrap().id().unwrap(),
        manifest.id().unwrap()
    );
    let dest = f.layout.release(&manifest.id().unwrap());
    let staging = dest.with_extension("staging");
    files::directory(&staging.join("bin")).unwrap();
    let partial = staging.join("bin/helm");
    fs::copy(source.join("helm"), &partial).unwrap();
    manifest.stage(&source, &dest).unwrap();
    manifest.stage(&source, &dest).unwrap();
    manifest.verify(&dest).unwrap();
    assert!(!staging.exists());
    fs::write(dest.join("bin/helm"), b"changed").unwrap();
    assert!(manifest.stage(&source, &dest).is_err());
    let (source, manifest) = f.source("two");
    fs::write(source.join("vessel"), b"changed").unwrap();
    assert!(Manifest::inspect(&source).is_err());
    let dest = f.layout.release(&manifest.id().unwrap());
    assert!(manifest.stage(&source, &dest).is_err());
    assert!(!dest.exists());
}

#[test]
fn manifests_reject_invalid_schema_target_hash_names_and_permissions() {
    let f = Fixture::new();
    let (source, manifest) = f.source("one");
    for field in ["schema", "target", "hash", "name", "version"] {
        let mut bad = manifest.clone();
        match field {
            "schema" => bad.schema_version = 2,
            "target" => bad.target = "foreign-host".into(),
            "hash" => bad.binaries.get_mut("helm").unwrap().sha256 = "x".repeat(64),
            "name" => {
                bad.binaries.remove("vessel");
            }
            "version" => bad.version = "bad\nversion".into(),
            _ => unreachable!(),
        }
        assert!(bad.validate().is_err(), "{field}");
    }
    for mode in [0o600, 0o777, 0o4700] {
        fs::set_permissions(source.join("helm"), fs::Permissions::from_mode(mode)).unwrap();
        assert!(Manifest::inspect(&source).is_err());
    }
    fs::remove_file(source.join("helm")).unwrap();
    symlink(source.join("vessel"), source.join("helm")).unwrap();
    assert!(Manifest::inspect(&source).is_err());
}

#[test]
fn links_preserve_unmanaged_commands_and_back_up_explicit_replacements() {
    let f = Fixture::new();
    let l = &f.layout;
    let j = l.journal().unwrap();
    files::directory(&l.bin).unwrap();
    files::write_new(&l.bin.join("helm"), b"user executable").unwrap();
    assert!(validate(l, &j, false).is_err());
    assert!(publish_links(l, false).is_err());
    assert_eq!(fs::read(l.bin.join("helm")).unwrap(), b"user executable");
    validate(l, &j, true).unwrap();
    publish_links(l, true).unwrap();
    let backups: Vec<_> = fs::read_dir(l.root.join("backups")).unwrap().collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read(backups[0].as_ref().unwrap().path()).unwrap(),
        b"user executable"
    );
    publish_links(l, false).unwrap();
    validate(l, &j, false).unwrap();
    for name in BINARIES {
        assert_eq!(
            fs::read_link(l.bin.join(name)).unwrap(),
            l.root.join("current/bin").join(name)
        );
    }
    fs::remove_file(l.bin.join("helm")).unwrap();
    symlink(f.path.join("unmanaged"), l.bin.join("helm")).unwrap();
    assert!(validate(l, &j, true).is_err());
    assert!(publish_links(l, true).is_err());
    assert_eq!(
        fs::read_link(l.bin.join("helm")).unwrap(),
        f.path.join("unmanaged")
    );
}

#[test]
fn invalid_journals_and_unexpected_next_links_are_not_overwritten() {
    let f = Fixture::new();
    let l = &f.layout;
    for bytes in [
        b"not json".as_slice(),
        br#"{"schema_version":2,"current":null,"previous":null,"pending":null}"#,
        br#"{"schema_version":1,"current":"../escape","previous":null,"pending":null}"#,
    ] {
        fs::write(l.root.join("transaction.json"), bytes).unwrap();
        assert!(l.journal().is_err());
        assert_eq!(fs::read(l.root.join("transaction.json")).unwrap(), bytes);
    }
    fs::remove_file(l.root.join("transaction.json")).unwrap();
    let id = f.stage("one");
    let mut j = l.journal().unwrap();
    symlink(f.path.join("unmanaged"), l.root.join("next")).unwrap();
    assert!(publish(l, &mut j, &id).is_err());
    assert_eq!(
        fs::read_link(l.root.join("next")).unwrap(),
        f.path.join("unmanaged")
    );
    assert!(l.pointer().unwrap().is_none());
    assert!(j.current.is_none());
}

#[test]
fn conflicting_partial_stage_and_manifest_identity_are_refused() {
    for metadata_conflict in [false, true] {
        let f = Fixture::new();
        let (source, manifest) = f.source("one");
        let dest = f.layout.release(&manifest.id().unwrap());
        let staging = dest.with_extension("staging");
        files::directory(&staging.join("bin")).unwrap();
        if metadata_conflict {
            let mut bad = manifest.clone();
            bad.version = "different".into();
            files::write_new(
                &staging.join("release.json"),
                &serde_json::to_vec(&bad).unwrap(),
            )
            .unwrap();
        } else {
            files::write_new(&staging.join("bin/helm"), b"unrelated contents").unwrap();
        }
        assert!(manifest.stage(&source, &dest).is_err());
        assert!(staging.exists());
        assert!(!dest.exists());
        if !metadata_conflict {
            assert_eq!(
                fs::read(staging.join("bin/helm")).unwrap(),
                b"unrelated contents"
            );
        }
    }
    let f = Fixture::new();
    let id = f.stage("one");
    let mut manifest = f.layout.verify(&id).unwrap();
    manifest.version = "changed".into();
    fs::write(
        f.layout.release(&id).join("release.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    assert!(f.layout.verify(&id).is_err());
}

#[test]
fn private_permissions_and_bounded_reads_are_enforced() {
    let f = Fixture::new();
    let id = f.stage("one");
    let root = f.layout.release(&id);
    let manifest = f.layout.verify(&id).unwrap();
    for mode in [0o600, 0o755, 0o4700] {
        fs::set_permissions(root.join("bin/helm"), fs::Permissions::from_mode(mode)).unwrap();
        assert!(manifest.verify(&root).is_err());
    }
    fs::set_permissions(root.join("bin/helm"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(manifest.verify(&root).is_err());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    manifest.verify(&root).unwrap();
    assert!(files::read(&root.join("release.json"), 1).is_err());
    assert!(files::read(&root, 100).is_err());
    let lock = files::lock(&f.layout.root.join("lock")).unwrap();
    assert!(files::lock(&f.layout.root.join("lock")).is_err());
    drop(lock);
    assert!(files::lock(&f.layout.root.join("lock")).is_ok());
}
