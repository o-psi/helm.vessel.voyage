//! Private-store failure boundaries; each fixture has an independent directory.
use super::*;
use crate::{completion::Obligation, todo::TodoId};
use serde_json::json;

struct Fixture {
    root: tempfile::TempDir,
    store: RunLedgerStore,
    ledger: RunLedger,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let scope = RunScope::new(root.path(), Uuid::new_v4()).unwrap();
        let store = RunLedgerStore::open(root.path().join("runs"), scope).unwrap();
        let ledger = RunLedger::new();
        store.create(&ledger).unwrap();
        Self {
            root,
            store,
            ledger,
        }
    }
    fn path(&self) -> PathBuf {
        self.store.path(self.ledger.run_id())
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(self.path()).unwrap()
    }
    fn replace(&self, value: serde_json::Value) {
        fs::write(self.path(), serde_json::to_vec(&value).unwrap()).unwrap();
    }
    fn stored(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes()).unwrap()
    }
}

#[test]
fn create_cannot_overwrite_existing_identity() {
    let f = Fixture::new();
    let before = f.bytes();
    assert!(f.store.create(&f.ledger).is_err());
    assert_eq!(f.bytes(), before);
    assert_eq!(f.store.load(f.ledger.run_id()).unwrap(), f.ledger);
}

#[test]
fn rejected_transaction_does_not_publish_partial_mutation() {
    let f = Fixture::new();
    let before = f.bytes();
    let result = f.store.update(f.ledger.run_id(), 0, |ledger| {
        ledger.adopt(Obligation::Todo(TodoId::new()), 0)?;
        anyhow::bail!("fixture rejection after mutation")
    });
    assert!(result.is_err());
    assert_eq!(f.bytes(), before);
    assert_eq!(f.store.load(f.ledger.run_id()).unwrap().revision(), 0);
}

#[test]
fn transaction_may_not_replace_run_identity() {
    let f = Fixture::new();
    let before = f.bytes();
    assert!(
        f.store
            .update(f.ledger.run_id(), 0, |ledger| {
                *ledger = RunLedger::new();
                Ok(())
            })
            .is_err()
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn committed_transaction_returns_value_and_reopens_durably() {
    let f = Fixture::new();
    let id = TodoId::new();
    let answer = f
        .store
        .update(f.ledger.run_id(), 0, |ledger| {
            ledger.adopt(Obligation::Todo(id), 0)?;
            Ok(())
        })
        .unwrap();
    assert_eq!(answer.revision(), 1);
    let reopened = RunLedgerStore::open(f.root.path().join("runs"), f.store.scope.clone()).unwrap();
    let ledger = reopened.load(f.ledger.run_id()).unwrap();
    assert_eq!(ledger.revision(), 1);
    assert_eq!(
        ledger.obligations().collect::<Vec<_>>(),
        vec![Obligation::Todo(id)]
    );
}

#[test]
fn lock_contention_refuses_load_create_and_transaction_without_changes() {
    let f = Fixture::new();
    let before = f.bytes();
    let lock = f.store.lock().unwrap();
    assert!(f.store.load(f.ledger.run_id()).is_err());
    assert!(f.store.create(&RunLedger::new()).is_err());
    assert!(f.store.update(f.ledger.run_id(), 0, |_| Ok(())).is_err());
    assert_eq!(f.bytes(), before);
    drop(lock);
    assert!(f.store.load(f.ledger.run_id()).is_ok());
}

#[test]
fn missing_known_run_does_not_create_replacement() {
    let f = Fixture::new();
    let missing = RunId::default();
    assert!(f.store.load(missing).is_err());
    assert!(f.store.update(missing, 0, |_| Ok(())).is_err());
    assert!(!f.store.path(missing).exists());
}

#[test]
fn malformed_envelope_and_ledger_are_never_repaired() {
    for bytes in [b"".as_slice(), b"null", b"{}", b"{", b"[]", b"\xff"] {
        let f = Fixture::new();
        fs::write(f.path(), bytes).unwrap();
        assert!(f.store.load(f.ledger.run_id()).is_err());
        assert_eq!(f.bytes(), bytes);
    }
    let f = Fixture::new();
    let mut value = f.stored();
    value["ledger"] = json!("{}");
    f.replace(value);
    assert!(f.store.load(f.ledger.run_id()).is_err());
}

#[test]
fn envelope_rejects_unknown_fields_versions_scope_and_identity() {
    for case in 0..5 {
        let f = Fixture::new();
        let mut value = f.stored();
        match case {
            0 => value["version"] = json!(999),
            1 => value["unexpected"] = json!(true),
            2 => value["scope"]["session_id"] = json!(Uuid::new_v4()),
            3 => value["scope"]["workspace"] = json!(f.root.path().join("other")),
            _ => {
                value["ledger"] =
                    json!(String::from_utf8(RunLedger::new().to_json().unwrap()).unwrap())
            }
        }
        f.replace(value);
        let before = f.bytes();
        assert!(f.store.load(f.ledger.run_id()).is_err(), "case {case}");
        assert_eq!(f.bytes(), before);
    }
}

#[test]
fn oversized_store_is_rejected_before_decoding() {
    let f = Fixture::new();
    let file = OpenOptions::new().write(true).open(f.path()).unwrap();
    file.set_len(MAX_STORE_BYTES as u64 + 1).unwrap();
    assert!(
        f.store
            .load(f.ledger.run_id())
            .unwrap_err()
            .to_string()
            .contains("byte limit")
    );
}

#[test]
fn nonregular_ledger_is_not_replaced() {
    let f = Fixture::new();
    fs::remove_file(f.path()).unwrap();
    fs::create_dir(f.path()).unwrap();
    assert!(f.store.load(f.ledger.run_id()).is_err());
    assert!(f.store.create(&f.ledger).is_err());
    assert!(f.path().is_dir());
}

#[test]
fn scope_requires_an_existing_directory() {
    let root = tempfile::tempdir().unwrap();
    assert!(RunScope::new(&root.path().join("absent"), Uuid::new_v4()).is_err());
    let file = root.path().join("file");
    fs::write(&file, "x").unwrap();
    assert!(RunScope::new(&file, Uuid::new_v4()).is_err());
}

#[cfg(unix)]
#[test]
fn symlink_ledger_and_lock_never_follow_external_target() {
    use std::os::unix::fs::symlink;
    for lock in [false, true] {
        let f = Fixture::new();
        let target = f.root.path().join("outside");
        fs::write(&target, b"preserve me").unwrap();
        let path = if lock {
            f.store.directory.join("writer.lock")
        } else {
            f.path()
        };
        fs::remove_file(&path).unwrap();
        symlink(&target, &path).unwrap();
        assert!(f.store.load(f.ledger.run_id()).is_err());
        assert!(f.store.create(&f.ledger).is_err());
        assert_eq!(fs::read(target).unwrap(), b"preserve me");
    }
}

#[cfg(unix)]
#[test]
fn public_file_permissions_are_refused() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    fs::set_permissions(f.path(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(f.store.load(f.ledger.run_id()).is_err());
    fs::set_permissions(f.path(), fs::Permissions::from_mode(0o600)).unwrap();
    assert!(f.store.load(f.ledger.run_id()).is_ok());
}

#[test]
fn stale_revision_never_invokes_mutation() {
    let f = Fixture::new();
    let mut invoked = false;
    let result = f.store.update(f.ledger.run_id(), 1, |_| {
        invoked = true;
        Ok(())
    });
    assert!(result.is_err());
    assert!(!invoked);
    assert_eq!(f.store.load(f.ledger.run_id()).unwrap(), f.ledger);
}

#[test]
fn no_op_update_preserves_exact_stored_bytes() {
    let f = Fixture::new();
    let before = f.bytes();
    assert_eq!(
        f.store.update(f.ledger.run_id(), 0, |_| Ok(())).unwrap(),
        f.ledger
    );
    assert_eq!(f.bytes(), before);
}

#[test]
fn membership_removal_and_revision_regression_are_atomic_failures() {
    for case in 0..3 {
        let f = Fixture::new();
        f.store
            .update(f.ledger.run_id(), 0, |ledger| {
                ledger.adopt(Obligation::Todo(TodoId::new()), 0)
            })
            .unwrap();
        let before = f.bytes();
        let result = f.store.update(f.ledger.run_id(), 1, |ledger| {
            match case {
                0 => ledger.revision = 0,
                1 => {
                    ledger.entries.clear();
                    ledger.revision += 1;
                }
                _ => ledger.entries.clear(),
            }
            Ok(())
        });
        assert!(result.is_err(), "case {case}");
        assert_eq!(f.bytes(), before);
        assert_eq!(
            f.store
                .load(f.ledger.run_id())
                .unwrap()
                .obligations()
                .count(),
            1
        );
    }
}

#[cfg(unix)]
#[test]
fn directory_permissions_and_symlink_aliases_are_not_silently_fixed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let scope = RunScope::new(root.path(), Uuid::new_v4()).unwrap();
    let directory = root.path().join("public");
    fs::create_dir(&directory).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(RunLedgerStore::open(directory.clone(), scope.clone()).is_err());
    assert_eq!(
        fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
        0o755
    );
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(RunLedgerStore::open(directory.clone(), scope.clone()).is_ok());
    let alias = root.path().join("alias");
    symlink(&directory, &alias).unwrap();
    assert!(RunLedgerStore::open(alias, scope).is_err());
}
