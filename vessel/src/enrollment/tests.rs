use super::*;
use voyage_protocol::enrollment::SigningKey;
const ORIGIN: &str = "https://vessel.example";
fn store() -> EnrollmentStore {
    EnrollmentStore::initialize(Connection::open_in_memory().unwrap(), ORIGIN.into()).unwrap()
}
fn proof(key: &SigningKey, c: Challenge) -> SignedChallenge {
    SignedChallenge {
        signature: key.sign(&c).unwrap(),
        challenge: c,
        new_signature: None,
    }
}
fn enroll(store: &mut EnrollmentStore) -> (SigningKey, Receipt, ProofOperation) {
    let key = SigningKey::generate().unwrap();
    let invite = store.invite(900_000, 1).unwrap();
    let op = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: invite.id,
        public_key: key.public_key(),
    };
    let c = store.challenge(op.clone(), Some(&invite.key), 1).unwrap();
    let receipt = store
        .complete(&proof(&key, c), Some(&invite.key), 1)
        .unwrap();
    (key, receipt, op)
}
#[test]
fn invitation_is_one_use_but_original_key_bound_recovery_is_finite() {
    let mut s = store();
    let (key, receipt, op) = enroll(&mut s);
    assert_eq!(receipt.owner_id, s.owner_id());
    assert_eq!(receipt.epoch, 1);
    assert!(!receipt.revoked);
    let c = s.challenge(op.clone(), None, 2).unwrap();
    let p = proof(&key, c);
    assert_eq!(s.complete(&p, None, 2).unwrap(), receipt);
    assert_eq!(
        s.complete(&p, None, 2).unwrap_err(),
        EnrollmentError::Denied
    );
    let mut altered = op.clone();
    if let ProofOperation::Enroll { transaction_id, .. } = &mut altered {
        *transaction_id = Uuid::new_v4();
    }
    assert!(s.challenge(altered, None, 2).is_err());
    let mut altered = op.clone();
    if let ProofOperation::Enroll { public_key, .. } = &mut altered {
        *public_key = SigningKey::generate().unwrap().public_key();
    }
    assert_eq!(
        s.challenge(altered, None, 2).unwrap_err(),
        EnrollmentError::Conflict
    );
    let c = s.challenge(op.clone(), None, RECOVERY_MS).unwrap();
    assert_eq!(
        s.complete(&proof(&key, c), None, RECOVERY_MS + 1)
            .unwrap_err(),
        EnrollmentError::Conflict
    );
    assert!(s.challenge(op, None, RECOVERY_MS + 1).is_err());
}
#[test]
fn stolen_join_key_and_altered_challenges_cannot_replace_an_enrollment() {
    let mut s = store();
    let invite = s.invite(100, 1).unwrap();
    let key = SigningKey::generate().unwrap();
    let op = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: invite.id,
        public_key: key.public_key(),
    };
    assert!(s.challenge(op.clone(), Some("wrong"), 1).is_err());
    let c = s.challenge(op.clone(), Some(&invite.key), 1).unwrap();
    let mut altered = c.clone();
    altered.expires_at_ms += 1;
    assert!(
        s.complete(&proof(&key, altered), Some(&invite.key), 1)
            .is_err()
    );
    assert!(
        s.complete(
            &proof(&SigningKey::generate().unwrap(), c.clone()),
            Some(&invite.key),
            1
        )
        .is_err()
    );
    s.complete(&proof(&key, c), Some(&invite.key), 1).unwrap();
    let attacker = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: invite.id,
        public_key: SigningKey::generate().unwrap().public_key(),
    };
    assert!(s.challenge(attacker, Some(&invite.key), 1).is_err());
    let hashes: Vec<u8> =
        s.db.query_row("SELECT key_hash FROM invitations", [], |r| r.get(0))
            .unwrap();
    assert_eq!(hashes, hash(invite.key.as_bytes()));
    let audit: String =
        s.db.query_row("SELECT group_concat(kind) FROM audit", [], |r| r.get(0))
            .unwrap();
    assert!(!audit.contains(&invite.key));
}
#[test]
fn rotation_invalidates_old_challenges_and_requires_both_keys_even_for_recovery() {
    let mut s = store();
    let (old, receipt, _) = enroll(&mut s);
    let new = SigningKey::generate().unwrap();
    let connect = s
        .challenge(
            ProofOperation::Connect {
                machine_id: receipt.machine_id,
                epoch: 1,
            },
            None,
            1,
        )
        .unwrap();
    let op = ProofOperation::Rotate {
        machine_id: receipt.machine_id,
        epoch: 1,
        transaction_id: Uuid::new_v4(),
        new_public_key: new.public_key(),
    };
    let c = s.challenge(op.clone(), None, 1).unwrap();
    let mut p = proof(&old, c);
    assert!(s.complete(&p, None, 1).is_err());
    p.new_signature = Some(new.sign(&p.challenge).unwrap());
    let rotated = s.complete(&p, None, 1).unwrap();
    assert_eq!(rotated.epoch, 2);
    assert!(s.current(receipt.machine_id, 1).is_err());
    assert!(s.complete(&proof(&old, connect), None, 1).is_err());
    let c = s.challenge(op, None, 2).unwrap();
    let mut p = proof(&old, c);
    p.new_signature = Some(new.sign(&p.challenge).unwrap());
    assert_eq!(s.complete(&p, None, 2).unwrap(), rotated);
    let c = s
        .challenge(
            ProofOperation::Connect {
                machine_id: receipt.machine_id,
                epoch: 2,
            },
            None,
            2,
        )
        .unwrap();
    assert_eq!(s.complete(&proof(&new, c), None, 2).unwrap(), rotated);
}
#[test]
fn revocation_fences_connections_and_all_prior_recovery_without_deleting_identity() {
    let mut s = store();
    let (key, r, enrollment) = enroll(&mut s);
    let c = s
        .challenge(
            ProofOperation::Connect {
                machine_id: r.machine_id,
                epoch: 1,
            },
            None,
            1,
        )
        .unwrap();
    let transaction = Uuid::new_v4();
    let revoked = s.revoke(r.machine_id, 1, transaction, 2).unwrap();
    assert!(revoked.revoked);
    assert_eq!(revoked.epoch, 2);
    assert_eq!(s.revoke(r.machine_id, 1, transaction, 2).unwrap(), revoked);
    assert!(s.current(r.machine_id, 1).is_err());
    assert!(s.current(r.machine_id, 2).is_err());
    assert!(s.complete(&proof(&key, c), None, 2).is_err());
    assert!(s.challenge(enrollment, None, 2).is_err());
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM machines", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let c = s
        .challenge(
            ProofOperation::Revoke {
                machine_id: r.machine_id,
                epoch: 1,
                transaction_id: transaction,
            },
            None,
            2,
        )
        .unwrap();
    assert_eq!(s.complete(&proof(&key, c), None, 2).unwrap(), revoked);
}
#[test]
fn signed_revocation_and_no_same_key_rotation() {
    let mut s = store();
    let (key, r, _) = enroll(&mut s);
    let op = ProofOperation::Rotate {
        machine_id: r.machine_id,
        epoch: 1,
        transaction_id: Uuid::new_v4(),
        new_public_key: key.public_key(),
    };
    let c = s.challenge(op, None, 1).unwrap();
    let mut p = proof(&key, c);
    p.new_signature = Some(key.sign(&p.challenge).unwrap());
    assert_eq!(
        s.complete(&p, None, 1).unwrap_err(),
        EnrollmentError::Invalid
    );
    let c = s
        .challenge(
            ProofOperation::Revoke {
                machine_id: r.machine_id,
                epoch: 1,
                transaction_id: Uuid::new_v4(),
            },
            None,
            1,
        )
        .unwrap();
    assert!(s.complete(&proof(&key, c), None, 1).unwrap().revoked);
}
#[test]
fn admission_and_challenge_consumption_roll_back_together_on_storage_failure() {
    let mut s = store();
    let i = s.invite(100, 1).unwrap();
    let k = SigningKey::generate().unwrap();
    let op = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: i.id,
        public_key: k.public_key(),
    };
    let c = s.challenge(op, Some(&i.key), 1).unwrap();
    let p = proof(&k, c);
    s.db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON receipts BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert_eq!(
        s.complete(&p, Some(&i.key), 1).unwrap_err(),
        EnrollmentError::Storage
    );
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM machines", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM challenges", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    s.db.execute_batch("DROP TRIGGER fail_receipt").unwrap();
    s.complete(&p, Some(&i.key), 1).unwrap();
}
#[test]
fn expiry_clock_rollback_and_capacity_are_explicit_and_bounded() {
    let mut s = store();
    assert!(s.invite(0, 1).is_err());
    assert!(s.invite(900_001, 1).is_err());
    let i = s.invite(1, 1).unwrap();
    let k = SigningKey::generate().unwrap();
    let op = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: i.id,
        public_key: k.public_key(),
    };
    assert!(s.challenge(op, Some(&i.key), 2).is_err());
    assert!(s.invite(1, 0).is_err());
    assert!(s.invite(1, i64::MAX).is_err());
    let mut s = store();
    let (key, r, _) = enroll(&mut s);
    for _ in 0..15 {
        let c = s
            .challenge(
                ProofOperation::Connect {
                    machine_id: r.machine_id,
                    epoch: 1,
                },
                None,
                1,
            )
            .unwrap();
        s.complete(&proof(&key, c), None, 1).unwrap();
    }
    let c = s
        .challenge(
            ProofOperation::Connect {
                machine_id: r.machine_id,
                epoch: 1,
            },
            None,
            1,
        )
        .unwrap();
    assert_eq!(
        s.complete(&proof(&key, c), None, 1).unwrap_err(),
        EnrollmentError::Capacity
    );
    let c = s
        .challenge(
            ProofOperation::Connect {
                machine_id: r.machine_id,
                epoch: 1,
            },
            None,
            60_001,
        )
        .unwrap();
    s.complete(&proof(&key, c), None, 60_001).unwrap();
}
#[test]
fn origin_policy_rejects_credentials_paths_queries_and_nonloopback_http() {
    for origin in [
        "http://example.com",
        "https://u:p@example.com",
        "https://example.com/path",
        "https://example.com/?token=x",
        "https://example.com/#x",
        "file:///tmp/test",
        "http://localhost",
        " https://example.com",
        "https://example.com\n",
    ] {
        assert!(validate_origin(origin, true).is_err(), "{origin:?}");
    }
    assert_eq!(
        validate_origin("https://EXAMPLE.COM:443/", false).unwrap(),
        "https://example.com"
    );
    assert!(validate_origin("http://127.0.0.1:9480", false).is_err());
    assert!(validate_origin("http://127.0.0.1:9480", true).is_ok());
    assert!(validate_origin("http://[::1]:9480", true).is_ok());
}
#[test]
fn unknown_database_is_not_adopted_and_schema_origin_mismatches_fail() {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(
        "CREATE TABLE control_plane(value TEXT);INSERT INTO control_plane VALUES('legacy');",
    )
    .unwrap();
    assert!(EnrollmentStore::initialize(db, ORIGIN.into()).is_err());
    let s = store();
    s.db.execute("UPDATE enrollment_schema SET version=99", [])
        .unwrap();
    assert!(EnrollmentStore::initialize(s.db, ORIGIN.into()).is_err());
}
#[test]
#[cfg(unix)]
fn private_storage_restart_lost_response_revocation_and_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority");
    let mut s = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    let (key, r, op) = enroll(&mut s);
    drop(s);
    let mut s = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    assert_eq!(s.current(r.machine_id, 1).unwrap(), r);
    let c = s.challenge(op, None, 2).unwrap();
    s.complete(&proof(&key, c), None, 2).unwrap();
    s.revoke(r.machine_id, 1, Uuid::new_v4(), 2).unwrap();
    drop(s);
    let s = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    assert!(s.current(r.machine_id, 1).is_err());
    drop(s);
    assert!(EnrollmentStore::open(&path, "https://other.example", false).is_err());
    let alias = dir.path().join("alias");
    symlink(&path, &alias).unwrap();
    assert!(EnrollmentStore::open(&alias, ORIGIN, false).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(EnrollmentStore::open(&path, ORIGIN, false).is_err());
}
#[test]
#[cfg(any(unix, windows))]
fn independent_connections_serialize_competing_redemption_and_busy_is_not_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("authority");
    let mut a = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    let i = a.invite(1000, 1).unwrap();
    let key = SigningKey::generate().unwrap();
    let op = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: i.id,
        public_key: key.public_key(),
    };
    let c = a.challenge(op, Some(&i.key), 1).unwrap();
    let p = proof(&key, c);
    let mut b = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    a.db.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(
        b.complete(&p, Some(&i.key), 1).unwrap_err(),
        EnrollmentError::Busy
    );
    a.db.execute_batch("ROLLBACK").unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let join = std::thread::spawn({
        let barrier = barrier.clone();
        let p = p.clone();
        let secret = i.key.clone();
        move || {
            barrier.wait();
            b.complete(&p, Some(&secret), 1)
        }
    });
    barrier.wait();
    let first = a.complete(&p, Some(&i.key), 1);
    let second = join.join().unwrap();
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        a.db.query_row("SELECT count(*) FROM machines", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn observed_expiry_stays_expired_after_clock_rollback() {
    let mut s = store();
    let i = s.invite(1, 1).unwrap();
    let k = SigningKey::generate().unwrap();
    let op = ProofOperation::Enroll {
        machine_id: Uuid::new_v4(),
        transaction_id: Uuid::new_v4(),
        invitation_id: i.id,
        public_key: k.public_key(),
    };
    let c = s.challenge(op.clone(), Some(&i.key), 1).unwrap();
    assert!(s.challenge(op.clone(), Some(&i.key), 2).is_err());
    assert_eq!(
        s.challenge(op, Some(&i.key), 1).unwrap_err(),
        EnrollmentError::Invalid
    );
    assert_eq!(
        s.complete(&proof(&k, c), Some(&i.key), 1).unwrap_err(),
        EnrollmentError::Invalid
    );
    let mut s = store();
    let (k, r, _) = enroll(&mut s);
    let c = s
        .challenge(
            ProofOperation::Connect {
                machine_id: r.machine_id,
                epoch: 1,
            },
            None,
            1,
        )
        .unwrap();
    let p = proof(&k, c);
    assert!(s.complete(&p, None, 60_001).is_err());
    assert_eq!(
        s.complete(&p, None, 2).unwrap_err(),
        EnrollmentError::Invalid
    );
}

#[test]
fn unauthenticated_challenge_requests_cannot_reserve_victim_slots() {
    let mut s = store();
    let (k, r, _) = enroll(&mut s);
    for _ in 0..100 {
        s.challenge(
            ProofOperation::Connect {
                machine_id: r.machine_id,
                epoch: 1,
            },
            None,
            1,
        )
        .unwrap();
    }
    assert_eq!(
        s.db.query_row("SELECT count(*) FROM challenges", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let c = s
        .challenge(
            ProofOperation::Connect {
                machine_id: r.machine_id,
                epoch: 1,
            },
            None,
            1,
        )
        .unwrap();
    s.complete(&proof(&k, c), None, 1).unwrap();
}

#[test]
fn reserved_revocation_capacity_survives_full_ordinary_receipts_and_audit() {
    for exhausted in ["receipts", "audit"] {
        let mut s = store();
        let (_, r, _) = enroll(&mut s);
        match exhausted {
            "receipts" => {
                s.db.execute("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO receipts SELECT 'fixture-'||x,zeroblob(32),zeroblob(32),?1,1,0,1 FROM n",[r.machine_id.to_string()]).unwrap();
            }
            _ => {
                s.db.execute_batch("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<100000) INSERT INTO audit(machine_id,kind,time) SELECT NULL,'fixture',1 FROM n;").unwrap();
            }
        }
        let tx = Uuid::new_v4();
        assert!(s.revoke(r.machine_id, 1, tx, 2).unwrap().revoked);
        assert!(s.current(r.machine_id, 1).is_err());
        let count: i64 =
            s.db.query_row("SELECT count(*) FROM audit", [], |r| r.get(0))
                .unwrap();
        s.revoke(r.machine_id, 1, tx, 2).unwrap();
        assert_eq!(
            s.db.query_row("SELECT count(*) FROM audit", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            count
        );
    }
}

#[test]
#[cfg(windows)]
fn native_private_sqlite_restart_and_journal_permissions() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("authority");
    let mut store = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    let owner = store.owner_id();
    let (key, receipt, operation) = enroll(&mut store);
    let private = store._private_directory.as_ref().unwrap();
    private.open_file("enrollment.sqlite3", false).unwrap();
    private
        .open_file("enrollment.sqlite3-journal", false)
        .unwrap();
    let mode: String = store
        .db
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "persist");
    drop(store);
    let mut reopened = EnrollmentStore::open(&path, ORIGIN, false).unwrap();
    assert_eq!(reopened.owner_id(), owner);
    assert_eq!(
        reopened.current(receipt.machine_id, receipt.epoch).unwrap(),
        receipt
    );
    let challenge = reopened.challenge(operation, None, 2).unwrap();
    assert_eq!(
        reopened.complete(&proof(&key, challenge), None, 2).unwrap(),
        receipt
    );
}

#[test]
#[cfg(windows)]
fn native_sqlite_linked_database_and_journal_are_rejected() {
    for file in ["enrollment.sqlite3", "enrollment.sqlite3-journal"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("authority");
        drop(EnrollmentStore::open(&path, ORIGIN, false).unwrap());
        let original = fs::read(path.join(file)).unwrap();
        fs::hard_link(path.join(file), path.join("linked-alias")).unwrap();
        assert!(EnrollmentStore::open(&path, ORIGIN, false).is_err());
        assert_eq!(fs::read(path.join(file)).unwrap(), original);
    }
}
