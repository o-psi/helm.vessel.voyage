use super::*;
use tempfile::TempDir;
use uuid::Uuid;
fn profile(root: &std::path::Path) -> ProfileRef {
    let directory = root.join("profiles");
    let snapshot = crate::policy_profile::store::ProfileStore::open(&directory)
        .unwrap()
        .inspect("restricted")
        .unwrap()
        .unwrap();
    ProfileRef {
        directory,
        name: snapshot.name.clone(),
        revision: snapshot.revision,
        digest: snapshot.digest().unwrap(),
        snapshot,
    }
}
fn change(root: &std::path::Path) -> DefaultsChange {
    DefaultsChange {
        operation_id: Uuid::new_v4(),
        key: DefaultKey::Preference {
            scope: DefaultScope::Global {},
        },
        expected_revision: 0,
        value: DefaultValue::Preference {
            profile: Some(profile(root)),
        },
    }
}
#[test]
fn private_defaults_cas_clear_restart_and_historical_retry() {
    let temp = TempDir::new().unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    let receipt = store.change(&first).unwrap();
    assert_eq!(receipt.snapshot.revision, 1);
    let clear = DefaultsChange {
        operation_id: Uuid::new_v4(),
        expected_revision: 1,
        value: DefaultValue::Preference { profile: None },
        ..first.clone()
    };
    let cleared = store.change(&clear).unwrap();
    assert_eq!(cleared.snapshot.revision, 2);
    assert!(
        store
            .change(&DefaultsChange {
                operation_id: Uuid::new_v4(),
                ..first.clone()
            })
            .is_err()
    );
    let old = store.change(&first).unwrap();
    assert!(old.duplicate);
    assert_eq!(old.snapshot, receipt.snapshot);
    drop(store);
    let reopened = DefaultsStore::open_existing(&anchor).unwrap();
    assert_eq!(
        reopened.inspect(&first.key).unwrap().unwrap(),
        cleared.snapshot
    );
    assert!(
        DefaultsStore::open_existing(&DefaultsSource {
            store_id: Uuid::new_v4(),
            ..anchor
        })
        .is_err()
    );
}
#[test]
fn missing_anchor_does_not_bootstrap_and_unknown_schema_is_rejected() {
    let temp = TempDir::new().unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("missing"),
        store_id: Uuid::new_v4(),
    };
    assert!(DefaultsStore::open_existing(&anchor).is_err());
    assert!(!anchor.directory.exists());
    assert!(serde_json::from_str::<DefaultsSource>(r#"{"directory":"/tmp","store_id":"00000000-0000-0000-0000-000000000001","authority":true}"#).is_err());
}
#[test]
fn exact_retry_conflict_and_activation_key_value_mismatch_fail() {
    let temp = TempDir::new().unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    assert!(
        store
            .change(&DefaultsChange {
                value: DefaultValue::Preference { profile: None },
                ..first.clone()
            })
            .is_err()
    );
    assert!(
        store
            .change(&DefaultsChange {
                operation_id: Uuid::new_v4(),
                key: DefaultKey::Activation {
                    workspace: temp.path().into()
                },
                ..first
            })
            .is_err()
    );
}
#[test]
fn configuration_serializes_only_defaults_anchor_not_launch_authority() {
    let temp = TempDir::new().unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let mut config = crate::Config {
        policy_defaults: Some(anchor.clone()),
        ..Default::default()
    };
    config
        .env
        .insert("PRIVATE_TEST_KEY".into(), "synthetic-private-value".into());
    let bytes = toml::to_string(&config).unwrap();
    let restored: crate::Config = toml::from_str(&bytes).unwrap();
    assert_eq!(restored.policy_defaults, Some(anchor));
    assert_eq!(restored.env, config.env);
    assert!(restored.policy_profile.is_none());
}
#[test]
fn global_clear_requires_workspace_bound_fallback_activation() {
    let temp = TempDir::new().unwrap();
    let one = temp.path().join("one");
    let two = temp.path().join("two");
    std::fs::create_dir(&one).unwrap();
    std::fs::create_dir(&two).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        policy_defaults: Some(anchor.clone()),
        ..Default::default()
    };
    let restricted = preview(&config, &one).unwrap();
    assert!(!restricted.requires_confirmation);
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            value: DefaultValue::Preference { profile: None },
            ..first
        })
        .unwrap();
    let cleared = preview(&config, &one).unwrap();
    assert!(cleared.requires_confirmation);
    assert!(resolve(&config, &one).is_err());
    activate(&config, &one, Uuid::new_v4(), 0, &cleared.transition_digest).unwrap();
    assert!(resolve(&config, &one).is_ok());
    assert!(resolve(&config, &two).is_err());
    assert!(activate(&config, &two, Uuid::new_v4(), 0, &cleared.transition_digest).is_err());
}
#[test]
fn common_runtime_resolves_actual_workspace_and_children_keep_default_freshness() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("work");
    let child = workspace.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let root = crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace).unwrap();
    assert_eq!(
        root.policy().access_mode(),
        crate::config::AccessMode::ReadOnly
    );
    #[derive(Debug)]
    struct Authority(std::sync::atomic::AtomicBool);
    impl crate::policy::ExecutionAuthority for Authority {
        fn check(&self) -> anyhow::Result<()> {
            anyhow::ensure!(self.0.load(std::sync::atomic::Ordering::SeqCst), "revoked");
            Ok(())
        }
    }
    let authority = std::sync::Arc::new(Authority(std::sync::atomic::AtomicBool::new(true)));
    let parent = root
        .policy()
        .clone()
        .with_execution_authority(authority.clone());
    let nested =
        crate::runtime_policy::RuntimePolicy::resolve_child(root.config(), &child, &parent)
            .unwrap();
    authority
        .0
        .store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(nested.policy().check_current().is_err());
    authority.0.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(nested.policy().check_current().is_ok());
    assert_eq!(
        nested.policy().access_mode(),
        crate::config::AccessMode::ReadOnly
    );
    assert!(nested.config().policy_defaults.is_none());
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            value: DefaultValue::Preference { profile: None },
            ..first
        })
        .unwrap();
    assert!(root.policy().check_current().is_err());
    assert!(nested.policy().check_current().is_err());
}
#[test]
fn explicit_workspace_choice_does_not_track_unrelated_global_edits() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("work");
    std::fs::create_dir(&workspace).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let global = change(temp.path());
    store.change(&global).unwrap();
    let profiles =
        crate::policy_profile::store::ProfileStore::open(&temp.path().join("profiles")).unwrap();
    let reference = |name: &str| {
        let snapshot = profiles.inspect(name).unwrap().unwrap();
        ProfileRef {
            directory: temp.path().join("profiles"),
            name: name.into(),
            revision: snapshot.revision,
            digest: snapshot.digest().unwrap(),
            snapshot,
        }
    };
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            key: DefaultKey::Preference {
                scope: DefaultScope::Workspace {
                    workspace: workspace.clone(),
                },
            },
            expected_revision: 0,
            value: DefaultValue::Preference {
                profile: Some(reference("autonomous")),
            },
        })
        .unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let preview = preview(&config, &workspace).unwrap();
    activate(
        &config,
        &workspace,
        Uuid::new_v4(),
        0,
        &preview.transition_digest,
    )
    .unwrap();
    let runtime = crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace).unwrap();
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            value: DefaultValue::Preference {
                profile: Some(reference("balanced")),
            },
            ..global
        })
        .unwrap();
    runtime.policy().check_current().unwrap();
}
#[test]
fn unactivated_intermediate_preference_never_launders_escalation_after_restart() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    let profiles =
        crate::policy_profile::store::ProfileStore::open(&temp.path().join("profiles")).unwrap();
    let snapshot = profiles.inspect("autonomous").unwrap().unwrap();
    let broad = ProfileRef {
        directory: temp.path().join("profiles"),
        name: snapshot.name.clone(),
        revision: 1,
        digest: snapshot.digest().unwrap(),
        snapshot,
    };
    for expected_revision in [1, 2] {
        store
            .change(&DefaultsChange {
                operation_id: Uuid::new_v4(),
                expected_revision,
                value: DefaultValue::Preference {
                    profile: Some(broad.clone()),
                },
                ..first.clone()
            })
            .unwrap();
    }
    drop(store);
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    assert!(preview(&config, &work).unwrap().requires_confirmation);
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &work).is_err());
}
#[test]
fn activation_exact_retry_is_historical_observation_after_source_changes() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            value: DefaultValue::Preference { profile: None },
            ..first.clone()
        })
        .unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let preview = preview(&config, &work).unwrap();
    let id = Uuid::new_v4();
    let original = activate(&config, &work, id, 0, &preview.transition_digest).unwrap();
    let retry = activate(&config, &work, id, 0, &preview.transition_digest).unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.snapshot, original.snapshot);
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 2,
            ..first
        })
        .unwrap();
    let old = activate(&config, &work, id, 0, &preview.transition_digest).unwrap();
    assert!(old.duplicate);
    assert_eq!(old.snapshot, original.snapshot);
    assert_eq!(
        crate::runtime_policy::RuntimePolicy::resolve(&config, &work)
            .unwrap()
            .policy()
            .access_mode(),
        crate::config::AccessMode::ReadOnly
    );
    assert!(activate(&config, &work, id, 1, &preview.transition_digest).is_err());
}
#[test]
fn clear_fallback_preserves_existing_trusted_config_environment_names() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            value: DefaultValue::Preference { profile: None },
            ..first
        })
        .unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        inherit_env: (0..150).map(|n| format!("existing-name-{n}")).collect(),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let preview = preview(&config, &work).unwrap();
    assert!(preview.requires_confirmation);
    activate(
        &config,
        &work,
        Uuid::new_v4(),
        0,
        &preview.transition_digest,
    )
    .unwrap();
    let runtime = crate::runtime_policy::RuntimePolicy::resolve(&config, &work).unwrap();
    let mut expected = config.inherit_env.clone();
    expected.sort();
    assert_eq!(runtime.config().inherit_env, expected);
}
#[test]
fn replacement_workspace_cannot_reuse_activation_or_active_authority() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let first = change(temp.path());
    store.change(&first).unwrap();
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            value: DefaultValue::Preference { profile: None },
            ..first
        })
        .unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::Unrestricted),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let before = preview(&config, &work).unwrap();
    activate(&config, &work, Uuid::new_v4(), 0, &before.transition_digest).unwrap();
    let runtime = crate::runtime_policy::RuntimePolicy::resolve(&config, &work).unwrap();
    std::fs::rename(&work, temp.path().join("old-work")).unwrap();
    std::fs::create_dir(&work).unwrap();
    assert!(runtime.policy().check_current().is_err());
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &work).is_err());
    assert!(activate(&config, &work, Uuid::new_v4(), 1, &before.transition_digest).is_err());
    let fresh = preview(&config, &work).unwrap();
    activate(&config, &work, Uuid::new_v4(), 1, &fresh.transition_digest).unwrap();
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &work).is_ok());
}
#[test]
fn equal_rules_new_candidate_cannot_inherit_another_candidates_escalation_receipt() {
    let temp = TempDir::new().unwrap();
    let work = temp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let profiles_path = temp.path().join("profiles");
    let profiles = crate::policy_profile::store::ProfileStore::open(&profiles_path).unwrap();
    let snapshot = profiles.inspect("autonomous").unwrap().unwrap();
    let first = DefaultsChange {
        operation_id: Uuid::new_v4(),
        key: DefaultKey::Preference {
            scope: DefaultScope::Global {},
        },
        expected_revision: 0,
        value: DefaultValue::Preference {
            profile: Some(ProfileRef {
                directory: profiles_path,
                name: snapshot.name.clone(),
                revision: snapshot.revision,
                digest: snapshot.digest().unwrap(),
                snapshot,
            }),
        },
    };
    store.change(&first).unwrap();
    let config = crate::Config {
        access: Some(crate::config::AccessMode::ReadOnly),
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let before = preview(&config, &work).unwrap();
    assert!(before.requires_confirmation);
    activate(&config, &work, Uuid::new_v4(), 0, &before.transition_digest).unwrap();
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &work).is_ok());
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            ..first
        })
        .unwrap();
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &work).is_err());
    assert!(activate(&config, &work, Uuid::new_v4(), 1, &before.transition_digest).is_err());
    let fresh = preview(&config, &work).unwrap();
    activate(&config, &work, Uuid::new_v4(), 1, &fresh.transition_digest).unwrap();
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &work).is_ok());
}

#[test]
fn default_profile_edit_invalidates_active_descendants_and_fresh_roots() {
    use crate::policy_profile::store::{Action, ProfileChange, ProfileStore};
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("work");
    let child = workspace.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let mut selected = profile(temp.path());
    let profiles = ProfileStore::open_existing(&selected.directory).unwrap();
    selected.snapshot = profiles
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 0,
            action: Action::Create {
                rules: selected.snapshot.rules.clone().unwrap(),
            },
        })
        .unwrap()
        .snapshot;
    selected.name = selected.snapshot.name.clone();
    selected.digest = selected.snapshot.digest().unwrap();
    let anchor = DefaultsSource {
        directory: temp.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    DefaultsStore::create(&anchor)
        .unwrap()
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 0,
            key: DefaultKey::Preference {
                scope: DefaultScope::Global {},
            },
            value: DefaultValue::Preference {
                profile: Some(selected.clone()),
            },
        })
        .unwrap();
    let config = crate::Config {
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let root = crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace).unwrap();
    let nested =
        crate::runtime_policy::RuntimePolicy::resolve_child(root.config(), &child, root.policy())
            .unwrap();
    profiles
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: selected.name,
            expected_revision: 1,
            action: Action::Replace {
                rules: selected.snapshot.rules.unwrap(),
            },
        })
        .unwrap();
    assert!(root.policy().check_current().is_err());
    assert!(nested.policy().check_current().is_err());
    assert!(crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace).is_err());
}
