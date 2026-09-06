use super::*;
use catalog::{Catalog, Scope};
use std::fs;
pub(super) fn bytes(id: &str, text: &str) -> Vec<u8> {
    serde_json::to_vec(&Archive {
        manifest: Manifest {
            format: 1,
            id: id.into(),
            version: "1.0.0".into(),
            helm: "0.1".into(),
            capabilities: vec!["model_context".into()],
            contents: vec![Content {
                path: "skill.md".into(),
                kind: Kind::Skill,
                sha256: digest(text.as_bytes()),
            }],
            entrypoints: vec!["skill.md".into()],
        },
        files: BTreeMap::from([("skill.md".into(), text.into())]),
    })
    .unwrap()
}
#[test]
fn strict_manifest_integrity_and_portability() {
    let good = bytes("demo", "hello");
    assert!(Archive::parse(&good).is_ok());
    let mut a = Archive::parse(&good).unwrap();
    a.files.insert("extra".into(), "x".into());
    assert!(a.validate().is_err());
    for bad in [
        "../x", "/x", "C:/x", "x\\y", "CON", "con.txt", "a/../b", "skill.MD", "lpt1.md", "a.",
        ".hidden",
    ] {
        assert!(!portable(bad), "{bad}");
    }
    for capabilities in [
        vec!["shell".into()],
        vec![],
        vec!["model_context".into(), "model_context".into()],
    ] {
        let mut bad = Archive::parse(&good).unwrap();
        bad.manifest.capabilities = capabilities;
        assert!(bad.validate().is_err());
    }
    let mut a = Archive::parse(&good).unwrap();
    a.files.insert("skill.md".into(), "changed".into());
    assert!(a.validate().is_err());
    a = Archive::parse(&good).unwrap();
    a.manifest.helm = "9.9".into();
    assert!(a.validate().is_err());
    a = Archive::parse(&good).unwrap();
    a.manifest.entrypoints.push("skill.md".into());
    assert!(a.validate().is_err());
    a = Archive::parse(&good).unwrap();
    a.manifest.contents[0].kind = Kind::Resource;
    assert!(a.validate().is_err());
    let mut value: serde_json::Value = serde_json::from_slice(&good).unwrap();
    value["manifest"]["command"] = "touch marker".into();
    assert!(Archive::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(Archive::parse(&vec![b' '; MAX_ARCHIVE + 1]).is_err());
}
#[test]
fn complete_lifecycle_scopes_restart_and_stale_cas() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let user = root.path().join("data");
    fs::create_dir(&work).unwrap();
    let c = Catalog::new(&work, &user).unwrap();
    let first = bytes("demo", "first");
    let sha = digest(&first);
    c.mutate(Scope::User, "demo", None, Some(&first), None)
        .unwrap();
    assert!(!c.inspect(Scope::User, "demo").unwrap().active);
    c.mutate(Scope::User, "demo", Some(&sha), None, Some(true))
        .unwrap();
    assert!(c.guidance().unwrap().contains("first"));
    c.mutate(Scope::Project, "demo", None, Some(&first), None)
        .unwrap();
    assert!(c.guidance().unwrap().is_empty());
    c.mutate(Scope::Project, "demo", Some(&sha), None, Some(true))
        .unwrap();
    assert!(c.guidance().unwrap().contains("first"));
    let second = bytes("demo", "second");
    let sha2 = digest(&second);
    assert!(
        c.mutate(Scope::Project, "demo", Some("stale"), Some(&second), None)
            .is_err()
    );
    assert!(c.guidance().unwrap().contains("first"));
    c.mutate(Scope::Project, "demo", Some(&sha), Some(&second), None)
        .unwrap();
    assert!(c.guidance().unwrap().is_empty());
    let restarted = Catalog::new(&work, &user).unwrap();
    assert!(!restarted.inspect(Scope::Project, "demo").unwrap().active);
    restarted
        .mutate(Scope::Project, "demo", Some(&sha2), None, Some(true))
        .unwrap();
    assert!(restarted.guidance().unwrap().contains("second"));
    restarted
        .mutate(Scope::Project, "demo", Some(&sha2), None, Some(false))
        .unwrap();
    assert!(restarted.guidance().unwrap().is_empty());
    restarted
        .mutate(Scope::Project, "demo", Some(&sha2), None, None)
        .unwrap();
    assert!(restarted.guidance().unwrap().contains("first"));
    restarted
        .mutate(Scope::User, "demo", Some(&sha), None, None)
        .unwrap();
    assert!(restarted.list().unwrap().is_empty());
}
#[test]
fn invalid_shadow_and_corrupt_grants_never_activate_fallback() {
    let root = tempfile::tempdir().unwrap();
    let work = root.path().join("work");
    let user = root.path().join("data");
    fs::create_dir(&work).unwrap();
    let c = Catalog::new(&work, &user).unwrap();
    let good = bytes("demo", "secret instruction");
    let sha = digest(&good);
    for scope in [Scope::User, Scope::Project] {
        c.mutate(scope, "demo", None, Some(&good), None).unwrap();
        c.mutate(scope, "demo", Some(&sha), None, Some(true))
            .unwrap();
    }
    let path = work.join(".helm/extensions/catalog.json");
    fs::write(&path, r#"{"format":1,"packages":{"demo":"malformed"}}"#).unwrap();
    assert!(c.guidance().unwrap().is_empty());
    fs::write(&path, "broken").unwrap();
    assert!(c.guidance().is_err());
    fs::remove_file(path).unwrap();
    fs::write(user.join("extension-grants/grants.json"), "broken").unwrap();
    assert!(c.guidance().is_err());
    assert!(!c.inspect(Scope::User, "demo").unwrap().active);
}
#[test]
fn pack_rejects_extra_and_symlink_inputs() {
    let root = tempfile::tempdir().unwrap();
    let raw = bytes("demo", "hello");
    let a = Archive::parse(&raw).unwrap();
    fs::write(
        root.path().join("manifest.json"),
        serde_json::to_vec(&a.manifest).unwrap(),
    )
    .unwrap();
    fs::write(root.path().join("skill.md"), "hello").unwrap();
    assert!(store::pack(root.path()).is_ok());
    fs::write(root.path().join("unexpected"), "x").unwrap();
    assert!(store::pack(root.path()).is_err());
    fs::remove_file(root.path().join("unexpected")).unwrap();
    #[cfg(unix)]
    {
        fs::remove_file(root.path().join("skill.md")).unwrap();
        std::os::unix::fs::symlink("manifest.json", root.path().join("skill.md")).unwrap();
        assert!(store::pack(root.path()).is_err());
    }
}
#[cfg(unix)]
#[test]
fn non_utf8_locations_never_collapse_activation_identity() {
    use std::os::unix::ffi::OsStringExt;
    let root = tempfile::tempdir().unwrap();
    let bad = root.path().join(std::ffi::OsString::from_vec(vec![0xff]));
    fs::create_dir(&bad).unwrap();
    assert!(Catalog::new(&bad, &root.path().join("data")).is_err());
    assert!(guidance(&bad).is_empty());
    assert!(Catalog::new(root.path(), &bad).is_err());
}

#[test]
fn duplicate_archive_map_keys_are_rejected() {
    let good = bytes("demo", "hello");
    let raw = String::from_utf8(good).unwrap();
    let doubled = raw.replace("\"files\":{", "\"files\":{\"skill.md\":\"ignored\",");
    assert!(Archive::parse(doubled.as_bytes()).is_err());
}
