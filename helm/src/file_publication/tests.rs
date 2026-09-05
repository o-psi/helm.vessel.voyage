use super::*;
fn directory(path: &Path) -> Dir {
    Dir::open_ambient_dir(path, cap_std::ambient_authority()).unwrap()
}
#[test]
fn rejects_paths_instead_of_basenames() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["", ".", "..", "../file", "dir/file", "/file", "file/"] {
        assert!(
            Publication::prepare(directory(temp.path()), Path::new(name)).is_err(),
            "{name}"
        );
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[cfg(not(target_os = "linux"))]
#[test]
fn unsupported_platform_has_no_publication_effects() {
    let temp = tempfile::tempdir().unwrap();
    assert!(Publication::prepare(directory(temp.path()), Path::new("config")).is_err());
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    #[test]
    fn anonymous_staging_and_abandoned_preparation_leave_no_names() {
        let temp = tempfile::tempdir().unwrap();
        let publication =
            Publication::prepare(directory(temp.path()), Path::new("config")).unwrap();
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        drop(publication);
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
    #[test]
    fn existing_file_and_dangling_symlink_are_never_replaced() {
        for symlink in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("config");
            let pending =
                Publication::prepare(directory(temp.path()), Path::new("config")).unwrap();
            if symlink {
                std::os::unix::fs::symlink("missing", &output).unwrap();
            } else {
                std::fs::write(&output, b"winner").unwrap();
            }
            assert!(pending.publish(b"loser").is_err());
            if symlink {
                assert_eq!(std::fs::read_link(&output).unwrap(), Path::new("missing"));
                assert!(!temp.path().join("missing").exists());
            } else {
                assert_eq!(std::fs::read(output).unwrap(), b"winner");
            }
        }
    }
    #[test]
    fn parent_replacement_does_not_redirect_publication() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        let retained = temp.path().join("retained");
        std::fs::create_dir(&parent).unwrap();
        let pending = Publication::prepare(directory(&parent), Path::new("config")).unwrap();
        std::fs::rename(&parent, &retained).unwrap();
        std::fs::create_dir(&parent).unwrap();
        pending.publish(b"reviewed").unwrap();
        assert_eq!(std::fs::read(retained.join("config")).unwrap(), b"reviewed");
        assert!(!parent.join("config").exists());
    }
    #[test]
    fn publication_failure_boundaries_preserve_exact_evidence() {
        for fail_after in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let output = temp.path().join("config");
            let pending =
                Publication::prepare(directory(temp.path()), Path::new("config")).unwrap();
            assert!(
                pending
                    .publish_observed(b"reviewed", |after| {
                        if after == fail_after {
                            bail!("injected failure");
                        }
                        Ok(())
                    })
                    .is_err()
            );
            assert_eq!(output.exists(), fail_after);
            let retry = Publication::prepare(directory(temp.path()), Path::new("config")).unwrap();
            if fail_after {
                assert!(retry.publish(b"retry").is_err());
                assert_eq!(std::fs::read(output).unwrap(), b"reviewed");
            } else {
                retry.publish(b"retry").unwrap();
                assert_eq!(std::fs::read(output).unwrap(), b"retry");
            }
            assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        }
    }
    #[test]
    fn concurrent_publishers_preserve_one_complete_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut workers = Vec::new();
        for byte in [b'a', b'b'] {
            let pending =
                Publication::prepare(directory(temp.path()), Path::new("config")).unwrap();
            let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                pending.publish(&vec![byte; 65536]).is_ok()
            }));
        }
        assert_eq!(
            workers
                .into_iter()
                .filter_map(|w| w.join().ok())
                .filter(|ok| *ok)
                .count(),
            1
        );
        let bytes = std::fs::read(temp.path().join("config")).unwrap();
        assert_eq!(bytes.len(), 65536);
        assert!(bytes.iter().all(|b| *b == bytes[0]));
        assert_eq!(
            std::fs::metadata(temp.path().join("config"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
