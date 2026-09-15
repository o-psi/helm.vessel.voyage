use super::*;
#[test]
fn explicit_overrides_only_include_named_invocation_fields() {
    let config = Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        github_enabled: true,
        ..Default::default()
    };
    let none = explicit(&config, &[], false).unwrap();
    assert!(none.access.is_none() && none.read_roots.is_none() && none.github_enabled.is_none());
    let keys = [
        "access=x",
        "unattended_approval=x",
        "allow_read=x",
        "allow_write=x",
        "inherit_env=x",
        "github_enabled=true",
    ]
    .map(String::from);
    let all = explicit(&config, &keys, false).unwrap();
    assert_eq!(all.access, Some(crate::config::AccessMode::Unrestricted));
    assert_eq!(all.github_enabled, Some(true));
    assert!(
        all.read_roots.is_some()
            && all.write_roots.is_some()
            && all.inherit_env.is_some()
            && all.unattended.is_some()
    );
    assert!(all.legacy_deny_commands.is_none());
    assert!(explicit(&config, &[], true).unwrap().access.is_some());
    assert_eq!(shell_word("a'b"), "'a'\\''b'");
}
#[cfg(unix)]
#[test]
fn import_input_is_bounded_regular_and_never_follows_symlinks() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("profile.toml");
    let doc = super::super::Builtin::Restricted.document();
    std::fs::write(&path, doc.encode().unwrap()).unwrap();
    assert_eq!(input(&path).unwrap(), doc);
    assert!(input(root.path()).is_err());
    std::os::unix::fs::symlink(&path, root.path().join("link")).unwrap();
    assert!(input(&root.path().join("link")).is_err());
    std::fs::write(&path, vec![b'x'; super::super::MAX_DOCUMENT + 1]).unwrap();
    assert!(input(&path).is_err());
}
