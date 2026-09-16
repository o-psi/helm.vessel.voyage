use super::*;
#[test]
fn enable_preserves_source_bytes_and_is_create_only_with_exact_retry() {
    let root = tempfile::tempdir().unwrap();
    let anchor = DefaultsSource {
        directory: root.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    DefaultsStore::create(&anchor).unwrap();
    let source = root.path().join("source.toml");
    let original = b"# retained comment\nmodel = 'fixture'\n";
    std::fs::write(&source, original).unwrap();
    let output = root.path().join("enabled.toml");
    enable(Some(&source), &output, &anchor).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(original));
    assert_eq!(std::fs::read(&source).unwrap(), original);
    let config = load_config(Some(&output)).unwrap();
    assert_eq!(config.policy_defaults, Some(anchor.clone()));
    enable(Some(&source), &output, &anchor).unwrap();
    std::fs::write(&output, b"do not replace").unwrap();
    assert!(enable(Some(&source), &output, &anchor).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"do not replace");
    let other = DefaultsSource {
        directory: root.path().join("other"),
        store_id: Uuid::new_v4(),
    };
    DefaultsStore::create(&other).unwrap();
    let selected = root.path().join("selected.toml");
    std::fs::write(
        &selected,
        toml::to_string(&Config {
            policy_defaults: Some(other),
            ..Default::default()
        })
        .unwrap(),
    )
    .unwrap();
    assert!(enable(Some(&selected), &root.path().join("refused"), &anchor).is_err());
    assert!(!root.path().join("refused").exists());
}
#[cfg(unix)]
#[test]
fn config_inputs_reject_symlinks_oversize_invalid_and_nonregular_files() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("config");
    std::fs::write(&path, b"model='fixture'").unwrap();
    assert!(load_config(Some(&path)).is_ok());
    std::os::unix::fs::symlink(&path, root.path().join("link")).unwrap();
    assert!(read_config(&root.path().join("link")).is_err());
    assert!(read_config(root.path()).is_err());
    std::fs::write(&path, vec![b'x'; MAX_CONFIG + 1]).unwrap();
    assert!(read_config(&path).is_err());
    std::fs::write(&path, b"invalid!!!").unwrap();
    assert!(load_config(Some(&path)).is_err());
}
#[test]
fn cli_preferences_set_inspect_clear_and_activation_bind_exact_source() {
    let root = tempfile::tempdir().unwrap();
    let anchor = DefaultsSource {
        directory: root.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    DefaultsStore::create(&anchor).unwrap();
    let profiles = root.path().join("profiles");
    let store = crate::policy_profile::store::ProfileStore::open(&profiles).unwrap();
    let snapshot = store.inspect("restricted").unwrap().unwrap();
    let config = Config {
        policy_defaults: Some(anchor.clone()),
        ..Default::default()
    };
    let args = |command| DefaultsArgs { command };
    run(
        args(Command::Set {
            scope: ScopeArgs {
                global: true,
                project: false,
            },
            profile_directory: profiles,
            name: snapshot.name.clone(),
            revision: snapshot.revision,
            digest: snapshot.digest().unwrap(),
            expected_revision: 0,
            operation: Some(Uuid::new_v4()),
        }),
        &config,
        root.path(),
        None,
    )
    .unwrap();
    let observed = DefaultsStore::open_existing(&anchor)
        .unwrap()
        .inspect(&DefaultKey::Preference {
            scope: DefaultScope::Global {},
        })
        .unwrap()
        .unwrap();
    assert_eq!(observed.revision, 1);
    run(
        args(Command::Inspect {
            scope: ScopeArgs {
                global: true,
                project: false,
            },
        }),
        &config,
        root.path(),
        None,
    )
    .unwrap();
    run(
        args(Command::List {
            after: None,
            limit: 10,
        }),
        &config,
        root.path(),
        None,
    )
    .unwrap();
    run(
        args(Command::Clear {
            scope: ScopeArgs {
                global: true,
                project: false,
            },
            expected_revision: 1,
            operation: Some(Uuid::new_v4()),
        }),
        &config,
        root.path(),
        None,
    )
    .unwrap();
    let observed = DefaultsStore::open_existing(&anchor)
        .unwrap()
        .inspect(&DefaultKey::Preference {
            scope: DefaultScope::Global {},
        })
        .unwrap()
        .unwrap();
    assert_eq!(observed.value, DefaultValue::Preference { profile: None });
    assert_eq!(observed.revision, 2);
}
