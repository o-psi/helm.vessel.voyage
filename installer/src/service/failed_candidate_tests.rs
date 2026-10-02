//! Private transaction-context regressions. The existing thread-local manager and
//! PID files are scripted; these do not establish native systemd/process behavior.
use super::*;
use crate::fixture_tests::Fixture;
use std::os::unix::fs::{PermissionsExt, symlink};
const PID: u32 = 700;
const OLD_PID: u32 = 701;

struct Owned {
    f: Fixture,
    old: PathBuf,
    bin: PathBuf,
    prior: Activation,
    pin: ContextPin,
    manager: String,
    effective: String,
    override_path: Option<PathBuf>,
}
impl Owned {
    fn new(credential: bool) -> Self {
        let f = Fixture::new();
        let old = crate::fixture_tests::release(&f, "prior", "1.0.2");
        let bin = crate::fixture_tests::release(&f, "candidate", "1.0.3");
        for dir in ["units", "state", "data/helm"] {
            crate::install::files::private_directory(&f.root.join(dir)).unwrap();
        }
        let state = f.root.join("state");
        let definition = unit::render(&old, &state).unwrap();
        let prior = Activation {
            active: true,
            enabled: true,
            unit_file_state: "enabled".into(),
            definition: Some(definition),
            state,
        };
        fs::write(
            f.root.join("units").join(NAME),
            unit::render(&bin, &prior.state).unwrap(),
        )
        .unwrap();
        let manager = format!(
            "HOME={}\nXDG_DATA_HOME={}\n",
            f.root.display(),
            f.root.join("data").display()
        );
        let (override_path, effective) = if credential {
            let path = f
                .root
                .join("units/voyage-vessel.service.d/credentials.conf");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let key = f.root.join("private-key-location");
            fs::write(
                &path,
                format!(
                    "[Service]\nEnvironment=\"VOYAGE_CREDENTIAL_KEY_FILE={}\"\n",
                    key.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            (
                Some(path),
                format!("VOYAGE_CREDENTIAL_KEY_FILE={}", key.display()),
            )
        } else {
            (None, String::new())
        };
        let placeholder = ContextPin {
            accounts: f.root.join("data/helm"),
            candidate_manifest_sha256: String::new(),
            namespace: BTreeMap::new(),
            manager_sha256: String::new(),
            unit_environment_sha256: String::new(),
            overrides_sha256: String::new(),
        };
        let mut owned = Self {
            f,
            old,
            bin,
            prior,
            pin: placeholder,
            manager,
            effective,
            override_path,
        };
        owned.queue_environment(&owned.manager, &owned.effective, true);
        let manifest = crate::install::release::Manifest::inspect(&owned.bin).unwrap();
        owned.pin = environment(
            &layout(&owned.prior).unwrap(),
            &owned.f.root.join("data/helm"),
            &digest(&serde_json::to_vec(&manifest).unwrap()),
        )
        .unwrap();
        owned.f.done();
        owned.process(PID, &owned.bin, false, 123, 10);
        owned.process(OLD_PID, &owned.old, true, 456, 11);
        owned
    }
    fn queue_effective(&self) {
        self.f.call(
            &["show", "--value", "--property", "UnitPath"],
            self.f.root.join("units").to_str().unwrap(),
        );
        self.f.query(
            "DropInPaths",
            self.override_path
                .as_ref()
                .map(|p| p.to_str().unwrap())
                .unwrap_or(""),
        );
        self.f.query(
            "FragmentPath",
            self.f.root.join("units").join(NAME).to_str().unwrap(),
        );
        self.f.query("KillMode", "process");
        self.f.query("ExecStop", "");
        self.f.query("ExecStopPost", "");
    }
    fn queue_environment(&self, manager: &str, effective: &str, account_matches: bool) {
        self.queue_effective();
        self.f.call(&["show-environment"], manager);
        self.f.query("Environment", effective);
        if account_matches {
            self.f.query(
                "DropInPaths",
                self.override_path
                    .as_ref()
                    .map(|p| p.to_str().unwrap())
                    .unwrap_or(""),
            );
        }
    }
    fn check(&self) {
        self.f.query("UnitFileState", "enabled");
        self.queue_environment(&self.manager, &self.effective, true);
    }
    fn observation(&self, active: &str, pid: u32, job: &str) {
        self.f.call(
            &["show", NAME, "--property=ActiveState,MainPID,Job"],
            &format!("ActiveState={active}\nMainPID={pid}\nJob={job}\n"),
        );
    }
    fn stat(&self, pid: u32, start: u64, counter: u64) {
        let mut fields = vec!["0".to_owned(); 24];
        fields[0] = "R".into();
        fields[11] = counter.to_string();
        fields[12] = (counter + 2).to_string();
        fields[19] = start.to_string();
        fs::write(
            process_file(pid, "stat"),
            format!("{pid} (owned fixture (nested)) {}\n", fields.join(" ")),
        )
        .unwrap();
    }
    fn process(&self, pid: u32, bin: &Path, old_capacity: bool, start: u64, counter: u64) {
        let path = self.f.root.join(format!("pid-{pid}"));
        symlink(bin.join("vessel"), path).unwrap();
        self.stat(pid, start, counter);
        let mut args = vec![
            bin.join("vessel").into_os_string().into_encoded_bytes(),
            b"local-serve".to_vec(),
            b"--directory".to_vec(),
            self.prior.state.as_os_str().as_encoded_bytes().to_vec(),
            b"--voyage-binary".to_vec(),
            bin.join("voyage").into_os_string().into_encoded_bytes(),
        ];
        if old_capacity {
            args.extend([b"--capacity".to_vec(), b"16".to_vec()]);
        }
        let mut command = args.join(&0);
        command.push(0);
        fs::write(process_file(pid, "cmdline"), command).unwrap();
        let mut environment = self
            .pin
            .namespace
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\0")
            .into_bytes();
        environment.push(0);
        fs::write(process_file(pid, "environ"), environment).unwrap();
    }
    fn unchanged_files(&self) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(self.f.root.join("units").join(NAME)).unwrap(),
            fs::read(self.bin.parent().unwrap().join("release.json")).unwrap(),
        )
    }
}

#[test]
fn only_exact_published_unit_can_cancel_pid_zero_restart_and_transition_jobs() {
    for (active, initial_job, terminal, final_job) in [
        ("activating", "77/start", "inactive", ""),
        ("deactivating", "78/stop", "failed", "0"),
        ("inactive", "79/start", "inactive", ""),
        ("failed", "80/start", "failed", "0"),
    ] {
        let owned = Owned::new(false);
        let before = owned.unchanged_files();
        owned.check();
        owned.observation(active, 0, initial_job);
        owned.f.call(&["--no-block", "stop", NAME], "");
        owned.check();
        owned.observation("deactivating", 0, "81/stop");
        owned.check();
        owned.observation(terminal, 0, final_job);
        quiesce(&owned.bin, &owned.prior, &owned.pin).unwrap();
        owned.f.done(); // Exactly one stop; no restart, enable or process signal.
        assert_eq!(owned.unchanged_files(), before);
    }
}

#[test]
fn owned_active_candidate_stops_once_and_requires_inactive_pid_zero_empty_job() {
    for (active, credential) in [
        ("active", false),
        ("active", true),
        ("activating", false),
        ("activating", true),
        ("deactivating", false),
        ("deactivating", true),
    ] {
        let owned = Owned::new(credential);
        let before = owned.unchanged_files();
        owned.check();
        owned.observation(active, PID, "");
        owned.observation(active, PID, "");
        owned.f.call(&["--no-block", "stop", NAME], "");
        owned.check();
        // PID0 alone is not cleanup while a job is still queued.
        owned.observation("inactive", 0, "82/stop");
        owned.check();
        owned.observation("inactive", 0, "");
        quiesce(&owned.bin, &owned.prior, &owned.pin).unwrap();
        owned.f.done();
        assert_eq!(owned.unchanged_files(), before);
    }
}

#[test]
fn changing_stat_counters_preserve_the_exact_start_witness_but_start_change_does_not() {
    let owned = Owned::new(false);
    let before = fs::read(process_file(PID, "stat")).unwrap();
    let first = process(PID, &owned.bin, &owned.prior.state, &owned.pin, false).unwrap();
    owned.stat(PID, 123, 90000);
    let after = fs::read(process_file(PID, "stat")).unwrap();
    assert_ne!(before, after, "mutable stat counters actually changed");
    let second = process(PID, &owned.bin, &owned.prior.state, &owned.pin, false).unwrap();
    assert_eq!(first, second);
    owned.stat(PID, 124, 90001);
    let replacement = process(PID, &owned.bin, &owned.prior.state, &owned.pin, false).unwrap();
    assert_ne!(
        replacement, first,
        "same PID with changed start is another owner"
    );
    owned.stat(PID, 0, 90002);
    assert!(process(PID, &owned.bin, &owned.prior.state, &owned.pin, false).is_err());
    owned.f.done();
}

#[test]
fn capture_pins_the_original_capacity_namespace_and_qualified_candidate_separately() {
    let owned = Owned::new(true);
    fs::write(
        owned.f.root.join("units").join(NAME),
        owned.prior.definition.as_ref().unwrap(),
    )
    .unwrap();
    owned.queue_effective();
    owned.f.query("ActiveState", "active");
    owned.f.query("UnitFileState", "enabled");
    owned.f.query("MainPID", &OLD_PID.to_string());
    owned.f.query("InvocationID", "original-private-invocation");
    owned.queue_environment(&owned.manager, &owned.effective, true);
    owned.f.query("MainPID", &OLD_PID.to_string());
    let candidate = crate::install::release::Manifest::inspect(&owned.bin).unwrap();
    let captured = capture(&owned.old, &owned.prior, &owned.pin.accounts, &candidate).unwrap();
    owned.f.done();
    assert_eq!(captured, owned.pin);
    assert_eq!(captured.accounts, owned.f.root.join("data/helm"));
    // Capacity16 compatibility belongs to capture, not the failed current image.
    assert!(process(OLD_PID, &owned.old, &owned.prior.state, &captured, false).is_err());
}

#[test]
fn generic_service_review_still_refuses_transitions_without_sending_stop() {
    for transition in ["activating", "deactivating"] {
        let owned = Owned::new(false);
        owned.queue_effective();
        owned.f.query("ActiveState", transition);
        assert!(super::super::systemd::review_activation(&owned.bin).is_err());
        owned.f.done();
    }
}

#[test]
fn malformed_unknown_or_active_pid_zero_never_admits_a_stop() {
    for reply in [
        "ActiveState=unknown\nMainPID=0\nJob=\n",
        "ActiveState=active\nMainPID=0\nJob=\n",
        "ActiveState=activating\nMainPID=1\nJob=\n",
        "ActiveState=activating\nMainPID=not-a-pid\nJob=\n",
        "ActiveState=activating\nMainPID=0\n",
        "ActiveState=activating\nMainPID=0\nJob=\nJob=duplicate\n",
        "ActiveState=activating\nMainPID=0\nJob=\nForeign=field\n",
    ] {
        let owned = Owned::new(false);
        owned.check();
        owned
            .f
            .call(&["show", NAME, "--property=ActiveState,MainPID,Job"], reply);
        assert!(quiesce(&owned.bin, &owned.prior, &owned.pin).is_err());
        owned.f.done();
    }
}

#[test]
fn independently_changed_unit_enablement_manifest_or_binary_refuses_before_stop() {
    for variant in 0..5 {
        let owned = Owned::new(false);
        match variant {
            0 => {
                let path = owned.f.root.join("units").join(NAME);
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(b"# Independent operator edit\n");
                fs::write(path, bytes).unwrap();
            }
            1 => owned.f.query("UnitFileState", "disabled"),
            2 => {
                owned.f.query("UnitFileState", "enabled");
                let path = owned.bin.parent().unwrap().join("release.json");
                let mut manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                manifest["target"] = "different-reviewed-target".into();
                fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
            3 => {
                owned.f.query("UnitFileState", "enabled");
                fs::write(owned.bin.join("vessel"), b"different image bytes").unwrap();
            }
            _ => {
                owned.f.query("UnitFileState", "enabled");
                let path = owned.bin.parent().unwrap().join("release.json");
                let mut manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                manifest["update_compatibility"] = serde_json::json!({"schema_version":1,"formats":{},"implementation_sha256":"a".repeat(64),"build_inputs_sha256":"b".repeat(64)});
                fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
            }
        }
        let independent = owned.unchanged_files();
        assert!(quiesce(&owned.bin, &owned.prior, &owned.pin).is_err());
        owned.f.done();
        assert_eq!(owned.unchanged_files(), independent);
    }
}

#[test]
fn manager_or_effective_namespace_and_credential_override_drift_refuses_before_stop() {
    for variant in 0..5 {
        let owned = Owned::new(variant >= 3);
        owned.f.query("UnitFileState", "enabled");
        match variant {
            0 => owned.queue_environment(
                &format!("{}UNRELATED_CONTEXT=changed\n", owned.manager),
                &owned.effective,
                true,
            ),
            1 => owned.queue_environment(&owned.manager, "UNRELATED_CONTEXT=changed", true),
            2 => owned.queue_environment(
                &format!(
                    "HOME={}\nXDG_DATA_HOME={}\n",
                    owned.f.root.display(),
                    owned.f.root.join("foreign-data").display()
                ),
                &owned.effective,
                false,
            ),
            3 => {
                let path = owned.override_path.as_ref().unwrap();
                let other = owned.f.root.join("another-key-location");
                fs::write(
                    path,
                    format!(
                        "[Service]\nEnvironment=\"VOYAGE_CREDENTIAL_KEY_FILE={}\"\n",
                        other.display()
                    ),
                )
                .unwrap();
                owned.queue_environment(
                    &owned.manager,
                    &format!("VOYAGE_CREDENTIAL_KEY_FILE={}", other.display()),
                    true,
                );
            }
            _ => {
                fs::write(
                    owned.override_path.as_ref().unwrap(),
                    b"[Service]\nExecStart=/unreviewed\n",
                )
                .unwrap();
                owned.f.call(
                    &["show", "--value", "--property", "UnitPath"],
                    owned.f.root.join("units").to_str().unwrap(),
                );
                owned.f.query(
                    "DropInPaths",
                    owned.override_path.as_ref().unwrap().to_str().unwrap(),
                );
            }
        }
        assert!(quiesce(&owned.bin, &owned.prior, &owned.pin).is_err());
        owned.f.done();
    }
}

#[test]
fn foreign_pid_image_or_argv_namespace_is_never_stopped() {
    for variant in 0..7 {
        let owned = Owned::new(false);
        match variant {
            0 => {
                let path = owned.f.root.join(format!("pid-{PID}"));
                fs::remove_file(&path).unwrap();
                symlink(owned.old.join("vessel"), path).unwrap();
            }
            1 => fs::write(process_file(PID, "cmdline"), b"foreign\0local-serve\0").unwrap(),
            2 => {
                let mut bytes = fs::read(process_file(PID, "cmdline")).unwrap();
                bytes.extend_from_slice(b"--capacity\0");
                bytes.extend_from_slice(b"16\0");
                fs::write(process_file(PID, "cmdline"), bytes).unwrap();
            }
            3 => fs::write(
                process_file(PID, "environ"),
                format!(
                    "HOME={}\0XDG_DATA_HOME={}\0",
                    owned.f.root.display(),
                    owned.f.root.join("foreign").display()
                ),
            )
            .unwrap(),
            4 => owned.stat(PID, 0, 500),
            5 => {
                let path = process_file(PID, "environ");
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(format!("HOME={}\0", owned.f.root.display()).as_bytes());
                fs::write(path, bytes).unwrap();
            }
            _ => {
                let path = process_file(PID, "environ");
                let mut bytes = fs::read(&path).unwrap();
                bytes.extend_from_slice(
                    format!(
                        "VOYAGE_CREDENTIAL_KEY_FILE={}\0",
                        owned.f.root.join("unreviewed-key-location").display()
                    )
                    .as_bytes(),
                );
                fs::write(path, bytes).unwrap();
            }
        }
        owned.check();
        owned.observation("active", PID, "");
        assert!(quiesce(&owned.bin, &owned.prior, &owned.pin).is_err());
        owned.f.done();
    }
}

#[test]
fn exact_image_requires_same_opened_inode_even_when_copied_bytes_match() {
    let owned = Owned::new(false);
    let path = owned.f.root.join("pid-900");
    fs::copy(owned.bin.join("vessel"), &path).unwrap();
    assert_eq!(
        crate::install::files::hash(&path).unwrap(),
        crate::install::files::hash(&owned.bin.join("vessel")).unwrap()
    );
    assert!(image(900, &owned.bin.join("vessel")).is_err());
    owned.f.done();
}

#[test]
fn oversized_owned_sparse_image_is_refused_before_unbounded_hashing() {
    let owned = Owned::new(false);
    let path = owned.f.root.join("pid-901");
    let file = fs::File::create(&path).unwrap();
    file.set_len(512 * 1024 * 1024 + 1).unwrap();
    // Same named/opened inode isolates the size fence from the identity fence.
    assert!(image(901, &path).is_err());
    assert_eq!(file.metadata().unwrap().len(), 512 * 1024 * 1024 + 1);
    owned.f.done();
}

#[test]
fn stop_admission_failure_is_not_claimed_as_observed_retirement() {
    let owned = Owned::new(false);
    let before = owned.unchanged_files();
    owned.check();
    owned.observation("activating", 0, "91/start");
    owned.f.fail(
        &["--no-block", "stop", NAME],
        "owned stop admission refused",
    );
    assert!(quiesce(&owned.bin, &owned.prior, &owned.pin).is_err());
    owned.f.done();
    assert_eq!(owned.unchanged_files(), before);
}

#[test]
fn operator_change_after_one_stop_retains_that_change_and_refuses_completion() {
    let owned = Owned::new(false);
    owned.check();
    owned.observation("activating", 0, "92/start");
    owned.f.call(&["--no-block", "stop", NAME], "");
    // The second context query independently observes changed enablement. Only
    // one stop is queued; no observed retirement, restart or enabling is inferred.
    owned.f.query("UnitFileState", "disabled");
    assert!(quiesce(&owned.bin, &owned.prior, &owned.pin).is_err());
    owned.f.done();
}
