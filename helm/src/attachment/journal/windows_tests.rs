use super::*;

fn grant_everyone(path: &Path) {
    assert!(
        std::process::Command::new("icacls.exe")
            .arg(path)
            .args(["/grant", "*S-1-1-0:(F)"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}

#[test]
fn insecure_directory_database_journal_wal_shm_and_execution_sidecars_fail_closed() {
    for name in [
        "directory",
        "journal.sqlite3",
        "journal.sqlite3-journal",
        "journal.sqlite3-wal",
        "journal.sqlite3-shm",
        "execution",
    ] {
        let (dir, journal, session, _) = setup();
        let directory = journal.directory.clone();
        let file = match name {
            "directory" => directory.clone(),
            "execution" => {
                drop(journal.acquire_execution(session.id).unwrap());
                directory.join(format!("{}.execution.lock", session.id))
            }
            name => directory.join(name),
        };
        if !file.exists() {
            fs::write(&file, b"do not erase insecure sidecar").unwrap();
        }
        drop(journal);
        let original = if file.is_file() {
            Some(fs::read(&file).unwrap())
        } else {
            None
        };
        grant_everyone(&file);
        if name == "execution" {
            let reopened = Journal::open(directory).unwrap();
            assert!(reopened.acquire_execution(session.id).is_err());
        } else {
            assert!(
                Journal::open(directory).is_err(),
                "accepted insecure {name}"
            );
        }
        if let Some(original) = original {
            assert_eq!(fs::read(&file).unwrap(), original);
        }
        drop(dir);
    }
}

#[test]
fn hardlinked_sqlite_and_execution_sidecars_fail_without_mutation() {
    for name in ["journal.sqlite3", "journal.sqlite3-journal", "execution"] {
        let (dir, journal, session, _) = setup();
        let path = if name == "execution" {
            drop(journal.acquire_execution(session.id).unwrap());
            journal
                .directory
                .join(format!("{}.execution.lock", session.id))
        } else {
            journal.directory.join(name)
        };
        let bytes = fs::read(&path).unwrap();
        fs::hard_link(&path, dir.path().join("alias")).unwrap();
        if name == "execution" {
            assert!(journal.acquire_execution(session.id).is_err());
        } else {
            drop(journal);
            assert!(Journal::open(dir.path().join("attachment")).is_err());
        }
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn guard_keeps_checked_ancestors_pinned_after_journal_drop() {
    let root = tempfile::tempdir().unwrap();
    let parent = root.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let path = parent.join("journal");
    let mut journal = Journal::open(path.clone()).unwrap();
    let session = Session::new(root.path().into(), "offline".into());
    journal.create_session(&session).unwrap();
    let guard = journal.acquire_execution(session.id).unwrap();
    drop(journal);
    assert!(fs::rename(&parent, root.path().join("moved")).is_err());
    assert!(fs::rename(&path, parent.join("moved")).is_err());
    let reopened = Journal::open(path).unwrap();
    assert!(reopened.acquire_execution(session.id).is_err());
    drop(reopened);
    drop(guard);
    fs::rename(&parent, root.path().join("moved")).unwrap();
    let reopened = Journal::open(root.path().join("moved/journal")).unwrap();
    reopened.acquire_execution(session.id).unwrap();
}

#[test]
fn junction_ancestor_is_rejected_before_store_creation() {
    let root = tempfile::tempdir().unwrap();
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    let alias = root.path().join("alias");
    assert!(
        std::process::Command::new("cmd.exe")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&alias)
            .arg(&real)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    assert!(Journal::open(alias.join("private")).is_err());
    assert!(!real.join("private").exists());
    fs::remove_dir(alias).unwrap();
}
