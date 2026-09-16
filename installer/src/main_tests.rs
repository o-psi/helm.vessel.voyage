use super::*;
use crate::fixture_tests::{Fixture, release, set_arguments};
fn preview(f: &Fixture) {
    f.effective(false);
    f.query("ActiveState", "inactive");
    f.query("UnitFileState", "disabled");
    f.query("InvocationID", "");
}
fn install_args(bin: &std::path::Path, action: &str, dry: bool) {
    let mut args = vec![action, "--bin-dir", bin.to_str().unwrap(), "--no-start"];
    if dry {
        args.push("--dry-run");
    }
    set_arguments(&args);
}
#[test]
fn cli_help_version_status_and_invalid_command() {
    let f = Fixture::new();
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["--version"],
        vec!["status"],
    ] {
        set_arguments(&args);
        assert!(!run().unwrap());
    }
    set_arguments(&["service-status"]);
    f.query("ActiveState", "inactive");
    f.query("MainPID", "0");
    assert!(!run().unwrap());
    f.done();
    set_arguments(&["unknown"]);
    assert!(run().is_err());
    set_arguments(&["service-stop", "extra"]);
    assert!(run().unwrap_err().to_string().contains("no arguments"));
}
#[test]
fn cli_review_dry_run_and_apply_use_same_release() {
    let f = Fixture::new();
    let bin = release(&f, "source", "1.0");
    install_args(&bin, "install-user-service", true);
    preview(&f);
    assert!(!run().unwrap());
    f.done();
    assert!(!f.root.join("install").exists());
    install_args(&bin, "install", false);
    preview(&f);
    preview(&f);
    f.call(&["daemon-reload"], "");
    f.effective(true);
    assert!(!run().unwrap());
    f.done();
    assert!(f.root.join("install/current").exists());
    // A no-op upgrade still reviews and configures the service.
    install_args(&bin, "upgrade", false);
    preview(&f);
    preview(&f);
    f.call(&["daemon-reload"], "");
    f.effective(true);
    assert!(!run().unwrap());
    f.done();
}
#[test]
fn cli_failed_service_upgrade_rolls_back_published_binaries() {
    let f = Fixture::new();
    let one = release(&f, "one", "1.0");
    let two = release(&f, "two", "2.0");
    let first = install::run(install::Options {
        bin_dir: one,
        replace_existing: false,
        dry_run: false,
    })
    .unwrap();
    install_args(&two, "upgrade", false);
    preview(&f);
    f.fail(
        &["show", "--value", "--property", "UnitPath"],
        "injected manager failure",
    );
    assert!(
        run()
            .unwrap_err()
            .to_string()
            .contains("previous binary release restored")
    );
    f.done();
    assert_eq!(
        std::fs::read_link(f.root.join("install/current")).unwrap(),
        first.release_dir
    );
    set_arguments(&["rollback", "--dry-run"]);
    preview(&f);
    assert!(!run().unwrap());
    f.done();
}
#[test]
fn cli_first_install_failure_retains_verified_binary_release() {
    let f = Fixture::new();
    let bin = release(&f, "source", "1.0");
    install_args(&bin, "install", false);
    preview(&f);
    f.fail(
        &["show", "--value", "--property", "UnitPath"],
        "manager unavailable",
    );
    assert!(
        run()
            .unwrap_err()
            .to_string()
            .contains("binaries installed; service configuration failed")
    );
    f.done();
    assert!(f.root.join("install/current/bin/helm").exists());
}
