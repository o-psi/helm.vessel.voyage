use super::*;
fn root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}
fn rules() -> Rules {
    Builtin::Balanced.document().rules
}
#[test]
fn github_profile_opt_in_is_visible_and_cannot_expand_a_ceiling() {
    let temp = root();
    let base = rules();
    assert!(!base.github_enabled);
    assert!(
        serde_json::to_value(&base)
            .unwrap()
            .get("github_enabled")
            .is_none()
    );
    let enabled = Layer::new(
        LayerKind::Explicit,
        "github-opt-in",
        Overrides {
            github_enabled: Some(true),
            ..Overrides::default()
        },
    )
    .unwrap();
    let before = resolve_test(temp.path(), &base, &[], None).unwrap();
    let after = resolve_test(temp.path(), &base, std::slice::from_ref(&enabled), None).unwrap();
    assert!(after.rules().github_enabled);
    assert!(transition(&before, &after).unwrap().requires_confirmation);
    let ceiling = CeilingDocument {
        schema: 1,
        rules: base.clone(),
    };
    let restricted = resolve_test(temp.path(), &base, &[enabled], Some(&ceiling)).unwrap();
    assert!(!restricted.rules().github_enabled);
}
fn resolve_test(
    workspace: &Path,
    base: &Rules,
    layers: &[Layer],
    ceiling: Option<&CeilingDocument>,
) -> Result<EffectivePolicy> {
    resolve_loaded(workspace, base, layers, ceiling)
}
#[test]
fn presets_are_complete_and_strictly_roundtrip_without_secrets() {
    for preset in [Builtin::Restricted, Builtin::Balanced, Builtin::Autonomous] {
        let doc = preset.document();
        let bytes = doc.encode().unwrap();
        assert_eq!(ProfileDocument::decode(&bytes).unwrap(), doc);
        assert_eq!(doc.rules.read_roots, vec!["$workspace"]);
        assert_eq!(doc.rules.write_roots, vec!["$workspace"]);
        assert_eq!(doc.rules.unattended, UnattendedApprovalMode::Deny);
        assert!(!String::from_utf8(bytes).unwrap().contains("api_key"));
    }
    assert_eq!(
        Builtin::Restricted.document().rules.access,
        AccessMode::ReadOnly
    );
    assert_eq!(
        Builtin::Autonomous.document().rules.access,
        AccessMode::Unrestricted
    );
}
#[test]
fn unknown_future_secret_and_unsupported_fields_fail_closed() {
    let bytes = Builtin::Balanced.document().encode().unwrap();
    for key in [
        "env",
        "api_key",
        "sandbox",
        "network",
        "allowed_tools",
        "max_runtime_secs",
    ] {
        let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        v["rules"][key] = serde_json::json!("secret");
        assert_eq!(
            ProfileDocument::decode(&serde_json::to_vec(&v).unwrap()).unwrap_err(),
            Error::Invalid
        );
    }
    let mut v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    v["schema"] = 2.into();
    assert!(ProfileDocument::decode(&serde_json::to_vec(&v).unwrap()).is_err());
    assert!(ProfileDocument::decode(&vec![b' '; MAX_DOCUMENT + 1]).is_err());
}
#[test]
fn layers_replace_rules_with_deterministic_per_field_provenance() {
    let temp = root();
    let patch = Overrides {
        access: Some(AccessMode::Unrestricted),
        ..Overrides::default()
    };
    let layers = vec![
        Layer::new(LayerKind::Global, "global", patch.clone()).unwrap(),
        Layer::new(
            LayerKind::Session,
            "session",
            Overrides {
                access: Some(AccessMode::ReadOnly),
                ..patch
            },
        )
        .unwrap(),
    ];
    let first = resolve_test(temp.path(), &rules(), &layers, None).unwrap();
    assert_eq!(first.rules().access, AccessMode::ReadOnly);
    assert_eq!(first.provenance()["access"].len(), 3);
    assert_eq!(
        first.digest(),
        resolve_test(temp.path(), &rules(), &layers, None)
            .unwrap()
            .digest()
    );
    let mut reversed = layers;
    reversed.reverse();
    assert!(resolve_test(temp.path(), &rules(), &reversed, None).is_err());
}
#[test]
fn ceiling_intersects_grants_unions_denials_and_limits_both_approval_modes() {
    let temp = root();
    let extra = root();
    let mut base = rules();
    base.access = AccessMode::Unrestricted;
    base.unattended = UnattendedApprovalMode::Allow;
    base.read_roots.push(extra.path().to_str().unwrap().into());
    base.inherit_env.push("SECRET_TOKEN".into());
    let mut ceiling = CeilingDocument {
        schema: 1,
        rules: rules(),
    };
    ceiling.rules.access = AccessMode::ReadOnly;
    ceiling.rules.deny_commands.push("curl".into());
    let effective = resolve_test(temp.path(), &base, &[], Some(&ceiling)).unwrap();
    assert_eq!(effective.rules().access, AccessMode::ReadOnly);
    assert_eq!(effective.rules().unattended, UnattendedApprovalMode::Deny);
    assert_eq!(
        effective.rules().read_roots,
        vec![temp.path().canonicalize().unwrap()]
    );
    assert!(
        !effective
            .rules()
            .inherit_env
            .contains(&"SECRET_TOKEN".into())
    );
    assert!(effective.rules().deny_commands.contains(&"curl".into()));
    assert_eq!(
        effective.provenance()["read_roots"].last().unwrap().source,
        "system-ceiling"
    );
}
#[test]
fn workspace_must_fit_ceiling_before_implicit_policy_grant() {
    let temp = root();
    let outside = root();
    let mut ceiling = CeilingDocument {
        schema: 1,
        rules: rules(),
    };
    ceiling.rules.read_roots = vec![outside.path().to_str().unwrap().into()];
    assert_eq!(
        resolve_test(temp.path(), &rules(), &[], Some(&ceiling)).unwrap_err(),
        Error::WorkspaceDenied
    );
    ceiling.rules.read_roots = vec!["$workspace".into()];
    ceiling.rules.write_roots = vec![];
    assert_eq!(
        resolve_test(temp.path(), &rules(), &[], Some(&ceiling)).unwrap_err(),
        Error::WorkspaceDenied
    );
}
#[test]
fn containment_and_normalization_are_conservative() {
    let temp = root();
    std::fs::create_dir(temp.path().join("child")).unwrap();
    let mut base = rules();
    base.read_roots.extend([
        temp.path().join("child").to_str().unwrap().into(),
        "$workspace".into(),
    ]);
    let effective = resolve_test(temp.path(), &base, &[], None).unwrap();
    assert_eq!(effective.rules().read_roots.len(), 1);
    base.read_roots = vec!["../escape".into()];
    assert!(resolve_test(temp.path(), &base, &[], None).is_err());
}
#[test]
fn escalation_binds_both_policies_and_workspace_identity() {
    let temp = root();
    let old = resolve_test(temp.path(), &rules(), &[], None).unwrap();
    let mut broad = rules();
    broad.access = AccessMode::Unrestricted;
    let new = resolve_test(temp.path(), &broad, &[], None).unwrap();
    let change = transition(&old, &new).unwrap();
    assert!(change.requires_confirmation());
    assert!(change.confirm(change.digest()).is_ok());
    assert!(change.confirm(old.digest()).is_err());
    assert!(!transition(&new, &old).unwrap().requires_confirmation());
    assert!(!transition(&old, &old).unwrap().requires_confirmation());
    let other = root();
    let other = resolve_test(other.path(), &rules(), &[], None).unwrap();
    assert!(transition(&old, &other).is_err());
}
#[test]
fn every_new_grant_or_removed_deny_requires_confirmation() {
    let temp = root();
    let extra = root();
    let old = resolve_test(temp.path(), &rules(), &[], None).unwrap();
    for kind in 0..5 {
        let mut r = rules();
        match kind {
            0 => r.unattended = UnattendedApprovalMode::Allow,
            1 => r.read_roots.push(extra.path().to_str().unwrap().into()),
            2 => r.write_roots.push(extra.path().to_str().unwrap().into()),
            3 => {
                r.deny_commands.pop();
            }
            _ => r.inherit_env.push("EXTRA".into()),
        };
        assert!(
            transition(&old, &resolve_test(temp.path(), &r, &[], None).unwrap())
                .unwrap()
                .requires_confirmation()
        );
    }
}
#[test]
fn malformed_names_roots_and_excessive_collections_are_rejected() {
    let mut doc = Builtin::Balanced.document();
    doc.name = "../bad".into();
    assert!(doc.encode().is_err());
    doc.name = "okay".into();
    doc.rules.inherit_env = vec!["TOKEN=value".into()];
    assert!(doc.encode().is_err());
    doc.rules = rules();
    doc.rules.deny_commands = vec!["sh".into(); MAX_ITEMS + 1];
    assert!(doc.encode().is_err());
}

#[test]
fn same_value_override_and_profile_revision_remain_visible_in_identity() {
    let temp = root();
    let mut doc = Builtin::Balanced.document();
    let first = resolve_test(
        temp.path(),
        &rules(),
        &[Layer::from_profile(LayerKind::Global, &doc).unwrap()],
        None,
    )
    .unwrap();
    assert_eq!(first.provenance()["access"].len(), 2);
    doc.revision += 1;
    let next = resolve_test(
        temp.path(),
        &rules(),
        &[Layer::from_profile(LayerKind::Global, &doc).unwrap()],
        None,
    )
    .unwrap();
    assert_ne!(first.digest(), next.digest());
    let unchanged = transition(&first, &first).unwrap();
    let changed = transition(&first, &next).unwrap();
    assert!(changed.confirm(unchanged.digest()).is_err());
}

#[cfg(unix)]
#[test]
fn recreated_workspace_and_symlink_aliases_have_honest_identity() {
    use std::os::unix::fs::symlink;
    let temp = root();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let original = resolve_test(&workspace, &rules(), &[], None).unwrap();
    symlink(&workspace, temp.path().join("alias")).unwrap();
    let alias = resolve_test(&temp.path().join("alias"), &rules(), &[], None).unwrap();
    assert_eq!(original.digest(), alias.digest());
    std::fs::rename(&workspace, temp.path().join("old")).unwrap();
    std::fs::create_dir(&workspace).unwrap();
    let replacement = resolve_test(&workspace, &rules(), &[], None).unwrap();
    assert!(transition(&original, &replacement).is_err());
}

#[test]
fn ceiling_normalized_containment_can_narrow_extra_root_without_adding_grants() {
    let temp = root();
    let outside = root();
    std::fs::create_dir(outside.path().join("allowed")).unwrap();
    let mut base = rules();
    base.read_roots
        .push(outside.path().to_str().unwrap().into());
    let mut ceiling = CeilingDocument {
        schema: 1,
        rules: rules(),
    };
    ceiling
        .rules
        .read_roots
        .push(outside.path().join("allowed").to_str().unwrap().into());
    let effective = resolve_test(temp.path(), &base, &[], Some(&ceiling)).unwrap();
    assert!(
        effective
            .rules()
            .read_roots
            .contains(&outside.path().join("allowed"))
    );
    assert!(
        !effective
            .rules()
            .read_roots
            .contains(&outside.path().to_path_buf())
    );
    assert!(
        !transition(
            &resolve_test(temp.path(), &base, &[], None).unwrap(),
            &effective
        )
        .unwrap()
        .requires_confirmation()
    );
}
