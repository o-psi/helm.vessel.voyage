//! Private defaults remain preferences, never a remembered authority bypass.
use super::*;
use crate::{
    config::AccessMode,
    policy_profile::{
        Builtin,
        store::{Action, ProfileChange, ProfileStore},
    },
};
use uuid::Uuid;

fn write_saved(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
fn roundtrip_retains_operator_presentation_and_explicit_policy_separately() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("nested/preferences.json");
    assert!(load_from(&path).unwrap().is_none());
    let mut config = Config {
        access: Some(AccessMode::ReadOnly),
        chat_preferences: Some(State::default()),
        policy_explicit: Overrides {
            access: Some(AccessMode::ReadOnly),
            ..Default::default()
        },
        ..Default::default()
    };
    let presentation = Presentation {
        verbose: true,
        log_format: "json".into(),
        plain: true,
        activity: true,
        tool_details: true,
    };
    save_to(&path, &config, "remembered-model", Some(presentation)).unwrap();
    let restored = load_from(&path).unwrap().unwrap();
    assert_eq!(restored.model, "remembered-model");
    assert_eq!(restored.policy_explicit, config.policy_explicit);
    let state = restored.chat_preferences.as_ref().unwrap();
    assert!(state.presentation.verbose && state.presentation.plain);
    assert!(state.presentation.activity && state.presentation.tool_details);
    assert_eq!(state.presentation.log_format, "json");
    assert!(state.profile.is_none());
    assert!(restored.policy_profile.is_none());
    config = restored;
    save_to(&path, &config, "next-model", None).unwrap();
    let next = load_from(&path).unwrap().unwrap();
    assert_eq!(next.model, "next-model");
    assert_eq!(
        next.chat_preferences.unwrap().presentation.log_format,
        "json"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    // An invocation that did not opt into remembered defaults does not write.
    remember(&Config::default(), "unused-model", None).unwrap();
}

#[test]
fn invalid_replacement_preserves_the_previous_exact_file() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("preferences.json");
    save_to(&path, &Config::default(), "valid-model", None).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(save_to(&path, &Config::default(), "   ", None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(save_to(&path, &Config::default(), &"x".repeat(LIMIT as usize), None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn malformed_version_format_configuration_and_bounds_refuse_loading() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("preferences.json");
    let baseline = serde_json::to_value(Saved {
        version: 1,
        config: Config::default(),
        state: State::default(),
    })
    .unwrap();
    for change in 0..5 {
        let mut value = baseline.clone();
        match change {
            0 => value["version"] = serde_json::json!(2),
            1 => value["state"]["presentation"]["log_format"] = serde_json::json!("unsupported"),
            2 => value["config"]["model"] = serde_json::json!(""),
            3 => value["unexpected"] = serde_json::json!(true),
            _ => value["state"]["presentation"] = serde_json::json!(false),
        }
        write_saved(&path, &value);
        assert!(
            load_from(&path).is_err(),
            "malformed preference case {change}"
        );
    }
    std::fs::write(&path, b"{truncated").unwrap();
    assert!(load_from(&path).is_err());
    std::fs::write(&path, vec![b' '; LIMIT as usize + 1]).unwrap();
    assert!(
        load_from(&path)
            .unwrap_err()
            .to_string()
            .contains("too large")
    );
}

#[cfg(unix)]
#[test]
fn public_mode_symlink_directory_and_fifo_never_become_preferences() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("preferences.json");
    save_to(&path, &Config::default(), "valid-model", None).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        load_from(&path)
            .unwrap_err()
            .to_string()
            .contains("private")
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = root.path().join("linked.json");
    symlink(&path, &link).unwrap();
    assert!(load_from(&link).is_err());
    assert!(load_from(root.path()).is_err());
    let fifo = root.path().join("fifo");
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let started = std::time::Instant::now();
    assert!(
        load_from(&fifo)
            .unwrap_err()
            .to_string()
            .contains("regular file")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
}

fn remembered_profile(root: &Path, name: &str) -> (Config, ProfileStore) {
    let directory = root.join("profiles");
    let store = ProfileStore::open(&directory).unwrap();
    let snapshot = store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: name.into(),
            expected_revision: 0,
            action: Action::Create {
                rules: Builtin::Autonomous.document().rules,
            },
        })
        .unwrap()
        .snapshot;
    let mut config = Config {
        access: Some(AccessMode::ReadOnly),
        chat_preferences: Some(State::default()),
        ..Default::default()
    };
    let request = SelectionRequest {
        directory,
        name: snapshot.name.clone(),
        revision: snapshot.revision,
        digest: snapshot.digest().unwrap(),
        explicit: config.policy_explicit.clone(),
    };
    let preview = Selection::preview(&config, root, &request).unwrap();
    assert!(preview.requires_confirmation);
    config.policy_profile =
        Some(Selection::bind(&config, root, request, Some(&preview.transition_digest)).unwrap());
    (config, store)
}

#[test]
fn remembered_profile_rebinds_only_exact_current_workspace_and_revision() {
    let root = tempfile::tempdir().unwrap();
    let (config, store) = remembered_profile(root.path(), "remembered");
    let path = root.path().join("preferences.json");
    save_to(&path, &config, "remembered-model", None).unwrap();
    let mut restored = load_from(&path).unwrap().unwrap();
    assert!(restored.policy_profile.is_none());
    restore_profile(&mut restored, root.path()).unwrap();
    assert_eq!(
        restored.policy_profile.as_ref().unwrap().confirmation(),
        config.policy_profile.as_ref().unwrap().confirmation()
    );
    let original = restored.policy_profile.as_ref().unwrap().request().clone();
    // Existing explicit selection wins over remembered preferences.
    restored
        .chat_preferences
        .as_mut()
        .unwrap()
        .profile
        .as_mut()
        .unwrap()
        .confirmation = "stale".into();
    restore_profile(&mut restored, root.path()).unwrap();
    assert_eq!(
        restored.policy_profile.as_ref().unwrap().request(),
        &original
    );
    let other = root.path().join("other-workspace");
    std::fs::create_dir(&other).unwrap();
    let mut wrong_workspace = load_from(&path).unwrap().unwrap();
    assert!(restore_profile(&mut wrong_workspace, &other).is_err());
    assert!(wrong_workspace.policy_profile.is_none());
    store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "remembered".into(),
            expected_revision: 1,
            action: Action::Replace {
                rules: Builtin::Restricted.document().rules,
            },
        })
        .unwrap();
    let mut stale = load_from(&path).unwrap().unwrap();
    assert!(restore_profile(&mut stale, root.path()).is_err());
    assert!(stale.policy_profile.is_none());
}

#[test]
fn changing_explicit_overrides_does_not_reuse_saved_escalation_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let (config, _) = remembered_profile(root.path(), "remembered");
    let path = root.path().join("preferences.json");
    save_to(&path, &config, "remembered-model", None).unwrap();
    let mut changed = load_from(&path).unwrap().unwrap();
    changed.policy_explicit.github_enabled = Some(true);
    assert!(restore_profile(&mut changed, root.path()).is_err());
    assert!(changed.policy_profile.is_none());
    let mut defaults = Config::default();
    restore_profile(&mut defaults, root.path()).unwrap();
    assert!(defaults.policy_profile.is_none());
}
