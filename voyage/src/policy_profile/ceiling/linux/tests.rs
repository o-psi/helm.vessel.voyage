use super::*;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
};
fn fixture() -> (tempfile::TempDir, Vec<u8>) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("policy")).unwrap();
    let document = CeilingDocument {
        schema: 1,
        rules: crate::policy_profile::Builtin::Restricted.document().rules,
    };
    let bytes = toml::to_string(&document).unwrap().into_bytes();
    fs::write(root.path().join("policy/ceiling.toml"), &bytes).unwrap();
    (root, bytes)
}
fn load(root: &std::path::Path) -> Result<Option<CeilingDocument>> {
    read(
        fs::File::open(root).unwrap(),
        &["policy", "ceiling.toml"],
        unsafe { libc::geteuid() },
        || Ok(()),
    )
}
#[test]
fn ceiling_is_read_only_optional_and_rejects_unsafe_permissions_or_links() {
    let (root, bytes) = fixture();
    assert!(load(root.path()).unwrap().is_some());
    assert_eq!(
        fs::read(root.path().join("policy/ceiling.toml")).unwrap(),
        bytes
    );
    let path = root.path().join("policy/ceiling.toml");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(load(root.path()).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let backup = root.path().join("backup");
    fs::hard_link(&path, &backup).unwrap();
    assert!(load(root.path()).is_err());
    fs::remove_file(backup).unwrap();
    fs::remove_file(&path).unwrap();
    assert!(load(root.path()).unwrap().is_none());
    symlink(root.path().join("elsewhere"), &path).unwrap();
    assert!(load(root.path()).is_err());
}
#[test]
fn ceiling_detects_replacement_during_read_and_malformed_documents() {
    let (root, _) = fixture();
    let path = root.path().join("policy/ceiling.toml");
    let result = read(
        fs::File::open(root.path()).unwrap(),
        &["policy", "ceiling.toml"],
        unsafe { libc::geteuid() },
        || {
            let old = fs::read(&path).unwrap();
            fs::rename(&path, root.path().join("retained")).unwrap();
            fs::write(&path, old).unwrap();
            Ok(())
        },
    );
    assert!(result.is_err());
    fs::write(&path, b"invalid toml").unwrap();
    assert!(load(root.path()).is_err());
    fs::write(&path, vec![b'x'; MAX_DOCUMENT + 1]).unwrap();
    assert!(load(root.path()).is_err());
    fs::remove_file(path).unwrap();
    fs::remove_dir(root.path().join("policy")).unwrap();
    assert!(load(root.path()).unwrap().is_none());
}
