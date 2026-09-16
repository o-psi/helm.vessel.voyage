use super::*;
use crate::{
    cli::{Action, Options},
    fixture_tests::{Fixture, release},
};
fn options(bin: &std::path::Path, action: Action) -> Options {
    let mut o = Options::parse(&[action.label().to_lowercase()]).unwrap();
    o.bin_dir = bin.into();
    o.local_source = action != Action::Rollback;
    o
}
#[test]
fn install_upgrade_plan_execute_and_rollback_preserve_history() {
    let f = Fixture::new();
    let one = release(&f, "one", "1.0");
    let two = release(&f, "two", "2.0");
    let first = options(&one, Action::Install);
    let report = plan(&first).unwrap();
    assert!(report.changed);
    assert!(!f.root.join("install").exists());
    let description = describe(&report, &first);
    assert!(description.iter().any(|s| s.contains("Install")));
    let installed = execute(&first, false).unwrap();
    assert_eq!(installed.release, report.release);
    let lock = crate::install::operation_lock().unwrap();
    assert!(crate::install::operation_lock().is_err());
    drop(lock);
    let current = plan(&first).unwrap();
    assert!(!current.changed);
    let upgrade = options(&two, Action::Upgrade);
    let next = execute(&upgrade, false).unwrap();
    assert!(next.changed);
    assert_eq!(
        next.current_release.as_deref(),
        Some(installed.release.as_str())
    );
    let rollback = options(&two, Action::Rollback);
    let planned = plan(&rollback).unwrap();
    assert_eq!(planned.release, installed.release);
    let rolled = execute(&rollback, false).unwrap();
    assert_eq!(rolled.release, installed.release);
    assert_eq!(plan(&rollback).unwrap().release, next.release);
    let status = crate::install::status().unwrap().join("\n");
    assert!(status.contains(&installed.release));
    for id in [&installed.release, &next.release] {
        assert!(f.root.join("install/releases").join(id).is_dir());
    }
}
#[test]
fn missing_action_and_missing_previous_fail_without_publication() {
    let f = Fixture::new();
    let o = Options::parse(&[]).unwrap();
    assert!(execute(&o, true).is_err());
    let rollback = options(&f.root, Action::Rollback);
    assert!(plan(&rollback).is_err());
    assert!(execute(&rollback, false).is_err());
    assert!(!f.root.join("install/current").exists());
}
#[test]
fn dry_run_and_replacement_are_explicit_and_back_up_unmanaged_binaries() {
    let f = Fixture::new();
    let bin = release(&f, "release", "1.0");
    f.script("bin/helm", "printf unmanaged");
    let mut o = options(&bin, Action::Install);
    assert!(plan(&o).is_err());
    o.replace_existing = true;
    execute(&o, true).unwrap();
    assert!(
        std::fs::read_to_string(f.root.join("bin/helm"))
            .unwrap()
            .contains("unmanaged")
    );
    execute(&o, false).unwrap();
    assert!(
        std::fs::symlink_metadata(f.root.join("bin/helm"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(
        std::fs::read_dir(f.root.join("install/backups"))
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("helm"))
    );
}
