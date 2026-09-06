use super::*;
use crate::config::{
    AccessMode, ApprovalMode, McpServerConfig, ProviderKind, UnattendedApprovalMode,
};
use std::collections::BTreeMap;

fn presentation() -> Presentation {
    Presentation {
        verbose: true,
        log_format: "json".into(),
        plain: true,
        activity: true,
        tool_details: true,
    }
}

fn fixture(root: &Path) -> Config {
    Config {
        chat_preferences: Some(State {
            presentation: presentation(),
            ..State::default()
        }),
        policy_defaults: Some(crate::policy_profile::defaults::DefaultsSource {
            directory: root.join("defaults"),
            store_id: uuid::Uuid::new_v4(),
        }),
        policy_explicit: Overrides {
            access: Some(AccessMode::ReadOnly),
            unattended: Some(UnattendedApprovalMode::Deny),
            read_roots: Some(vec![root.display().to_string()]),
            write_roots: Some(vec![]),
            deny_commands: Some(vec!["fixture-denied".into()]),
            inherit_env: Some(vec!["LANG".into()]),
            github_enabled: Some(false),
        },
        policy_profile: None,
        github_enabled: true,
        provider: ProviderKind::OpenaiChat,
        model: "old-model".into(),
        api_key_env: "FIXTURE_API_KEY".into(),
        api_key_required: false,
        chat_use_max_tokens: true,
        base_url: Some("http://127.0.0.1:12345/v1".into()),
        chatgpt_base_url: Some("https://example.invalid/backend".into()),
        system_prompt: "Fixture system instructions".into(),
        max_tokens: 1024,
        context_window: 8192,
        temperature: Some(0.25),
        provider_retry_attempts: 2,
        provider_retry_initial_ms: 100,
        provider_retry_max_ms: 200,
        command_timeout_secs: 19,
        max_output_bytes: 4096,
        terminal_max_count: 3,
        terminal_max_unread_bytes: 8192,
        subagent_max_concurrency: 2,
        subagent_event_history: 17,
        access: Some(AccessMode::ReadOnly),
        approval: ApprovalMode::Always,
        unattended_approval: UnattendedApprovalMode::Allow,
        workspace: Some(root.into()),
        allow_read: vec![root.join("read")],
        allow_write: vec![root.join("write")],
        deny_commands: vec!["fixture-denied".into()],
        env: BTreeMap::from([("FIXTURE_VALUE".into(), "private fixture".into())]),
        inherit_env: vec!["LANG".into()],
        redact_values: vec!["private fixture".into()],
        mcp_servers: BTreeMap::from([(
            "fixture".into(),
            McpServerConfig {
                command: "fixture-command".into(),
                args: vec!["--fixture".into()],
                env: BTreeMap::from([("FIXTURE_MCP_VALUE".into(), "fixture".into())]),
            },
        )]),
        codex_command: "fixture-compatibility-command".into(),
    }
}

#[test]
fn every_config_setting_and_presentation_survive_with_latest_model() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let mut expected = fixture(directory.path());
    save_to(&path, &expected, "latest-model", None).unwrap();
    expected.model = "latest-model".into();
    let loaded = load_from(&path).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&loaded).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert_eq!(loaded.policy_explicit, expected.policy_explicit);
    let state = loaded.chat_preferences.unwrap();
    assert_eq!(state.explicit, expected.policy_explicit);
    assert_eq!(
        serde_json::to_value(state.presentation).unwrap(),
        serde_json::to_value(presentation()).unwrap()
    );
    assert!(state.profile.is_none());
    assert!(loaded.policy_profile.is_none());
}

#[test]
fn current_presentation_and_removed_profile_replace_previous_choices() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let mut config = fixture(directory.path());
    config.chat_preferences.as_mut().unwrap().profile = Some(Profile {
        request: SelectionRequest {
            directory: directory.path().join("profiles"),
            name: "old".into(),
            revision: 1,
            digest: "0".repeat(64),
            explicit: Overrides::default(),
        },
        confirmation: "old-confirmation".into(),
    });
    let current = Presentation {
        log_format: "text".into(),
        ..Presentation::default()
    };
    save_to(&path, &config, &config.model, Some(current.clone())).unwrap();
    let state = load_from(&path).unwrap().unwrap().chat_preferences.unwrap();
    assert!(state.profile.is_none());
    assert_eq!(
        serde_json::to_value(state.presentation).unwrap(),
        serde_json::to_value(current).unwrap()
    );
}

#[test]
fn invalid_and_oversized_updates_preserve_previous_complete_preferences() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let mut config = fixture(directory.path());
    save_to(&path, &config, "valid-model", None).unwrap();
    let previous = std::fs::read(&path).unwrap();
    assert!(save_to(&path, &config, "", None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    config.system_prompt = "x".repeat(LIMIT as usize + 1);
    assert!(save_to(&path, &config, "another-model", None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), previous);
    assert_eq!(load_from(&path).unwrap().unwrap().model, "valid-model");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn missing_preferences_are_optional_but_invalid_documents_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    assert!(load_from(&path).unwrap().is_none());
    let config = fixture(directory.path());
    save_to(&path, &config, &config.model, None).unwrap();
    let valid =
        serde_json::json!({ "version": 1, "config": Config::default(), "state": State::default() });
    let mut documents = vec![b"{".to_vec(), vec![b' '; LIMIT as usize + 1]];
    for (pointer, value) in [
        ("/version", serde_json::json!(2)),
        (
            "/state/presentation/log_format",
            serde_json::json!("unsupported"),
        ),
        ("/config/model", serde_json::json!("")),
    ] {
        let mut changed = valid.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        documents.push(serde_json::to_vec(&changed).unwrap());
    }
    for document in documents {
        std::fs::write(&path, document).unwrap();
        assert!(load_from(&path).is_err());
    }
}

#[test]
fn in_process_config_edits_retain_preference_recording_and_presentation() {
    let directory = tempfile::tempdir().unwrap();
    let mut config = fixture(directory.path());
    let previous = serde_json::to_value(config.chat_preferences.as_ref().unwrap()).unwrap();
    let explicit = config.policy_explicit.clone();
    config.apply_override("temperature", "0.75").unwrap();
    assert_eq!(config.temperature, Some(0.75));
    assert_eq!(config.policy_explicit, explicit);
    assert_eq!(
        serde_json::to_value(config.chat_preferences.unwrap()).unwrap(),
        previous
    );
}

#[cfg(unix)]
#[test]
fn preference_files_are_private_and_symlinks_are_rejected() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    let config = fixture(directory.path());
    save_to(&path, &config, &config.model, None).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let linked = directory.path().join("linked.json");
    symlink(&path, &linked).unwrap();
    assert!(load_from(&linked).is_err());
    assert!(load_from(directory.path()).is_err());
}

#[cfg(unix)]
#[test]
fn fifo_preferences_are_rejected_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.fifo");
    let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // The owned temporary directory keeps this fixture local to the test.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    assert!(load_from(&path).is_err());
}

#[cfg(target_os = "linux")]
mod profiles {
    use super::*;
    use crate::{
        policy_profile::{
            Builtin,
            store::{Action, ProfileChange, ProfileStore},
        },
        runtime_policy::RuntimePolicy,
    };

    fn selected(root: &Path) -> Config {
        let directory = root.join("profiles");
        let snapshot = ProfileStore::open(&directory)
            .unwrap()
            .change(&ProfileChange {
                operation_id: uuid::Uuid::new_v4(),
                name: "review".into(),
                expected_revision: 0,
                action: Action::Create {
                    rules: Builtin::Restricted.document().rules,
                },
            })
            .unwrap()
            .snapshot;
        let mut config = Config {
            access: Some(AccessMode::Unrestricted),
            chat_preferences: Some(State::default()),
            policy_explicit: Overrides {
                deny_commands: Some(
                    Config::default()
                        .deny_commands
                        .into_iter()
                        .chain(["fixture-denied".into()])
                        .collect(),
                ),
                ..Overrides::default()
            },
            ..Config::default()
        };
        let request = SelectionRequest {
            directory,
            name: snapshot.name.clone(),
            revision: snapshot.revision,
            digest: snapshot.digest().unwrap(),
            explicit: config.policy_explicit.clone(),
        };
        config.policy_profile = Some(Selection::bind(&config, root, request, None).unwrap());
        config
    }

    #[test]
    fn saved_profile_rebinds_fresh_authority_and_explicit_removal_stays_removed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let config = selected(directory.path());
        save_to(&path, &config, &config.model, None).unwrap();
        let mut loaded = load_from(&path).unwrap().unwrap();
        assert!(loaded.policy_profile.is_none());
        assert!(loaded.chat_preferences.as_ref().unwrap().profile.is_some());
        assert_eq!(loaded.policy_explicit, config.policy_explicit);
        // Nonpolicy edits must preserve the recorded source and its confirmation.
        loaded.apply_override("temperature", "0.25").unwrap();
        restore_profile(&mut loaded, directory.path()).unwrap();
        let resolved = RuntimePolicy::resolve(&loaded, directory.path()).unwrap();
        assert_eq!(resolved.config().access_mode(), AccessMode::ReadOnly);
        assert_eq!(
            loaded.policy_profile.as_ref().unwrap().request().explicit,
            config.policy_explicit
        );
        loaded.policy_profile = None;
        save_to(&path, &loaded, &loaded.model, None).unwrap();
        let mut cleared = load_from(&path).unwrap().unwrap();
        assert!(cleared.chat_preferences.as_ref().unwrap().profile.is_none());
        restore_profile(&mut cleared, directory.path()).unwrap();
        assert!(cleared.policy_profile.is_none());
    }

    #[test]
    fn remembered_profile_rejects_another_workspace_or_changed_and_deleted_source() {
        for action in [
            Action::Replace {
                rules: Builtin::Autonomous.document().rules,
            },
            Action::Delete {},
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("preferences.json");
            let config = selected(directory.path());
            save_to(&path, &config, &config.model, None).unwrap();
            let other = directory.path().join("other");
            std::fs::create_dir(&other).unwrap();
            let mut loaded = load_from(&path).unwrap().unwrap();
            assert!(restore_profile(&mut loaded, &other).is_err());
            assert!(loaded.policy_profile.is_none());
            ProfileStore::open_existing(&directory.path().join("profiles"))
                .unwrap()
                .change(&ProfileChange {
                    operation_id: uuid::Uuid::new_v4(),
                    name: "review".into(),
                    expected_revision: 1,
                    action,
                })
                .unwrap();
            let mut loaded = load_from(&path).unwrap().unwrap();
            assert!(restore_profile(&mut loaded, directory.path()).is_err());
            assert!(loaded.policy_profile.is_none());
        }
    }
}
