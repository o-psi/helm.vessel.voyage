use super::*;
use crate::fixture_tests::{Fixture, release};

#[test]
fn adapted_legacy_identity_requires_all_four_verified_shipped_versions() {
    for case in 0..4 {
        let f = Fixture::new();
        let bin = release(&f, "adapted", if case == 1 { "1.0.3" } else { "1.0.2" });
        fs::set_permissions(bin.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
        let mut manifest = Manifest::inspect(&bin).unwrap();
        manifest.version = "v1.0.2-debian12-isolated-glibc".into();
        if case == 2 {
            let file = f.script("adapted/bin/voyage", "printf 'voyage 1.0.1\n'");
            manifest.binaries.get_mut("voyage").unwrap().sha256 = files::hash(&file).unwrap();
        } else if case == 3 {
            f.script("adapted/bin/vessel", "printf 'vessel 1.0.2\n'; : changed");
        }
        if case == 0 {
            assert!(manifest.is_legacy_v102(bin.parent().unwrap()).unwrap());
            manifest.version = "v1.0.2-other-adaptation".into();
            assert!(manifest.is_legacy_v102(bin.parent().unwrap()).is_err());
        } else {
            assert!(manifest.is_legacy_v102(bin.parent().unwrap()).is_err());
        }
        f.done();
    }
}
