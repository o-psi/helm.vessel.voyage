use super::*;
#[test]
fn default_activation_is_exact_and_profile_changes_invalidate_dispatch() {
    let root = tempfile::tempdir().unwrap();
    let anchor = DefaultsSource {
        directory: root.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let profiles = root.path().join("profiles");
    let profile_store = ProfileStore::open(&profiles).unwrap();
    let snapshot = profile_store.inspect("autonomous").unwrap().unwrap();
    let profile = ProfileRef {
        directory: profiles.clone(),
        name: snapshot.name.clone(),
        revision: snapshot.revision,
        digest: snapshot.digest().unwrap(),
        snapshot,
    };
    let config = Config {
        access: Some(crate::config::AccessMode::ReadOnly),
        policy_defaults: Some(anchor.clone()),
        ..Default::default()
    };
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            key: DefaultKey::Preference {
                scope: DefaultScope::Global {},
            },
            expected_revision: 0,
            value: DefaultValue::Preference {
                profile: Some(profile),
            },
        })
        .unwrap();
    let before = preview(&config, root.path()).unwrap();
    assert!(before.requires_confirmation);
    assert!(!before.activation_current);
    assert!(resolve_using(&config, root.path(), Source::System).is_err());
    assert!(activate(&config, root.path(), Uuid::new_v4(), 0, "wrong").is_err());
    let id = Uuid::new_v4();
    let receipt = activate(&config, root.path(), id, 0, &before.transition_digest).unwrap();
    assert!(!receipt.duplicate);
    assert!(
        activate(&config, root.path(), id, 0, &before.transition_digest)
            .unwrap()
            .duplicate
    );
    assert!(activate(&config, root.path(), id, 1, &before.transition_digest).is_err());
    let (_, guard) = resolve_using(&config, root.path(), Source::System).unwrap();
    guard.check_current().unwrap();
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            key: DefaultKey::Preference {
                scope: DefaultScope::Global {},
            },
            expected_revision: 1,
            value: DefaultValue::Preference { profile: None },
        })
        .unwrap();
    assert!(guard.check_current().is_err());
    let after = preview(&config, root.path()).unwrap();
    assert!(!after.activation_current);
    assert_ne!(before.candidate_digest, after.candidate_digest);
}
#[test]
fn workspace_default_overrides_global_without_using_stale_named_revision() {
    let root = tempfile::tempdir().unwrap();
    let anchor = DefaultsSource {
        directory: root.path().join("defaults"),
        store_id: Uuid::new_v4(),
    };
    let store = DefaultsStore::create(&anchor).unwrap();
    let profiles = root.path().join("profiles");
    let source = ProfileStore::open(&profiles).unwrap();
    let change = crate::policy_profile::store::ProfileChange {
        operation_id: Uuid::new_v4(),
        name: "custom".into(),
        expected_revision: 0,
        action: crate::policy_profile::store::Action::Create {
            rules: crate::policy_profile::Builtin::Restricted.document().rules,
        },
    };
    let snapshot = source.change(&change).unwrap().snapshot;
    let profile = ProfileRef {
        directory: profiles,
        name: snapshot.name.clone(),
        revision: 1,
        digest: snapshot.digest().unwrap(),
        snapshot,
    };
    let key = DefaultKey::Preference {
        scope: DefaultScope::Workspace {
            workspace: root.path().to_owned(),
        },
    };
    store
        .change(&DefaultsChange {
            operation_id: Uuid::new_v4(),
            key: key.clone(),
            expected_revision: 0,
            value: DefaultValue::Preference {
                profile: Some(profile),
            },
        })
        .unwrap();
    let config = Config {
        policy_defaults: Some(anchor),
        ..Default::default()
    };
    let preview = self::preview(&config, root.path()).unwrap();
    assert_eq!(preview.selected_profile.unwrap().name, "custom");
    source
        .change(&crate::policy_profile::store::ProfileChange {
            operation_id: Uuid::new_v4(),
            expected_revision: 1,
            action: crate::policy_profile::store::Action::Delete {},
            ..change
        })
        .unwrap();
    assert!(self::preview(&config, root.path()).is_err());
}
