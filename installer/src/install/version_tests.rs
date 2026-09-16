use super::*;
use crate::fixture_tests::Fixture;
#[test]
fn consistent_version_identity_is_required_for_every_local_binary() {
    let f = Fixture::new();
    for name in super::super::release::BINARIES {
        f.script(&format!("bin/{name}"), &format!("printf '{name} 1.2.3\\n'"));
    }
    assert_eq!(inspect(&f.root.join("bin")).unwrap(), "1.2.3");
    for body in [
        "printf 'wrong 1.2.3'",
        "printf helm",
        "printf 'helm 2.0'",
        "printf 'helm 1.2.3 trailing'",
        "printf '\\377'",
        "exit 1",
    ] {
        f.script("bin/helm", body);
        assert!(inspect(&f.root.join("bin")).is_err(), "{body}");
    }
    assert!(inspect(&f.root.join("missing")).is_err());
}
#[test]
fn version_output_is_bounded_and_long_versions_rejected() {
    let f = Fixture::new();
    f.script("bin/helm", "head -c 4097 /dev/zero");
    assert!(
        inspect(&f.root.join("bin"))
            .unwrap_err()
            .to_string()
            .contains("Oversized")
    );
    f.script("bin/helm", &format!("printf 'helm {}'", "x".repeat(129)));
    assert!(inspect(&f.root.join("bin")).is_err());
}
