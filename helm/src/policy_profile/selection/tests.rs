use super::*;
use crate::policy_profile::{
    Builtin,
    store::{Action, ProfileChange, ProfileStore},
};
use crate::{Config, config::AccessMode, runtime_policy::RuntimePolicy};
use tempfile::TempDir;
use uuid::Uuid;
fn setup() -> (TempDir, Config, SelectionRequest) {
    let temp = TempDir::new().unwrap();
    let directory = temp.path().join("profiles");
    let store = ProfileStore::open(&directory).unwrap();
    let snapshot = store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 0,
            action: Action::Create {
                rules: Builtin::Restricted.document().rules,
            },
        })
        .unwrap()
        .snapshot;
    let config = Config {
        access: Some(AccessMode::Unrestricted),
        ..Config::default()
    };
    let request = SelectionRequest {
        directory,
        name: snapshot.name.clone(),
        revision: snapshot.revision,
        digest: snapshot.digest().unwrap(),
        explicit: Overrides::default(),
    };
    (temp, config, request)
}
#[test]
fn explicit_selection_executes_and_is_not_serialized_as_resume_authority() {
    let (temp, mut config, request) = setup();
    config.policy_profile = Some(Selection::bind(&config, temp.path(), request, None).unwrap());
    assert_eq!(
        RuntimePolicy::resolve(&config, temp.path())
            .unwrap()
            .config()
            .access_mode(),
        AccessMode::ReadOnly
    );
    assert!(config.clone().policy_profile.is_some());
    let serialized = toml::to_string(&config).unwrap();
    assert!(!serialized.contains("policy_profile"));
    assert!(!serialized.contains("profiles"));
    assert!(
        toml::from_str::<Config>(&serialized)
            .unwrap()
            .policy_profile
            .is_none()
    );
}
#[test]
fn active_child_retains_root_selection_freshness_without_reapplying_broader_profile() {
    let (temp, mut config, request) = setup();
    let directory = request.directory.clone();
    config.policy_profile = Some(Selection::bind(&config, temp.path(), request, None).unwrap());
    let resolved = RuntimePolicy::resolve(&config, temp.path()).unwrap();
    let child =
        RuntimePolicy::resolve_child(resolved.config(), temp.path(), resolved.policy()).unwrap();
    assert_eq!(child.config().access_mode(), AccessMode::ReadOnly);
    assert!(child.config().policy_profile.is_none());
    ProfileStore::open(&directory)
        .unwrap()
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 1,
            action: Action::Replace {
                rules: Builtin::Autonomous.document().rules,
            },
        })
        .unwrap();
    assert!(resolved.policy().check_current().is_err());
    assert!(child.policy().check_current().is_err());
}
#[test]
fn escalation_requires_exact_fresh_workspace_and_config_transition() {
    let (temp, mut config, mut request) = setup();
    config.access = Some(AccessMode::ReadOnly);
    let preset = ProfileStore::open(&request.directory)
        .unwrap()
        .inspect("autonomous")
        .unwrap()
        .unwrap();
    request.name = preset.name.clone();
    request.revision = preset.revision;
    request.digest = preset.digest().unwrap();
    let preview = Selection::preview(&config, temp.path(), &request).unwrap();
    assert!(preview.requires_confirmation);
    assert!(Selection::bind(&config, temp.path(), request.clone(), None).is_err());
    assert!(Selection::bind(&config, temp.path(), request.clone(), Some("wrong")).is_err());
    let selection = Selection::bind(
        &config,
        temp.path(),
        request.clone(),
        Some(&preview.transition_digest),
    )
    .unwrap();
    config.policy_profile = Some(selection);
    assert!(RuntimePolicy::resolve(&config, temp.path()).is_ok());
    config.deny_commands.push("extra".into());
    assert!(RuntimePolicy::resolve(&config, temp.path()).is_err());
    let other = temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    assert!(Selection::bind(&config, &other, request, Some(&preview.transition_digest)).is_err());
}
#[test]
fn explicit_overrides_win_profile_but_are_part_of_confirmation_and_provenance() {
    let (temp, mut config, mut request) = setup();
    config.access = Some(AccessMode::ReadOnly);
    request.explicit.access = Some(AccessMode::Unrestricted);
    let preview = Selection::preview(&config, temp.path(), &request).unwrap();
    assert!(preview.requires_confirmation);
    assert_eq!(preview.proposed.rules().access, AccessMode::Unrestricted);
    assert!(
        serde_json::to_string(&preview.proposed)
            .unwrap()
            .contains("Explicit:cli")
    );
    let confirmation = preview.transition_digest;
    request.explicit.deny_commands = Some(vec!["git".into()]);
    assert!(Selection::bind(&config, temp.path(), request, Some(&confirmation)).is_err());
}
#[test]
fn deleted_recreated_name_cannot_reuse_old_selection_and_effective_clone_is_not_saved() {
    let (temp, mut config, request) = setup();
    let path = request.directory.clone();
    let selection = Selection::bind(&config, temp.path(), request, None).unwrap();
    config.policy_profile = Some(selection.clone());
    let store = ProfileStore::open(&path).unwrap();
    store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 1,
            action: Action::Delete {},
        })
        .unwrap();
    assert!(selection.check_current().is_err());
    store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 2,
            action: Action::Create {
                rules: Builtin::Restricted.document().rules,
            },
        })
        .unwrap();
    assert!(selection.check_current().is_err());
    assert_eq!(config.access_mode(), AccessMode::Unrestricted);
}
#[test]
fn secret_values_never_enter_preview_or_selection_and_exact_file_cli_roots_work() {
    let (temp, mut config, mut request) = setup();
    let file = temp.path().join("exact");
    std::fs::write(&file, "x").unwrap();
    config
        .env
        .insert("SECRET".into(), "profile-secret-canary".into());
    config.redact_values.push("redaction-canary".into());
    config.allow_read = vec![file.clone()];
    request.explicit.read_roots = Some(vec![file.to_str().unwrap().into()]);
    let preview = Selection::preview(&config, temp.path(), &request).unwrap();
    let text = serde_json::to_string(&preview).unwrap();
    assert!(!text.contains("profile-secret-canary"));
    assert!(!text.contains("redaction-canary"));
    let selection = Selection::bind(
        &config,
        temp.path(),
        request,
        Some(&preview.transition_digest),
    )
    .unwrap();
    assert!(!format!("{selection:?}").contains("profile-secret-canary"));
}
#[test]
fn normal_config_override_handoff_retains_selected_revision_freshness() {
    let (temp, mut config, request) = setup();
    let path = request.directory.clone();
    config.policy_profile = Some(Selection::bind(&config, temp.path(), request, None).unwrap());
    config.apply_override("model", "different-model").unwrap();
    assert!(
        config.policy_profile.is_some(),
        "in-process override must not silently deselect authority"
    );
    assert!(RuntimePolicy::resolve(&config, temp.path()).is_ok());
    ProfileStore::open(&path)
        .unwrap()
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "review".into(),
            expected_revision: 1,
            action: Action::Delete {},
        })
        .unwrap();
    assert!(RuntimePolicy::resolve(&config, temp.path()).is_err());
}
