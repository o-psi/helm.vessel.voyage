use super::*;
fn repository(root: &Path) {
    let output = Command::new("git")
        .args(["init", "-b", "main"])
        .arg(root)
        .output()
        .unwrap();
    assert!(output.status.success());
}
#[test]
fn managed_preparation_is_scoped_idempotent_and_preserves_unowned_contents() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    repository(&repo);
    let resource = tmp.path().join("private");
    let manager = WorktreeManager::discover_managed(&repo, &resource)
        .unwrap()
        .unwrap();
    let other = WorktreeManager::discover_managed(&repo, &tmp.path().join("other"))
        .unwrap()
        .unwrap();
    assert_ne!(manager.root, other.root);
    assert!(!manager.root.exists());
    assert!(!resource.exists());
    assert!(manager.root.starts_with(repo.join(DIRECTORY)));
    manager.managed.as_ref().unwrap().prepare(&manager).unwrap();
    manager.managed.as_ref().unwrap().prepare(&manager).unwrap();
    let marker = manager.root.join(".gitignore");
    assert_eq!(std::fs::read(&marker).unwrap(), IGNORE);
    std::fs::write(&marker, b"operator-owned").unwrap();
    assert!(manager.managed.as_ref().unwrap().prepare(&manager).is_err());
    assert_eq!(std::fs::read(&marker).unwrap(), b"operator-owned");
    std::fs::remove_file(&marker).unwrap();
    assert!(manager.managed.as_ref().unwrap().prepare(&manager).is_err());
    assert!(!marker.exists());
}
#[test]
fn directory_creation_rejects_files_and_symlinks_without_replacing_them() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("dir");
    assert!(directory(&path).unwrap());
    assert!(!directory(&path).unwrap());
    let file = root.path().join("file");
    std::fs::write(&file, b"keep").unwrap();
    assert!(directory(&file).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
    #[cfg(unix)]
    {
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(directory(&link).is_err());
        assert!(link.is_symlink());
    }
}
#[test]
fn tracked_managed_area_is_never_adopted() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("repo");
    repository(&repo);
    std::fs::create_dir(repo.join(DIRECTORY)).unwrap();
    std::fs::write(repo.join(DIRECTORY).join("project"), b"keep").unwrap();
    assert!(
        Command::new("git")
            .current_dir(&repo)
            .args(["add", ".voyage-worktrees/project"])
            .status()
            .unwrap()
            .success()
    );
    let manager = WorktreeManager::discover_managed(&repo, &tmp.path().join("resource"))
        .unwrap()
        .unwrap();
    assert!(manager.managed.as_ref().unwrap().prepare(&manager).is_err());
    assert_eq!(
        std::fs::read(repo.join(DIRECTORY).join("project")).unwrap(),
        b"keep"
    );
    assert!(!manager.root.exists());
}
