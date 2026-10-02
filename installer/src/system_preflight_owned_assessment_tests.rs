//! Ordinary read-only assessment. Never install/adopt/start or claim root eligibility.
use super::*;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    time::{Duration, Instant},
};
const CHILD: &str = "VOYAGE_OWNED_ASSESSMENT_ROOT";
fn ordinary_name() -> String {
    let uid = unsafe { libc::getuid() };
    assert!(
        uid != 0 && uid == unsafe { libc::geteuid() },
        "assessment cohort requires its ordinary runner"
    );
    let mut entry = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut pointer = std::ptr::null_mut();
    let mut buffer = vec![0u8; 65536];
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut pointer,
        )
    };
    assert_eq!(status, 0);
    assert!(!pointer.is_null() && !entry.pw_name.is_null());
    let name = unsafe { CStr::from_ptr(entry.pw_name) }
        .to_str()
        .unwrap()
        .to_owned();
    assert!(valid_name(&name) && name != "root");
    name
}
type Inventory = BTreeMap<PathBuf, (u32, u32, u32, u64, u64, Vec<u8>)>;
fn inventory(path: &Path) -> Inventory {
    fn walk(root: &Path, path: &Path, out: &mut Inventory) {
        let m = fs::symlink_metadata(path).unwrap();
        let contents = if m.file_type().is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if m.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        out.insert(
            path.strip_prefix(root).unwrap().into(),
            (m.uid(), m.gid(), m.mode(), m.ino(), m.nlink(), contents),
        );
        if m.is_dir() {
            for child in fs::read_dir(path).unwrap() {
                walk(root, &child.unwrap().path(), out);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(path, path, &mut out);
    out
}
fn populated(f: &crate::fixture_tests::Fixture) -> PathBuf {
    let install = f.root.join("owned-inventory");
    fs::create_dir(&install).unwrap();
    for name in [
        "release.json",
        "unit",
        "journal",
        "operation",
        "startup.lock",
    ] {
        let path = install.join(name);
        fs::write(&path, format!("owned fixture {name}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    install
}
fn leases_free(path: &Path) {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path.join("startup.lock"))
        .unwrap();
    file.try_lock().unwrap();
    drop(file);
}
fn check<'a>(assessment: &'a Assessment, name: &str) -> &'a Check {
    assessment.checks.iter().find(|c| c.name == name).unwrap()
}
fn closed(assessment: &Assessment) {
    assert_eq!(assessment.schema_version, 1);
    assert_eq!(assessment.scope, "system");
    assert!(!assessment.ready_to_install);
    assert!(
        assessment
            .blockers
            .iter()
            .any(|b| b.contains("production qualification remain incomplete"))
    );
    assert!(
        assessment
            .blockers
            .iter()
            .any(|b| b.contains("actual identity and service review before mutation"))
    );
    for item in &assessment.checks {
        if !item.passed {
            assert!(
                assessment
                    .blockers
                    .iter()
                    .any(|b| b.starts_with(&format!("{}:", item.name)))
            );
        }
    }
    if assessment.existing_system_installation || assessment.existing_user_installation {
        assert!(
            assessment
                .blockers
                .iter()
                .any(|b| b.contains("before adoption"))
        );
    }
}
#[test]
fn owned_assessment_child() {
    let Some(root) = std::env::var_os(CHILD) else {
        return;
    };
    let root = PathBuf::from(root);
    let name = ordinary_name();
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let before = inventory(&path);
    crate::fixture_tests::set_arguments(&[
        "system-assess",
        "--execution-user",
        "root",
        "--gateway-user",
        &name,
    ]);
    assert!(!crate::run().unwrap());
    assert!(inventory(&path) == before);
    leases_free(&path);
    f.done();
    fs::write(
        root.join("completed"),
        b"ordinary-dispatch-returned-no-wizard",
    )
    .unwrap();
}
#[test]
fn whole_cli_dispatch_emits_closed_json_and_preserves_owned_install_inventory_without_wizard() {
    ordinary_name();
    let root = crate::fixture_tests::Fixture::new();
    let log = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.root.as_path().join("child.log"))
        .unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args([
            "--exact",
            "system_preflight::owned_assessment_tests::owned_assessment_child",
            "--nocapture",
        ])
        .env(CHILD, root.root.as_path())
        .env("HOME", root.root.as_path())
        .env("XDG_DATA_HOME", root.root.as_path().join("data"))
        .env("XDG_CONFIG_HOME", root.root.as_path().join("config"))
        .env("XDG_STATE_HOME", root.root.as_path().join("state"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "owned assessment child failed: {}",
                fs::read_to_string(root.root.as_path().join("child.log")).unwrap()
            );
            break;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("assessment child exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let log = fs::read_to_string(root.root.as_path().join("child.log")).unwrap();
    let begin = log.find('{').unwrap();
    let report = serde_json::Deserializer::from_str(&log[begin..])
        .into_iter::<Value>()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(report["scope"], "system");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["ready_to_install"], false);
    assert_eq!(report["execution_user"], "root");
    assert_eq!(report["gateway_user"], ordinary_name());
    assert!(
        report["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("before mutation"))
    );
    assert_eq!(
        fs::read(root.root.as_path().join("completed")).unwrap(),
        b"ordinary-dispatch-returned-no-wizard"
    );
    root.done();
}
#[test]
fn whole_assessment_retains_root_account_refusal_and_each_unavailable_probe_as_blocker() {
    let name = ordinary_name();
    let a = assess("root", &name).unwrap();
    closed(&a);
    assert!(!check(&a, "execution account").passed);
    assert!(
        check(&a, "execution account")
            .observed
            .contains("account must be ordinary")
    );
    assert!(!check(&a, "effective identity").passed);
    assert_eq!(a.execution_user, "root");
    assert_eq!(a.gateway_user, name);
    let encoded = serde_json::to_value(&a).unwrap();
    assert!(
        encoded["checks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|v| v.get("name").is_some()
                && v.get("observed").is_some()
                && v.get("required").is_some()
                && v["passed"].is_boolean())
    );
}
#[test]
fn whole_assessment_ordinary_execution_metadata_never_becomes_install_or_workspace_authority() {
    let name = ordinary_name();
    let expected = account(&name).unwrap();
    let a = assess(&name, "root").unwrap();
    closed(&a);
    assert!(check(&a, "execution account").passed);
    assert!(
        check(&a, "execution account")
            .observed
            .contains(&format!("uid={}", expected.uid))
    );
    assert!(!check(&a, "gateway account").passed && !check(&a, "effective identity").passed);
    assert_eq!(
        a.existing_user_installation,
        fs::symlink_metadata(expected.home.join(".local/share/voyage/install")).is_ok()
    );
    // Installation presence is metadata only: no account/home contents are read.
}
#[test]
fn malformed_explicit_cli_requests_cannot_fall_through_into_installation_or_infer_login() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let before = inventory(&path);
    for args in [
        vec![],
        vec!["--execution-user".into(), "root".into()],
        vec![
            "--gateway-user".into(),
            "root".into(),
            "--execution-user".into(),
            "other".into(),
        ],
        vec![
            "--execution-user".into(),
            "root".into(),
            "--gateway-user".into(),
            "root".into(),
        ],
        vec![
            "--execution-user".into(),
            "../other".into(),
            "--gateway-user".into(),
            "root".into(),
        ],
    ] {
        assert!(run(&args).is_err());
        assert!(inventory(&path) == before);
        leases_free(&path);
    }
    f.done();
}
#[test]
fn nss_observation_returns_current_ordinary_identity_and_refuses_root_without_changing_accounts() {
    let name = ordinary_name();
    let a = account(&name).unwrap();
    assert_eq!(a.uid, unsafe { libc::getuid() });
    assert_eq!(a.gid, unsafe { libc::getgid() });
    assert!(a.home.is_absolute() && a.groups.contains(&a.gid));
    assert!(a.groups.windows(2).all(|pair| pair[0] < pair[1]) && a.groups.len() <= 256);
    assert!(account("root").is_err());
    assert!(account("synthetic\0not-an-account").is_err());
    let same = account(&name).unwrap();
    assert_eq!(same.uid, a.uid);
    assert_eq!(same.gid, a.gid);
    assert_eq!(same.home, a.home);
    assert_eq!(same.groups, a.groups);
}
#[test]
fn namespace_and_capability_diagnostics_agree_with_current_kernel_but_do_not_grant_root() {
    ordinary_name();
    let status = proc_value("/proc/self/status", 65536).unwrap();
    let raw = status
        .lines()
        .find_map(|s| s.strip_prefix("CapEff:\t"))
        .unwrap();
    assert_eq!(
        capability_bits().unwrap(),
        u64::from_str_radix(raw, 16).unwrap()
    );
    for path in ["/proc/self/uid_map", "/proc/self/gid_map"] {
        let value = proc_value(path, 4096).unwrap();
        let columns: Vec<_> = value.split_ascii_whitespace().collect();
        if full_identity_map(&value) {
            assert_eq!(columns, vec!["0", "0", "4294967295"]);
        }
        assert!(!value.is_empty());
    }
    assert_ne!(unsafe { libc::geteuid() }, 0);
}
#[test]
fn exact_utf8_probe_is_bounded_trimmed_and_never_repairs_or_rewrites_its_source() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let file = path.join("probe");
    fs::write(&file, "  owned café 日本語\n").unwrap();
    let before = inventory(&path);
    assert_eq!(
        proc_value(file.to_str().unwrap(), 64).unwrap(),
        "owned café 日本語"
    );
    assert!(inventory(&path) == before);
    leases_free(&path);
}
#[test]
fn oversized_binary_missing_and_directory_probes_remain_unavailable_without_partial_mutation() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    for (name, bytes) in [("oversize", vec![b'x'; 65]), ("binary", vec![0xff, 0xfe])] {
        fs::write(path.join(name), bytes).unwrap();
    }
    let before = inventory(&path);
    for name in ["oversize", "binary", "absent"] {
        assert!(proc_value(path.join(name).to_str().unwrap(), 64).is_err());
        assert!(inventory(&path) == before);
    }
    assert!(proc_value(path.to_str().unwrap(), 64).is_err());
    leases_free(&path);
}
#[test]
fn absent_descendant_and_mount_inventory_are_observations_without_creating_install_roots() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let missing = path.join("future/install/root");
    let before = inventory(&path);
    assert_eq!(
        checked_path(&missing, unsafe { libc::geteuid() }, true).unwrap(),
        "absent"
    );
    let writable = mount_writable(&missing).unwrap();
    assert_eq!(writable, mount_writable(&path).unwrap());
    assert!(!missing.exists());
    assert!(inventory(&path) == before);
    leases_free(&path);
}
#[test]
fn protected_inventory_types_are_exact_and_refusal_does_not_replace_existing_file_or_directory() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let file = path.join("unit");
    let uid = unsafe { libc::geteuid() };
    let before = inventory(&path);
    assert_eq!(checked_path(&path, uid, true).unwrap(), "present");
    assert_eq!(checked_path(&file, uid, false).unwrap(), "present");
    assert!(checked_path(&path, uid, false).is_err());
    assert!(checked_path(&file, uid, true).is_err());
    assert!(checked_path(Path::new("relative/system"), uid, true).is_err());
    assert!(inventory(&path) == before);
    leases_free(&path);
}
#[test]
fn independent_writable_ancestor_modes_are_preserved_on_refusal_instead_of_chmod_repair() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let child = path.join("protected");
    fs::create_dir(&child).unwrap();
    let file = child.join("unit");
    fs::write(&file, b"owned unit").unwrap();
    for mode in [0o770, 0o707, 0o777] {
        fs::set_permissions(&child, fs::Permissions::from_mode(mode)).unwrap();
        let before = inventory(&path);
        assert!(checked_path(&file, unsafe { libc::geteuid() }, false).is_err());
        assert!(inventory(&path) == before);
    }
    leases_free(&path);
}
#[test]
fn hardlinks_and_symlink_inventory_are_never_followed_or_unlinked_by_assessment() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let file = path.join("unit");
    let hard = path.join("hard");
    fs::hard_link(&file, &hard).unwrap();
    let link = path.join("linked");
    symlink(&file, &link).unwrap();
    let before = inventory(&path);
    assert!(checked_path(&file, unsafe { libc::geteuid() }, false).is_err());
    assert!(checked_path(&link, unsafe { libc::geteuid() }, false).is_err());
    assert!(inventory(&path) == before);
    leases_free(&path);
}
#[test]
fn owned_ordinary_files_do_not_masquerade_as_root_controlled_installation_inventory() {
    ordinary_name();
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let before = inventory(&path);
    assert!(checked_path(&path, 0, true).is_err());
    assert!(checked_path(&path.join("release.json"), 0, false).is_err());
    assert!(inventory(&path) == before);
    leases_free(&path);
}
#[test]
fn home_probe_reports_shape_without_traversing_linked_or_missing_home_or_initializing_state() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let link = path.join("home-link");
    symlink(&path, &link).unwrap();
    let before = inventory(&path);
    assert_eq!(checked_home(&path).unwrap(), "present");
    for candidate in [link, path.join("unit"), path.join("missing-home")] {
        assert!(checked_home(&candidate).is_err());
        assert!(inventory(&path) == before);
    }
    leases_free(&path);
}
#[test]
fn fixed_service_show_observation_is_bounded_or_unavailable_without_mutating_owned_units() {
    ordinary_name();
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let before = inventory(&path);
    // This is an actual fixed read-only systemctl query, NOT the fixture user-manager seam.
    for unit in [
        crate::system_service::ROOT_UNIT,
        crate::system_service::GATEWAY_UNIT,
    ] {
        let result = service_state(unit);
        if let Ok(value) = &result {
            assert!(value.len() <= 8192 && !value.contains('\n'));
        }
        let mut checks = Vec::new();
        let mut blockers = Vec::new();
        record(
            &mut checks,
            &mut blockers,
            "observed service",
            "inspectable read-only unit metadata",
            result,
            |v| v.contains("LoadState=loaded") || v.contains("LoadState=not-found"),
        );
        assert_eq!(checks.len(), 1);
        assert_eq!(blockers.is_empty(), checks[0].passed);
        assert!(inventory(&path) == before);
        leases_free(&path);
    }
    f.done();
}
#[test]
fn unavailable_owned_probe_error_is_retained_in_report_and_never_treated_as_success() {
    let f = crate::fixture_tests::Fixture::new();
    let path = populated(&f);
    let before = inventory(&path);
    let result = proc_value(path.join("missing-probe").to_str().unwrap(), 128);
    let mut checks = Vec::new();
    let mut blockers = Vec::new();
    let called = std::cell::Cell::new(false);
    record(
        &mut checks,
        &mut blockers,
        "private probe",
        "bounded UTF-8 observation",
        result,
        |_| {
            called.set(true);
            true
        },
    );
    assert!(!called.get());
    assert!(!checks[0].passed);
    assert!(checks[0].observed.starts_with("unavailable:"));
    assert_eq!(blockers.len(), 1);
    assert!(blockers[0].contains("required: bounded UTF-8 observation"));
    assert!(inventory(&path) == before);
    leases_free(&path);
}
