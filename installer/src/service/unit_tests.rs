use super::*;
use crate::fixture_tests::Fixture;
use std::os::unix::{ffi::OsStringExt, fs::symlink};
#[test]
fn templates_round_trip_escaping_and_legacy_capacity() {
    for bin in [
        "/opt/voyage/bin",
        "/opt/with space/quo\"te/back\\slash/100%",
        "/opt/日本語",
    ] {
        let bin = Path::new(bin);
        let state = Path::new("/private/state%");
        let content = render(bin, state).unwrap();
        assert_eq!(recognized(&content, state).unwrap(), bin);
        assert_eq!(
            recognized(
                &content.replace("\nWorkingDirectory=", " --capacity 16\nWorkingDirectory="),
                state
            )
            .unwrap(),
            bin
        );
        assert!(content.contains("KillMode=process"));
        assert!(recognized(&content, Path::new("/different")).is_err());
        for edit in [
            content.replace("KillMode=process", "KillMode=control-group"),
            content.replace("--voyage-binary", "--other"),
            content.replace("local-serve", "serve"),
            format!("{content}# custom\n"),
        ] {
            assert!(recognized(&edit, state).is_err());
        }
    }
}
#[test]
fn malformed_quoted_paths_and_non_utf8_fail_closed() {
    for value in ["unquoted", "\"unfinished", "\"bad\\q\"", "\"bad\\"] {
        assert!(quoted(value).is_err());
    }
    assert_eq!(
        quoted("\"a%%b\" rest").unwrap(),
        (PathBuf::from("a%b"), " rest")
    );
    for value in ["/line\nbreak", "/tab\tpath", "/nul\0path"] {
        assert!(render(Path::new(value), Path::new("/state")).is_err());
    }
    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'/', 255]));
    assert!(quote(&path).is_err());
}
#[test]
fn existing_rejects_nonfiles_oversize_and_unknown_contents() {
    let f = Fixture::new();
    let l = Layout::discover().unwrap();
    assert!(existing(&l).unwrap().is_none());
    fs::create_dir_all(&l.units).unwrap();
    fs::create_dir(&l.unit).unwrap();
    assert!(existing(&l).is_err());
    fs::remove_dir(&l.unit).unwrap();
    fs::write(&l.unit, vec![b'x'; 16385]).unwrap();
    assert!(existing(&l).is_err());
    fs::write(&l.unit, "operator custom unit").unwrap();
    assert!(existing(&l).is_err());
    fs::remove_file(&l.unit).unwrap();
    let content = render(&f.root.join("bin"), &l.state).unwrap();
    let target = f.root.join("other");
    fs::write(&target, &content).unwrap();
    symlink(&target, &l.unit).unwrap();
    assert!(existing(&l).is_err());
    fs::remove_file(&l.unit).unwrap();
    fs::write(&l.unit, &content).unwrap();
    assert_eq!(existing(&l).unwrap(), Some(content));
}
#[test]
fn effective_manager_configuration_rejects_unsafe_overrides() {
    for failure in ["path", "dropin", "fragment", "kill", "stop", "post"] {
        let f = Fixture::new();
        let l = Layout::discover().unwrap();
        f.call(
            &["show", "--value", "--property", "UnitPath"],
            if failure == "path" {
                "/wrong"
            } else {
                l.units.to_str().unwrap()
            },
        );
        if failure != "path" {
            f.query(
                "DropInPaths",
                if failure == "dropin" { "/override" } else { "" },
            );
        }
        if !matches!(failure, "path" | "dropin") {
            f.query(
                "FragmentPath",
                if failure == "fragment" {
                    "/other"
                } else {
                    l.unit.to_str().unwrap()
                },
            );
        }
        if matches!(failure, "kill" | "stop" | "post") {
            f.query(
                "KillMode",
                if failure == "kill" {
                    "mixed"
                } else {
                    "process"
                },
            );
        }
        if matches!(failure, "stop" | "post") {
            f.query("ExecStop", if failure == "stop" { "danger" } else { "" });
        }
        if failure == "post" {
            f.query("ExecStopPost", "danger");
        }
        assert!(check_effective(&l).is_err());
        f.done();
    }
}
