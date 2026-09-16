use super::*;
use crate::fixture_tests::Fixture;
use std::fs;
#[test]
fn status_does_not_require_owned_unit() {
    let f = Fixture::new();
    f.query("ActiveState", "inactive");
    f.query("MainPID", "0");
    manage("service-status").unwrap();
    f.done();
    assert!(
        manage("service-stop")
            .unwrap_err()
            .to_string()
            .contains("No recognized")
    );
    assert!(crate::service::manage("service-status", &["unexpected".into()]).is_err());
}
#[test]
fn stop_and_uninstall_keep_data_and_binaries() {
    let f = Fixture::new();
    let l = unit::Layout::discover().unwrap();
    fs::create_dir_all(&l.units).unwrap();
    fs::create_dir(&l.state).unwrap();
    fs::write(l.state.join("owner"), "retained").unwrap();
    fs::write(
        &l.unit,
        unit::render(&f.root.join("bin"), &l.state).unwrap(),
    )
    .unwrap();
    f.effective(true);
    f.call(&["--no-block", "stop", unit::NAME], "");
    f.query("ActiveState", "failed");
    manage("service-stop").unwrap();
    f.done();
    f.effective(true);
    f.query("ActiveState", "active");
    assert!(manage("service-uninstall").is_err());
    f.done();
    assert!(l.unit.exists());
    f.effective(true);
    f.query("ActiveState", "inactive");
    f.call(&["disable", unit::NAME], "");
    f.call(&["daemon-reload"], "");
    manage("service-uninstall").unwrap();
    f.done();
    assert!(!l.unit.exists());
    assert_eq!(
        fs::read_to_string(l.state.join("owner")).unwrap(),
        "retained"
    );
}
#[test]
fn unknown_operation_and_manager_failure_are_reported() {
    let f = Fixture::new();
    let l = unit::Layout::discover().unwrap();
    fs::create_dir_all(&l.units).unwrap();
    fs::write(
        &l.unit,
        unit::render(&f.root.join("bin"), &l.state).unwrap(),
    )
    .unwrap();
    f.effective(true);
    assert!(
        manage("unknown")
            .unwrap_err()
            .to_string()
            .contains("Unknown")
    );
    f.done();
    f.effective(true);
    f.fail(&["--no-block", "stop", unit::NAME], "failed to stop");
    assert!(manage("service-stop").is_err());
    assert!(l.unit.exists());
    f.done();
}
