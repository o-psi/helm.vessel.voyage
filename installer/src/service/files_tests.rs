use super::*;
use crate::fixture_tests::Fixture;
use std::os::unix::fs::{PermissionsExt, symlink};
#[test]
fn private_directory_and_executable_checks() {
    let f = Fixture::new();
    let uid = unsafe { libc::geteuid() };
    let path = f.root.join("nested/private");
    directory(&path, uid, true).unwrap();
    directory(&path, uid, true).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(directory(&path, uid, true).is_err());
    directory(&path, uid, false).unwrap();
    let exe = f.script("program", "exit 0");
    executable(&exe, uid).unwrap();
    for mode in [0o600, 0o4700, 0o2700, 0o722] {
        fs::set_permissions(&exe, fs::Permissions::from_mode(mode)).unwrap();
        assert!(executable(&exe, uid).is_err());
    }
    assert!(executable(&f.root.join("missing"), uid).is_err());
    assert!(executable(&path, uid).is_err());
    assert!(check_path(Path::new("relative"), uid).is_err());
    assert!(check_path(&f.root.join("../escape"), uid).is_err());
    symlink(&path, f.root.join("link")).unwrap();
    assert!(check_path(&f.root.join("link/child"), uid).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(check_path(&path.join("missing"), uid).is_err());
}
#[test]
fn replacement_preserves_displaced_contents_and_refuses_races() {
    let f = Fixture::new();
    let path = f.root.join("unit");
    replace(&path, "first", None).unwrap();
    assert_eq!(read(&path).unwrap(), "first");
    assert!(replace(&path, "unreviewed", None).is_err());
    assert_eq!(read(&path).unwrap(), "first");
    replace(&path, "second", Some("first")).unwrap();
    assert!(
        fs::read_dir(&f.root)
            .unwrap()
            .flatten()
            .any(|e| e.path() != path && fs::read_to_string(e.path()).unwrap() == "first")
    );
    assert!(replace(&path, "third", Some("stale")).is_err());
    assert_eq!(read(&path).unwrap(), "second");
    assert!(remove_reviewed(&path, "stale").is_err());
    assert_eq!(read(&path).unwrap(), "second");
    remove_reviewed(&path, "second").unwrap();
    assert!(!path.exists());
    assert!(remove_reviewed(&path, "second").is_err());
}
#[test]
fn bounded_nofollow_read_and_rename_errors() {
    let f = Fixture::new();
    let target = f.root.join("target");
    fs::write(&target, vec![b'x'; 16385]).unwrap();
    assert!(read(&target).is_err());
    fs::write(&target, [255]).unwrap();
    assert!(read(&target).is_err());
    symlink(&target, f.root.join("link")).unwrap();
    assert!(read(&f.root.join("link")).is_err());
    assert!(read(&f.root).is_err());
    assert!(rename(Path::new("nul\0"), &target, 0).is_err());
    assert!(rename(&target, Path::new("nul\0"), 0).is_err());
    assert!(replace(&f.root.join("missing/unit"), "x", None).is_err());
}
