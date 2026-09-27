use super::*;
#[test]
fn approval_classification_is_conservative_for_mutating_shell_shapes() {
    let root = tempfile::tempdir().unwrap();
    let config = Config {
        access: Some(AccessMode::Approval),
        ..Default::default()
    };
    let policy = Policy::new(&config, root.path().into()).unwrap();
    for command in [
        "pwd",
        "ls -la",
        "cat file",
        "env",
        "rg needle file",
        "grep needle file",
        "find . -name fixture",
        "sed -n '1,3p' file",
        "git status",
        "git branch --show-current",
    ] {
        assert!(
            matches!(policy.command(command), Decision::Allow),
            "{command}"
        );
    }
    for command in [
        "env X=1 true",
        "rg --pre cat needle",
        "find . -delete",
        "find . -exec echo x \\;",
        "sed 's/a/b/' file",
        "git branch new",
        "git reset --hard",
        "unknown-program",
        "echo x > file",
    ] {
        assert!(
            !matches!(policy.command(command), Decision::Allow),
            "{command}"
        );
    }
    assert!(matches!(policy.external_tool("fixture"), Decision::Ask(_)));
    assert!(matches!(policy.write(root.path(), false), Decision::Ask(_)));
}
#[test]
fn child_delegation_cannot_broaden_modes_roots_or_environment() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let config = Config {
        access: Some(AccessMode::ReadOnly),
        inherit_env: vec!["PATH".into()],
        github_enabled: false,
        ..Default::default()
    };
    let policy = Policy::new(&config, root.path().into()).unwrap();
    let mut child = Config {
        access: Some(AccessMode::Unrestricted),
        inherit_env: vec!["PATH".into(), "SECRET_NAME".into()],
        github_enabled: true,
        ..Default::default()
    };
    policy.limit_child_config(&mut child, root.path()).unwrap();
    assert_eq!(child.access_mode(), AccessMode::ReadOnly);
    assert_eq!(child.inherit_env, vec!["PATH"]);
    assert!(!child.github_enabled);
    child.allow_read = vec![outside.path().into()];
    assert!(policy.limit_child_config(&mut child, root.path()).is_err());
    assert!(policy.check_delegated_workspace(outside.path()).is_err());
    assert!(matches!(policy.external_tool("fixture"), Decision::Deny(_)));
    assert!(matches!(policy.write(root.path(), true), Decision::Deny(_)));
}
#[cfg(unix)]
#[test]
fn filesystem_resolution_checks_canonical_targets_and_missing_ancestors() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("inside"), b"x").unwrap();
    std::fs::write(outside.path().join("outside"), b"x").unwrap();
    let policy = Policy::new(&Config::default(), root.path().into()).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("redirect")).unwrap();
    assert!(policy.resolve_read(Path::new("redirect/outside")).is_err());
    assert!(
        policy
            .resolve_write(Path::new("redirect/new/child"))
            .is_err()
    );
    assert_eq!(
        policy.resolve_write(Path::new("new/child")).unwrap(),
        root.path().join("new/child")
    );
    assert_eq!(
        policy.resolve_read(Path::new("./inside")).unwrap(),
        root.path().join("inside")
    );
    assert!(policy.resolve_read(Path::new("missing")).is_err());
}
