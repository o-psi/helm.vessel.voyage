use super::*;
use crate::fixture_tests::Fixture;
use serde_json::json;
use std::os::unix::{ffi::OsStringExt, fs::symlink};
#[test]
fn catalogue_requires_array_and_passes_no_start_arguments() {
    let f = Fixture::new();
    f.script("bin/helm", "[ \"$1\" = connect ] && [ \"$2\" = --no-start ] && [ \"$3\" = --directory ] && [ \"$5\" = list ] || exit 9\nprintf '[{\"session_id\":\"one\"}]'");
    let entries = catalogue(&f.root.join("bin"), &f.root.join("state")).unwrap();
    assert_eq!(entries, vec![json!({"session_id":"one"})]);
    for body in [
        "printf '{}'",
        "printf 'null'",
        "printf 'bad json'",
        "exit 7",
    ] {
        f.script("bin/helm", body);
        assert!(catalogue(&f.root.join("bin"), &f.root.join("state")).is_err());
    }
    let invalid = std::path::PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 255]));
    assert!(catalogue(&f.root.join("bin"), &invalid).is_err());
}
#[test]
fn readiness_preserves_identity_and_accepts_clean_suspension() {
    for state in ["live", "suspended", "gone", "starting"] {
        let f = Fixture::new();
        let bin = f.root.join("bin");
        f.script("bin/helm", &format!("printf '[{{\"session_id\":\"one\",\"incarnation\":\"original\",\"state\":\"{state}\"}}]'"));
        symlink(bin.join("vessel"), f.root.join("pid-42")).unwrap();
        f.query("ActiveState", "active");
        f.query("MainPID", "42");
        f.query("InvocationID", "new");
        let prior = vec![
            json!({"session_id":"one", "incarnation":"original", "state":"live"}),
            json!({"session_id":"ignored", "state":"suspended"}),
        ];
        assert_eq!(
            wait(&bin, &f.root.join("state"), &prior, Some("old")).is_ok(),
            matches!(state, "live" | "suspended")
        );
        f.done();
    }
}
#[test]
fn readiness_rejects_missing_or_reincarnated_owner() {
    for output in [
        "[]",
        "[{\"session_id\":\"one\",\"incarnation\":\"new\",\"state\":\"live\"}]",
    ] {
        let f = Fixture::new();
        let bin = f.root.join("bin");
        f.script("bin/helm", &format!("printf '%s' '{output}'"));
        symlink(bin.join("vessel"), f.root.join("pid-42")).unwrap();
        f.query("ActiveState", "active");
        f.query("MainPID", "42");
        assert!(
            wait(
                &bin,
                &f.root.join("state"),
                &[json!({"session_id":"one", "incarnation":"old", "state":"live"})],
                None
            )
            .unwrap_err()
            .to_string()
            .contains("original incarnation")
        );
        f.done();
    }
}
#[test]
fn readiness_retries_transition_pid_and_stale_invocation() {
    let f = Fixture::new();
    let bin = f.root.join("bin");
    f.script("bin/helm", "printf '[]'");
    symlink(bin.join("vessel"), f.root.join("pid-42")).unwrap();
    f.query("ActiveState", "activating");
    f.query("ActiveState", "active");
    f.query("MainPID", "bad");
    f.query("ActiveState", "active");
    f.query("MainPID", "42");
    f.query("InvocationID", "same");
    f.query("ActiveState", "active");
    f.query("MainPID", "42");
    f.query("InvocationID", "new");
    wait(&bin, &f.root.join("state"), &[], Some("same")).unwrap();
    f.done();
}
