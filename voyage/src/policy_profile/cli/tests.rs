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

#[test]
fn administrative_commands_preserve_revision_identity_and_only_explicitly_select_profiles() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("profiles");
    let flags = SelectionArgs {
        policy_directory: Some(dir.clone()),
        ..Default::default()
    };
    let config = Config::default();
    let execute = |command| {
        run(
            PolicyArgs { command },
            &flags,
            Some(&config),
            Some(root.path()),
            Default::default(),
            None,
        )
    };
    execute(PolicyCommand::Create {
        name: "custom".into(),
        preset: "restricted".into(),
        expected_revision: 0,
        operation: Some(Uuid::new_v4()),
    })
    .unwrap();
    let store = ProfileStore::open_existing(&dir).unwrap();
    let current = store.inspect("custom").unwrap().unwrap();
    assert_eq!(current.revision, 1);
    execute(PolicyCommand::List {
        after: None,
        limit: 10,
    })
    .unwrap();
    execute(PolicyCommand::Inspect {
        name: "custom".into(),
    })
    .unwrap();
    execute(PolicyCommand::Export {
        name: "custom".into(),
    })
    .unwrap();
    execute(PolicyCommand::Duplicate {
        source: "custom".into(),
        name: "copy".into(),
        expected_revision: 0,
        operation: Some(Uuid::new_v4()),
    })
    .unwrap();
    let input = root.path().join("profile.toml");
    let mut doc = current.document().unwrap();
    doc.rules = Builtin::Balanced.document().rules;
    std::fs::write(&input, doc.encode().unwrap()).unwrap();
    execute(PolicyCommand::Edit {
        name: "custom".into(),
        input: input.clone(),
        expected_revision: 1,
        operation: Some(Uuid::new_v4()),
    })
    .unwrap();
    assert_eq!(store.inspect("custom").unwrap().unwrap().revision, 2);
    execute(PolicyCommand::Import {
        name: "imported".into(),
        input,
        expected_revision: 0,
        operation: Some(Uuid::new_v4()),
    })
    .unwrap();
    execute(PolicyCommand::Delete {
        name: "copy".into(),
        expected_revision: 1,
        operation: Some(Uuid::new_v4()),
    })
    .unwrap();
    assert!(store.inspect("copy").unwrap().unwrap().rules.is_none());
    assert!(
        execute(PolicyCommand::Create {
            name: "bad".into(),
            preset: "unknown".into(),
            expected_revision: 0,
            operation: None
        })
        .is_err()
    );
    assert!(config.policy_profile.is_none());
}
#[cfg(unix)]
#[test]
fn default_parent_initialization_never_follows_a_redirected_component() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("one/two");
    initialize_default_parent(&path).unwrap();
    assert!(path.is_dir());
    std::os::unix::fs::symlink(root.path().join("one"), root.path().join("link")).unwrap();
    assert!(initialize_default_parent(&root.path().join("link/three")).is_err());
    assert!(!root.path().join("one/three").exists());
    assert!(initialize_default_parent(Path::new("relative")).is_err());
}
