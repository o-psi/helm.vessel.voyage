//! Filesystem fault boundaries in a private fixture; no extension is launched.
use super::*;
#[test]
fn failed_publication_before_rename_preserves_previous_bytes_and_cleans_temporary() {
    for stopped in [0, 1, 2] {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        publish(&dir, "catalog.json", b"retained original").unwrap();
        let mut observed = Vec::new();
        let result = publish_observed(&dir, "catalog.json", b"candidate", |phase| {
            observed.push(phase);
            if phase == stopped {
                anyhow::bail!("private injected failure");
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(
            read(&dir, "catalog.json", 64).unwrap().unwrap(),
            b"retained original"
        );
        assert_eq!(observed.last(), Some(&stopped));
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
#[test]
fn post_rename_failure_is_observed_as_committed_bytes_without_rollback_or_retry() {
    let root = tempfile::tempdir().unwrap();
    let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    publish(&dir, "catalog.json", b"original").unwrap();
    let result = publish_observed(&dir, "catalog.json", b"committed candidate", |phase| {
        if phase == 3 {
            anyhow::bail!("reply lost after rename");
        }
        Ok(())
    });
    assert!(result.is_err());
    assert_eq!(
        read(&dir, "catalog.json", 64).unwrap().unwrap(),
        b"committed candidate"
    );
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}
#[test]
fn read_only_directory_discovery_never_creates_missing_catalog_ancestors() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        directory(root.path(), &["nested", "catalog"], false)
            .unwrap()
            .is_none()
    );
    assert!(!root.path().join("nested").exists());
    let dir = directory(root.path(), &["nested", "catalog"], true)
        .unwrap()
        .unwrap();
    assert!(read(&dir, "absent.json", 8).unwrap().is_none());
    assert!(!root.path().join("nested/catalog/absent.json").exists());
}
#[test]
fn exact_file_limit_is_inclusive_and_nonregular_inputs_refuse() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("bytes");
    std::fs::write(&file, b"eight---").unwrap();
    assert_eq!(local_bounded(&file, 8).unwrap(), b"eight---");
    assert!(local_bounded(&file, 7).is_err());
    assert!(local_bounded(root.path(), 8).is_err());
    let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
    assert!(read(&dir, "bytes", 7).is_err());
    assert_eq!(read(&dir, "bytes", 8).unwrap().unwrap(), b"eight---");
}
#[cfg(unix)]
#[test]
fn catalog_symlink_and_fifo_inputs_refuse_without_following_or_waiting() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private"), b"outside bytes").unwrap();
    symlink(outside.path().join("private"), root.path().join("link")).unwrap();
    assert!(local_bounded(&root.path().join("link"), 32).is_err());
    symlink(outside.path(), root.path().join("directory-link")).unwrap();
    assert!(directory(root.path(), &["directory-link"], false).is_err());
    let fifo = root.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(local_bounded(&fifo, 32).is_err());
    assert_eq!(
        std::fs::read(outside.path().join("private")).unwrap(),
        b"outside bytes"
    );
}
