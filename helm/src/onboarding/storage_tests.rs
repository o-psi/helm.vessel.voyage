use super::*;

#[test]
fn bounded_reads_and_create_only_publication_preserve_originals() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("nested")).unwrap();
    let root = Root::open(temp.path()).unwrap();
    root.publish(Path::new("nested/draft.md"), b"review me")
        .unwrap();
    assert_eq!(
        root.read(Path::new("nested/draft.md"), 9).unwrap(),
        b"review me"
    );
    assert_eq!(
        root.read(&temp.path().join("nested/draft.md"), 9).unwrap(),
        b"review me"
    );
    assert!(root.read(Path::new("nested/draft.md"), 8).is_err());
    assert!(root.read(Path::new("nested"), 100).is_err());
    assert!(
        root.publish(Path::new("nested/draft.md"), b"replace")
            .is_err()
    );
    assert_eq!(
        std::fs::read(temp.path().join("nested/draft.md")).unwrap(),
        b"review me"
    );
    root.publish(Path::new("empty"), b"").unwrap();
    assert!(root.read(Path::new("empty"), 0).unwrap().is_empty());
    let entries = root.entries(Path::new("")).unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.directory))
            .collect::<Vec<_>>(),
        [("empty", false), ("nested", true)]
    );
    for path in ["", ".", "../escape", "nested/../escape"] {
        assert!(root.read(Path::new(path), 100).is_err());
        assert!(root.publish(Path::new(path), b"no").is_err());
    }
    let outside = tempfile::tempdir().unwrap();
    assert!(root.publish(&outside.path().join("escape"), b"no").is_err());
    assert!(!outside.path().join("escape").exists());
}

#[test]
fn active_guidance_has_shared_case_guard_and_runtime_size_limit() {
    for name in ["AGENTS.md", "agents.md"] {
        let temp = tempfile::tempdir().unwrap();
        let root = Root::open(temp.path()).unwrap();
        assert!(
            root.publish(
                Path::new(name),
                &vec![b'x'; crate::workspace_instructions::MAX_BYTES as usize + 1]
            )
            .is_err()
        );
        assert!(!temp.path().join(name).exists());
        root.publish(Path::new(name), b"reviewed").unwrap();
        for other in ["AGENTS.md", "agents.md"] {
            assert!(root.publish(Path::new(other), b"replacement").is_err());
        }
        root.publish(Path::new("draft.md"), b"sidecar").unwrap();
        assert_eq!(root.read(Path::new(name), 100).unwrap(), b"reviewed");
    }
}

#[cfg(unix)]
#[test]
fn capability_walk_rejects_symlink_files_and_directories() {
    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("original"), b"preserved").unwrap();
    std::os::unix::fs::symlink(outside.path(), temp.path().join("link")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("original"), temp.path().join("file")).unwrap();
    let root = Root::open(temp.path()).unwrap();
    assert!(root.read(Path::new("file"), 100).is_err());
    assert!(root.read(Path::new("link/original"), 100).is_err());
    assert!(root.publish(Path::new("link/new"), b"no").is_err());
    assert!(root.publish(Path::new("file"), b"no").is_err());
    assert_eq!(
        std::fs::read(outside.path().join("original")).unwrap(),
        b"preserved"
    );
    assert!(!outside.path().join("new").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn publication_lock_is_exclusive_and_released_on_drop() {
    let temp = tempfile::tempdir().unwrap();
    let first = Root::open(temp.path()).unwrap();
    let second = Root::open(temp.path()).unwrap();
    let guard = instruction_guard(&first.directory, Path::new("AGENTS.md"), b"a").unwrap();
    assert!(second.publish(Path::new("agents.md"), b"b").is_err());
    second.publish(Path::new("sidecar.md"), b"b").unwrap();
    drop(guard);
    second.publish(Path::new("agents.md"), b"b").unwrap();
}
