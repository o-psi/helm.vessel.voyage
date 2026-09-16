use super::*;
use crate::fixture_tests::Fixture;
#[test]
fn command_captures_binary_stdout_and_piped_input() {
    let f = Fixture::new();
    let script = f.script("echo", "cat; printf '\\000\\377'; printf diagnostic >&2");
    assert_eq!(
        run(&script, &[], Some(b"payload\n")).unwrap(),
        b"payload\n\0\xff"
    );
    let script = f.script("args", "printf '%s|%s' \"$1\" \"$2\"");
    assert_eq!(
        run(&script, &["space argument", "percent%"], None).unwrap(),
        b"space argument|percent%"
    );
    assert!(
        run(&f.root.join("missing"), &[], None)
            .unwrap_err()
            .to_string()
            .contains("Cannot execute")
    );
}
#[test]
fn failed_command_sanitizes_and_bounds_diagnostics() {
    let f = Fixture::new();
    let script = f.script("fail", "printf 'bad\\001\\033\\tmessage\\n' >&2; exit 7");
    let error = run(&script, &[], None).unwrap_err().to_string();
    assert!(error.contains("badmessage\n"));
    assert!(!error.contains('\x1b'));
    assert!(error.contains('7'));
    let script = f.script(
        "long",
        "i=0; while [ $i -lt 500 ]; do printf x >&2; i=$((i+1)); done; exit 1",
    );
    let error = run(&script, &[], None).unwrap_err().to_string();
    assert!(error.ends_with(&"x".repeat(400)));
    assert!(!error.ends_with(&"x".repeat(401)));
}
#[test]
fn bounded_stderr_is_rejected_even_on_success() {
    let f = Fixture::new();
    let script = f.script("large", "head -c 65537 /dev/zero >&2");
    assert!(
        run(&script, &[], None)
            .unwrap_err()
            .to_string()
            .contains("exceeded its bound")
    );
}
#[test]
fn query_uses_exact_property_protocol() {
    let f = Fixture::new();
    f.query("MainPID", "42");
    assert_eq!(query("MainPID").unwrap(), "42");
    f.done();
}
