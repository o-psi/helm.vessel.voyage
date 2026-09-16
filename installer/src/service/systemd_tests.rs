use super::*;
use crate::fixture_tests::Fixture;
use std::os::unix::fs::symlink;
fn binaries(f: &Fixture, name: &str) -> std::path::PathBuf {
    for binary in ["helm", "vessel", "voyage"] {
        f.script(
            &format!("{name}/{binary}"),
            if binary == "helm" {
                "printf '[]'"
            } else {
                "exit 0"
            },
        );
    }
    f.root.join(name)
}
fn setup(_f: &Fixture, bin: &Path, active: bool, enabled: bool, start: bool) -> Plan {
    let layout = unit::Layout::discover().unwrap();
    fs::create_dir_all(&layout.units).unwrap();
    let content = unit::render(bin, &layout.state).unwrap();
    fs::write(&layout.unit, &content).unwrap();
    Plan {
        layout,
        content: content.clone(),
        previous: Some(content.clone()),
        active,
        enabled,
        start,
        invocation: "before".into(),
        restart: false,
        rollback: Some(content),
    }
}
fn plan_queries(f: &Fixture, active: &str, enabled: &str) {
    f.effective(false);
    f.query("ActiveState", active);
    f.query("UnitFileState", enabled);
}
fn ready(f: &Fixture, bin: &Path, invocation: Option<&str>) {
    let pid = f.root.join("pid-42");
    let _ = fs::remove_file(&pid);
    symlink(bin.join("vessel"), pid).unwrap();
    f.query("ActiveState", "active");
    f.query("MainPID", "42");
    if let Some(value) = invocation {
        f.query("InvocationID", value);
    }
}
#[test]
fn preview_inactive_and_start_are_read_only() {
    for start in [false, true] {
        let f = Fixture::new();
        let bin = f.root.join("release/bin");
        plan_queries(&f, "inactive", "not-found");
        f.query("InvocationID", "");
        let text = preview(&bin, start).unwrap();
        assert!(text.contains(if start {
            "enable and start/restart"
        } else {
            "leave inactive; manual activation"
        }));
        assert!(text.contains("KillMode=process"));
        assert!(!f.root.join("units").exists());
        f.done();
    }
}
#[test]
fn planning_rejects_transition_unowned_active_and_enablement() {
    for active in [
        "activating",
        "deactivating",
        "reloading",
        "unknown",
        "active",
    ] {
        let f = Fixture::new();
        f.effective(false);
        f.query("ActiveState", active);
        assert!(plan(&f.root.join("bin"), false).is_err());
        f.done();
    }
    for enabled in ["masked", "static", "linked", "enabled-runtime"] {
        let f = Fixture::new();
        plan_queries(&f, "inactive", enabled);
        assert!(plan(&f.root.join("bin"), false).is_err());
        f.done();
    }
}
#[test]
fn active_plan_uses_running_binary_for_rollback_and_restarts_upgrade() {
    let f = Fixture::new();
    let old = binaries(&f, "old");
    let new = binaries(&f, "new");
    let saved = setup(&f, &old, true, true, false);
    plan_queries(&f, "active", "enabled");
    f.query("MainPID", "42");
    f.query("InvocationID", "old-id");
    symlink(old.join("vessel"), f.root.join("pid-42")).unwrap();
    let p = plan(&new, false).unwrap();
    assert!(p.active && p.enabled && p.restart);
    assert_eq!(p.rollback, Some(saved.content));
    assert_eq!(p.invocation, "old-id");
    f.done();
}
#[test]
fn active_plan_rejects_invalid_pid_and_executable() {
    for pid in ["garbage", "0", "1", "42"] {
        let f = Fixture::new();
        let bin = binaries(&f, "release");
        let _p = setup(&f, &bin, true, false, false);
        plan_queries(&f, "active", "disabled");
        f.query("MainPID", pid);
        if pid == "42" {
            symlink(bin.join("helm"), f.root.join("pid-42")).unwrap();
        }
        assert!(plan(&bin, false).is_err());
        f.done();
    }
}
#[test]
fn configure_inactive_install_and_idempotent_reapply() {
    let f = Fixture::new();
    let bin = binaries(&f, "release");
    for _ in 0..2 {
        plan_queries(&f, "inactive", "disabled");
        f.query("InvocationID", "");
        f.call(&["daemon-reload"], "");
        f.effective(true);
        configure(&bin, false, false).unwrap();
        f.done();
        assert_eq!(
            fs::read_to_string(f.root.join("units").join(unit::NAME)).unwrap(),
            unit::render(&bin, &f.root.join("state")).unwrap()
        );
    }
}
#[test]
fn configure_starts_new_service_and_verifies_readiness() {
    let f = Fixture::new();
    let bin = binaries(&f, "release");
    plan_queries(&f, "failed", "disabled");
    f.query("InvocationID", "");
    f.call(&["daemon-reload"], "");
    f.effective(true);
    f.call(&["enable", unit::NAME], "");
    f.call(&["--no-block", "start", unit::NAME], "");
    ready(&f, &bin, None);
    configure(&bin, true, false).unwrap();
    f.done();
}
#[test]
fn configure_active_upgrade_preserves_owners_and_checks_new_invocation() {
    let f = Fixture::new();
    let old = binaries(&f, "old");
    let new = binaries(&f, "new");
    let _saved = setup(&f, &old, true, true, false);
    // Same synthetic PID identifies the new executable; the old unit still forces restart.
    symlink(new.join("vessel"), f.root.join("pid-42")).unwrap();
    plan_queries(&f, "active", "enabled");
    f.query("MainPID", "42");
    f.query("InvocationID", "before");
    f.call(&["daemon-reload"], "");
    f.effective(true);
    f.call(&["--no-block", "restart", unit::NAME], "");
    ready(&f, &new, Some("after"));
    configure(&new, false, false).unwrap();
    f.done();
}
#[test]
fn activation_failure_removes_new_unit_and_retains_state() {
    let f = Fixture::new();
    let bin = binaries(&f, "release");
    plan_queries(&f, "inactive", "disabled");
    f.query("InvocationID", "");
    f.fail(&["daemon-reload"], "injected reload failure");
    f.effective(true);
    f.query("ActiveState", "inactive");
    f.call(&["disable", unit::NAME], "");
    f.call(&["daemon-reload"], "");
    let error = configure(&bin, false, false).unwrap_err().to_string();
    assert!(error.contains("previous unit and activation state restored"));
    assert!(!f.root.join("units").join(unit::NAME).exists());
    assert!(f.root.join("state").is_dir());
    f.done();
}
#[test]
fn failed_compensation_reports_unconfirmed_rollback() {
    let f = Fixture::new();
    let bin = binaries(&f, "release");
    plan_queries(&f, "inactive", "disabled");
    f.query("InvocationID", "");
    f.fail(&["daemon-reload"], "activation failed");
    f.fail(
        &["show", "--value", "--property", "UnitPath"],
        "manager unavailable",
    );
    let error = configure(&bin, false, false).unwrap_err().to_string();
    assert!(error.contains("rollback could not be confirmed"));
    assert!(f.root.join("units").join(unit::NAME).exists());
    f.done();
}
#[test]
fn restore_enabled_active_service_stops_then_restarts_original() {
    let f = Fixture::new();
    let bin = binaries(&f, "old");
    let p = setup(&f, &bin, true, true, false);
    f.effective(true);
    f.query("ActiveState", "active");
    f.call(&["--no-block", "stop", unit::NAME], "");
    f.query("ActiveState", "deactivating");
    f.query("ActiveState", "inactive");
    f.call(&["daemon-reload"], "");
    f.call(&["enable", unit::NAME], "");
    f.call(&["--no-block", "start", unit::NAME], "");
    ready(&f, &bin, None);
    restore(&p, &[]).unwrap();
    assert_eq!(fs::read_to_string(&p.layout.unit).unwrap(), p.content);
    f.done();
}
#[test]
fn rollback_refuses_independent_unit_edit() {
    let f = Fixture::new();
    let bin = binaries(&f, "release");
    let p = setup(&f, &bin, false, false, false);
    fs::write(&p.layout.unit, "operator edit").unwrap();
    assert!(
        restore(&p, &[])
            .unwrap_err()
            .to_string()
            .contains("independently changed")
    );
    assert_eq!(fs::read_to_string(&p.layout.unit).unwrap(), "operator edit");
    f.done();
}
#[test]
fn dry_run_and_external_supervisor_are_never_adopted() {
    let f = Fixture::new();
    let bin = binaries(&f, "release");
    plan_queries(&f, "inactive", "");
    f.query("InvocationID", "");
    configure(&bin, true, true).unwrap();
    assert!(!f.root.join("units").exists());
    f.done();
    fs::create_dir(f.root.join("state")).unwrap();
    fs::write(f.root.join("state/process-http.json"), "fixture").unwrap();
    plan_queries(&f, "inactive", "disabled");
    f.query("InvocationID", "");
    assert!(
        configure(&bin, false, false)
            .unwrap_err()
            .to_string()
            .contains("outside this service")
    );
    assert!(!f.root.join("units").join(unit::NAME).exists());
    f.done();
}
