//! Actual capacity/recovery APIs on private child fixtures. Metadata here is
//! synthetic input to refusal/recovery contracts, not native guardian evidence.
use super::*;
use serde_json::{Value, json};
use std::os::unix::fs::{PermissionsExt, symlink};

fn directory(path: &Path) {
    fs::create_dir_all(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn write(path: &Path, value: &Value) {
    private(path, value.to_string().as_bytes());
}
fn child(name: &str, body: impl FnOnce()) {
    crate::host_browser::owned_child_tests::run(name, body);
}

struct Fixture {
    session: Uuid,
    session_dir: PathBuf,
    worker: PathBuf,
    capacity: PathBuf,
    original: Vec<u8>,
    foreign: Uuid,
}
impl Fixture {
    fn new() -> Self {
        let session = Uuid::new_v4();
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let session_dir = home.join("sessions").join(session.to_string());
        let worker = session_dir.join("journal/host-browser");
        directory(&worker);
        write(
            &session_dir.join("registration.json"),
            &json!({"session_id":session}),
        );
        write(
            &session_dir.join("stopped.json"),
            &json!({"session_id":session,"suspended":true,"cleanup_observed":true}),
        );
        let original = json!({"pid":i32::MAX,"fixture_only":true,"retained":"exact-lock"})
            .to_string()
            .into_bytes();
        private(&worker.join("worker.lock"), &original);
        write(&worker.join("guardian-cleanup.json"), &Self::marker());
        let capacity = crate::config::default_data_dir().join("host-browser-capacity");
        let foreign = Uuid::new_v4();
        for owner in [session, session, foreign] {
            drop(Capacity::acquire(&capacity, owner, 3).unwrap());
        }
        Self {
            session,
            session_dir,
            worker,
            capacity,
            original,
            foreign,
        }
    }
    fn marker() -> Value {
        json!({"observed":true,"cleanup_complete":true,"descendants_terminated":true,"descendants_reaped":true,"temporary_cleaned":true,"fixture_only":true})
    }
    fn slots(&self) -> Vec<Vec<u8>> {
        (0..3)
            .map(|n| fs::read(self.capacity.join(format!("browser-slot-{n}"))).unwrap())
            .collect()
    }
    fn audits(&self) -> Vec<PathBuf> {
        fs::read_dir(&self.session_dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("browser-capacity-recovery-")
            })
            .collect()
    }
    fn retained(&self, slots: &[Vec<u8>]) {
        assert_eq!(self.slots(), slots);
        assert_eq!(
            fs::read(self.worker.join("worker.lock")).unwrap(),
            self.original
        );
        assert!(self.audits().is_empty());
    }
    fn recover(&self) -> Result<Value> {
        recover(
            &self.session_dir,
            self.session,
            true,
            "owned synthetic fixture observation",
        )
    }
    fn automatic(&self) -> Result<Capacity> {
        let next = Uuid::new_v4();
        Capacity::acquire_for_session(
            &self.capacity,
            &self.session_dir.parent().unwrap().join(next.to_string()),
            next,
            3,
        )
    }
}

#[test]
fn explicit_recovery_releases_only_exact_owned_slots_and_retains_original_lock_audit_uncertainty() {
    child(
        "host_browser_capacity::recovery_tests::explicit_recovery_releases_only_exact_owned_slots_and_retains_original_lock_audit_uncertainty",
        || {
            let f = Fixture::new();
            let before = f.slots();
            let record = f.recover().unwrap();
            assert_eq!(record["session_id"], json!(f.session));
            assert_eq!(record["slots"], json!([0, 1]));
            assert_eq!(record["observed_no_descendants"], true);
            assert_eq!(record["external_effects_reconciled"], false);
            assert!(
                !f.capacity.join("browser-slot-0").exists()
                    && !f.capacity.join("browser-slot-1").exists()
            );
            assert_eq!(
                fs::read(f.capacity.join("browser-slot-2")).unwrap(),
                before[2]
            );
            let id: Uuid = serde_json::from_value(record["recovery_id"].clone()).unwrap();
            assert!(!f.worker.join("worker.lock").exists());
            assert_eq!(
                fs::read(f.worker.join(format!("worker.lock.recovered-{id}"))).unwrap(),
                f.original
            );
            assert_eq!(f.audits().len(), 1);
            assert_eq!(
                serde_json::from_slice::<Value>(&fs::read(&f.audits()[0]).unwrap()).unwrap(),
                record
            );
            let replacement = Capacity::acquire(&f.capacity, Uuid::new_v4(), 3).unwrap();
            replacement.release().unwrap();
            assert_eq!(
                fs::read(f.capacity.join("browser-slot-2")).unwrap(),
                f.foreign.to_string().as_bytes()
            );
        },
    );
}

#[test]
fn explicit_identity_observation_reason_and_missing_directory_refuse_before_capacity_or_worker_mutation()
 {
    child(
        "host_browser_capacity::recovery_tests::explicit_identity_observation_reason_and_missing_directory_refuse_before_capacity_or_worker_mutation",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            for (session, observed, reason) in [
                (Uuid::nil(), true, "owned"),
                (f.session, false, "owned"),
                (f.session, true, "  "),
                (f.session, true, &"x".repeat(513)),
            ] {
                assert!(recover(&f.session_dir, session, observed, reason).is_err());
                f.retained(&slots);
            }
            assert!(
                recover(
                    &f.session_dir.join("missing"),
                    f.session,
                    true,
                    "owned fixture"
                )
                .is_err()
            );
            f.retained(&slots);
        },
    );
}

#[test]
fn registration_and_stopped_state_adverse_matrix_preserves_exact_reservations_and_worker() {
    child(
        "host_browser_capacity::recovery_tests::registration_and_stopped_state_adverse_matrix_preserves_exact_reservations_and_worker",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            let registration = f.session_dir.join("registration.json");
            let stopped = f.session_dir.join("stopped.json");
            let valid_registration = fs::read(&registration).unwrap();
            let valid_stopped = fs::read(&stopped).unwrap();
            for (path, value) in [
                (&registration, json!({"session_id":Uuid::new_v4()})),
                (&registration, json!({})),
                (
                    &stopped,
                    json!({"session_id":Uuid::new_v4(),"suspended":true,"cleanup_observed":true}),
                ),
                (
                    &stopped,
                    json!({"session_id":f.session,"suspended":false,"cleanup_observed":true}),
                ),
                (
                    &stopped,
                    json!({"session_id":f.session,"suspended":true,"cleanup_observed":false}),
                ),
                (
                    &stopped,
                    json!({"session_id":f.session,"suspended":"true","cleanup_observed":true}),
                ),
                (&stopped, json!({"session_id":f.session,"suspended":true})),
            ] {
                write(path, &value);
                assert!(f.recover().is_err());
                assert_eq!(
                    serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),
                    value
                );
                f.retained(&slots);
                private(&registration, &valid_registration);
                private(&stopped, &valid_stopped);
            }
            private(&stopped, b"invalid-json");
            assert!(f.recover().is_err());
            f.retained(&slots);
        },
    );
}

#[test]
fn actual_guardian_and_startup_locks_fence_recovery_until_exact_owner_releases_them() {
    child(
        "host_browser_capacity::recovery_tests::actual_guardian_and_startup_locks_fence_recovery_until_exact_owner_releases_them",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            for leaf in ["guardian.lock", "startup.lock"] {
                let held = crate::attachment::journal::open_private_file(&f.session_dir.join(leaf))
                    .unwrap();
                held.try_lock().unwrap();
                assert!(f.recover().is_err());
                f.retained(&slots);
                drop(held);
            }
            f.recover().unwrap();
            assert!(!f.worker.join("worker.lock").exists());
        },
    );
}

#[test]
fn malformed_worker_pid_and_current_owned_process_presence_refuse_despite_stopped_records() {
    child(
        "host_browser_capacity::recovery_tests::malformed_worker_pid_and_current_owned_process_presence_refuse_despite_stopped_records",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            let path = f.worker.join("worker.lock");
            for worker in [
                json!({}),
                json!({"pid":0}),
                json!({"pid":1}),
                json!({"pid":-1}),
                json!({"pid":"123"}),
                json!({"pid":i32::MAX as u64+1}),
                json!({"pid":std::process::id()}),
            ] {
                write(&path, &worker);
                assert!(f.recover().is_err());
                assert_eq!(f.slots(), slots);
                assert_eq!(
                    serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
                    worker
                );
                assert!(f.audits().is_empty());
            }
            private(&path, &f.original);
            f.retained(&slots);
        },
    );
}

#[test]
fn unsafe_private_record_links_modes_and_bounds_do_not_authorize_recovery() {
    child(
        "host_browser_capacity::recovery_tests::unsafe_private_record_links_modes_and_bounds_do_not_authorize_recovery",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            let path = f.session_dir.join("stopped.json");
            let original = fs::read(&path).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(f.recover().is_err());
            f.retained(&slots);
            private(&path, &original);
            let link = f.session_dir.join("record-hardlink");
            fs::hard_link(&path, &link).unwrap();
            assert!(f.recover().is_err());
            f.retained(&slots);
            fs::remove_file(link).unwrap();
            let target = f.session_dir.join("retained-record");
            fs::rename(&path, &target).unwrap();
            symlink(&target, &path).unwrap();
            assert!(f.recover().is_err());
            f.retained(&slots);
            assert_eq!(fs::read(&target).unwrap(), original);
            fs::remove_file(&path).unwrap();
            fs::rename(&target, &path).unwrap();
            private(&path, &vec![b'x'; 16385]);
            assert!(f.recover().is_err());
            f.retained(&slots);
        },
    );
}

#[test]
fn unsafe_slot_or_no_matching_owner_cannot_remove_foreign_capacity_or_retire_worker() {
    child(
        "host_browser_capacity::recovery_tests::unsafe_slot_or_no_matching_owner_cannot_remove_foreign_capacity_or_retire_worker",
        || {
            let f = Fixture::new();
            let slot = f.capacity.join("browser-slot-0");
            let original = fs::read(&slot).unwrap();
            fs::set_permissions(&slot, fs::Permissions::from_mode(0o644)).unwrap();
            assert!(f.recover().is_err());
            assert_eq!(fs::read(&slot).unwrap(), original);
            assert!(f.audits().is_empty());
            private(&slot, f.foreign.to_string().as_bytes());
            private(
                &f.capacity.join("browser-slot-1"),
                f.foreign.to_string().as_bytes(),
            );
            let slots = f.slots();
            assert!(f.recover().is_err());
            f.retained(&slots);
            let record = f.capacity.join("foreign-retained");
            fs::rename(&slot, &record).unwrap();
            symlink(&record, &slot).unwrap();
            assert!(f.recover().is_err());
            assert_eq!(fs::read(&record).unwrap(), f.foreign.to_string().as_bytes());
            assert_eq!(fs::read(f.worker.join("worker.lock")).unwrap(), f.original);
            assert!(f.audits().is_empty());
        },
    );
}

#[test]
fn session_directory_symlink_refuses_before_following_private_recovery_state() {
    child(
        "host_browser_capacity::recovery_tests::session_directory_symlink_refuses_before_following_private_recovery_state",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            let backing = f.session_dir.with_file_name("retained-session");
            fs::rename(&f.session_dir, &backing).unwrap();
            symlink(&backing, &f.session_dir).unwrap();
            assert!(f.recover().is_err());
            assert_eq!(f.slots(), slots);
            assert_eq!(
                fs::read(backing.join("journal/host-browser/worker.lock")).unwrap(),
                f.original
            );
            assert_eq!(
                fs::read_dir(&backing)
                    .unwrap()
                    .filter(|e| e
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("browser-capacity-recovery-"))
                    .count(),
                0
            );
        },
    );
}

#[test]
fn automatic_reclaim_requires_every_complete_guardian_boolean_and_keeps_unverified_slots() {
    child(
        "host_browser_capacity::recovery_tests::automatic_reclaim_requires_every_complete_guardian_boolean_and_keeps_unverified_slots",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            let marker = f.worker.join("guardian-cleanup.json");
            for field in [
                "observed",
                "cleanup_complete",
                "descendants_terminated",
                "descendants_reaped",
                "temporary_cleaned",
            ] {
                for bad in [Value::Null, json!(false), json!("true")] {
                    let mut value = Fixture::marker();
                    value[field] = bad;
                    write(&marker, &value);
                    assert!(f.automatic().is_err());
                    f.retained(&slots);
                }
            }
            private(&marker, b"malformed-json");
            assert!(f.automatic().is_err());
            f.retained(&slots);
        },
    );
}

#[test]
fn automatic_reclaim_holds_actual_execution_lock_and_only_recycles_one_observed_slot() {
    child(
        "host_browser_capacity::recovery_tests::automatic_reclaim_holds_actual_execution_lock_and_only_recycles_one_observed_slot",
        || {
            let f = Fixture::new();
            let slots = f.slots();
            let execution = crate::attachment::journal::open_private_file(
                &f.session_dir
                    .join("journal")
                    .join(format!("{}.execution.lock", f.session)),
            )
            .unwrap();
            execution.try_lock().unwrap();
            assert!(f.automatic().is_err());
            f.retained(&slots);
            drop(execution);
            let next = f.automatic().unwrap();
            assert_ne!(next.owner, f.session);
            assert_eq!(
                fs::read(f.capacity.join("browser-slot-0")).unwrap(),
                next.owner.to_string().as_bytes()
            );
            assert_eq!(
                fs::read(f.capacity.join("browser-slot-1")).unwrap(),
                slots[1]
            );
            assert_eq!(
                fs::read(f.capacity.join("browser-slot-2")).unwrap(),
                slots[2]
            );
            assert!(!f.worker.join("worker.lock").exists());
            assert_eq!(f.audits().len(), 1);
            let record: Value = serde_json::from_slice(&fs::read(&f.audits()[0]).unwrap()).unwrap();
            assert_eq!(record["external_effects_reconciled"], false);
            next.release().unwrap();
            assert!(!f.capacity.join("browser-slot-0").exists());
        },
    );
}
