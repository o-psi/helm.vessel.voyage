use super::*;
use crate::policy_profile::{
    Builtin,
    store::{Action, ProfileChange},
};
use uuid::Uuid;
#[test]
fn preview_requires_exact_confirmation_and_rechecks_changed_profiles() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("profiles");
    let store = ProfileStore::open(&directory).unwrap();
    let config = Config {
        access: Some(crate::config::AccessMode::ReadOnly),
        ..Default::default()
    };
    let runtime = RuntimePolicy::resolve(&config, root.path()).unwrap();
    let context = SwitchContext::new(config, runtime.policy().effective().clone());
    let profiles = context.profiles(&directory).unwrap();
    assert_eq!(profiles.len(), 3);
    let profile = store.inspect("autonomous").unwrap().unwrap();
    let target = context.target(directory.clone(), &profile).unwrap();
    let preview = context.preview(&target).unwrap();
    assert!(preview.requires_confirmation);
    let mut request = SwitchRequest {
        target,
        preview_digest: preview.digest.clone(),
        confirmation: None,
    };
    assert!(context.prepare_with_digest(&request).is_err());
    request.confirmation = Some("stale".into());
    assert!(context.prepare_with_digest(&request).is_err());
    request.confirmation = Some(preview.digest.clone());
    let (config, digest) = context.prepare_with_digest(&request).unwrap();
    assert!(config.policy_profile.is_some());
    assert_eq!(digest, preview.proposed.digest());
    let custom = store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "custom".into(),
            expected_revision: 0,
            action: Action::Create {
                rules: Builtin::Restricted.document().rules,
            },
        })
        .unwrap()
        .snapshot;
    let target = context.target(directory, &custom).unwrap();
    let preview = context.preview(&target).unwrap();
    assert!(!preview.requires_confirmation);
    store
        .change(&ProfileChange {
            operation_id: Uuid::new_v4(),
            name: "custom".into(),
            expected_revision: 1,
            action: Action::Replace {
                rules: Builtin::Autonomous.document().rules,
            },
        })
        .unwrap();
    assert!(
        context
            .prepare_with_digest(&SwitchRequest {
                target,
                preview_digest: preview.digest,
                confirmation: None
            })
            .is_err()
    );
}
#[test]
fn contention_retries_only_busy_and_never_invalid_evidence() {
    let mut calls = 0;
    let result = read_sources(|| {
        calls += 1;
        if calls == 1 {
            Err(crate::policy_profile::store::StoreError::Busy.into())
        } else {
            Ok(7)
        }
    })
    .unwrap();
    assert_eq!(result, 7);
    assert_eq!(calls, 2);
    let mut calls = 0;
    assert!(
        read_sources::<()>(|| {
            calls += 1;
            Err(crate::policy_profile::store::StoreError::Evidence.into())
        })
        .is_err()
    );
    assert_eq!(calls, 1);
}
