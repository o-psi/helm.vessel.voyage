//! Offline catalogue and authority regressions; never consult host accounts.
use super::*;
use crate::process::test_support::Fixture;
use voyage_protocol::accounts::{AccountBinding, Transport};

fn profile(name: &str) -> ExecutionProfile {
    ExecutionProfile {
        id: Uuid::new_v4(),
        name: name.into(),
        account: AccountBinding {
            account_id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            identity_generation: 1,
            connection_revision: 1,
            transport: Transport::OpenaiResponses,
        },
        model: "offline-model".into(),
        reasoning_effort: Some("high".into()),
        service_tier: Some("priority".into()),
    }
}

fn save(
    f: &Fixture,
    profile: ExecutionProfile,
    revision: u64,
    make_default: bool,
) -> VesselCommand {
    VesselCommand::SaveProfile {
        command_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        expected_revision: revision,
        profile,
        make_default,
    }
}

fn delete(f: &Fixture, profile_id: Uuid, revision: u64) -> VesselCommand {
    VesselCommand::DeleteProfile {
        command_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        expected_revision: revision,
        profile_id,
    }
}

fn apply(f: &Fixture, command: &VesselCommand) -> ProfileCatalogue {
    transact(&f.0, None, Some((command, "owner"))).unwrap()
}

#[test]
fn bootstrap_waits_for_account_then_occurs_only_once() {
    let f = Fixture::new();
    assert_eq!(
        transact(&f.0, None, None).unwrap(),
        ProfileCatalogue::default()
    );
    let first = profile("Default");
    let initial = transact(&f.0, Some(first.clone()), None).unwrap();
    assert_eq!(initial.revision, 1);
    assert_eq!(initial.default_profile_id, Some(first.id));
    assert_eq!(initial.profiles, vec![first]);
    assert_eq!(
        transact(&f.0, Some(profile("Replacement")), None).unwrap(),
        initial
    );
}

#[test]
fn crud_duplicate_default_and_last_deletion_preserve_resolved_values() {
    let f = Fixture::new();
    let original = profile("Everyday");
    let created = apply(&f, &save(&f, original.clone(), 0, false));
    assert_eq!(created.revision, 1);
    assert_eq!(created.default_profile_id, Some(original.id));
    // A start receives values, not a live reference into the catalogue.
    let start_snapshot = serde_json::to_value(&created.profiles[0]).unwrap();
    let mut duplicate = original.clone();
    duplicate.id = Uuid::new_v4();
    duplicate.name = "Deep work".into();
    let copied = apply(&f, &save(&f, duplicate.clone(), 1, false));
    assert_eq!(copied.profiles.len(), 2);
    assert_eq!(copied.default_profile_id, Some(original.id));
    let selected = apply(
        &f,
        &VesselCommand::SetDefaultProfile {
            command_id: Uuid::new_v4(),
            workspace: f.0.clone(),
            expected_revision: 2,
            profile_id: duplicate.id,
        },
    );
    assert_eq!(selected.default_profile_id, Some(duplicate.id));
    let mut edited = original.clone();
    edited.name = "Renamed".into();
    edited.model = "replacement-model".into();
    edited.reasoning_effort = None;
    edited.service_tier = None;
    edited.account = profile("Another account").account;
    let updated = apply(&f, &save(&f, edited.clone(), 3, true));
    assert_eq!(updated.profiles[0], edited);
    assert_eq!(updated.default_profile_id, Some(original.id));
    let remaining = apply(&f, &delete(&f, original.id, 4));
    assert_eq!(remaining.default_profile_id, Some(duplicate.id));
    assert_eq!(remaining.profiles, vec![duplicate.clone()]);
    let empty = apply(&f, &delete(&f, duplicate.id, 5));
    assert_eq!(empty.revision, 6);
    assert!(empty.profiles.is_empty());
    assert_eq!(empty.default_profile_id, None);
    assert_eq!(
        transact(&f.0, Some(profile("Default")), None).unwrap(),
        empty
    );
    assert_eq!(
        serde_json::from_value::<ExecutionProfile>(start_snapshot).unwrap(),
        original
    );
}

#[test]
fn receipts_are_exact_actor_bound_and_survive_later_edits() {
    let f = Fixture::new();
    let first = profile("Everyday");
    let command = save(&f, first.clone(), 0, false);
    let receipt = apply(&f, &command);
    let second = save(&f, profile("Other"), 1, true);
    let current = apply(&f, &second);
    assert_eq!(apply(&f, &command), receipt);
    assert_eq!(transact(&f.0, None, None).unwrap(), current);
    let mut changed = command.clone();
    if let VesselCommand::SaveProfile { profile, .. } = &mut changed {
        profile.model = "different".into();
    }
    for (attempt, actor) in [(&changed, "owner"), (&command, "another-actor")] {
        let error = transact(&f.0, None, Some((attempt, actor))).unwrap_err();
        assert!(error.to_string().contains("identity conflict"), "{error:#}");
    }
    assert_eq!(transact(&f.0, None, None).unwrap(), current);
}

#[test]
fn stale_revisions_name_collisions_and_missing_ids_do_not_mutate() {
    let f = Fixture::new();
    let original = profile("Everyday");
    let initial = apply(&f, &save(&f, original.clone(), 0, true));
    let stale = save(&f, profile("New"), 0, false);
    let collision = save(&f, profile("EVERYDAY"), 1, false);
    let missing = delete(&f, Uuid::new_v4(), 1);
    let default_missing = VesselCommand::SetDefaultProfile {
        command_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        expected_revision: 1,
        profile_id: Uuid::new_v4(),
    };
    for (command, message) in [
        (&stale, "reload"),
        (&collision, "name already exists"),
        (&missing, "no longer exists"),
        (&default_missing, "no longer exists"),
    ] {
        let error = transact(&f.0, None, Some((command, "owner"))).unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
        assert_eq!(transact(&f.0, None, None).unwrap(), initial);
    }
    // A rejected command has no successful receipt; its corrected request can commit.
    let mut corrected = collision;
    if let VesselCommand::SaveProfile { profile, .. } = &mut corrected {
        profile.name = "Different".into();
    }
    assert_eq!(apply(&f, &corrected).revision, 2);
}

#[test]
fn rejects_invalid_text_and_nil_command_identity_atomically() {
    let f = Fixture::new();
    for name in ["", " padded", "trailing ", "line\nbreak", "escape\u{1b}"] {
        let command = save(&f, profile(name), 0, false);
        assert!(transact(&f.0, None, Some((&command, "owner"))).is_err());
    }
    let mut cases = vec![];
    let valid = profile("Valid");
    let mut p = valid.clone();
    p.id = Uuid::nil();
    cases.push(p);
    let mut p = valid.clone();
    p.name = "n".repeat(81);
    cases.push(p);
    let mut p = valid.clone();
    p.model = "m".repeat(257);
    cases.push(p);
    let mut p = valid.clone();
    p.reasoning_effort = Some(" ".into());
    cases.push(p);
    let mut p = valid.clone();
    p.service_tier = Some("x".repeat(65));
    cases.push(p);
    for p in cases {
        let command = save(&f, p, 0, false);
        assert!(transact(&f.0, None, Some((&command, "owner"))).is_err());
    }
    let mut command = save(&f, valid, 0, false);
    if let VesselCommand::SaveProfile { command_id, .. } = &mut command {
        *command_id = Uuid::nil();
    }
    assert!(transact(&f.0, None, Some((&command, "owner"))).is_err());
    assert_eq!(
        transact(&f.0, None, None).unwrap(),
        ProfileCatalogue::default()
    );
}

#[test]
fn concurrent_compare_and_swap_accepts_exactly_one_writer() {
    let f = Fixture::new();
    let initial = profile("Initial");
    transact(&f.0, Some(initial), None).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let commands = [
        save(&f, profile("First"), 1, false),
        save(&f, profile("Second"), 1, false),
    ];
    let results: Vec<_> = std::thread::scope(|threads| {
        let handles: Vec<_> = commands
            .iter()
            .map(|command| {
                let barrier = barrier.clone();
                let root = &f.0;
                threads.spawn(move || {
                    barrier.wait();
                    transact(root, None, Some((command, "owner")))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect()
    });
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    let rejected = results.iter().find_map(|r| r.as_ref().err()).unwrap();
    assert!(rejected.to_string().contains("reload"), "{rejected:#}");
    let current = transact(&f.0, None, None).unwrap();
    assert_eq!(current.revision, 2);
    assert_eq!(current.profiles.len(), 2);
}

#[tokio::test]
async fn scoped_humans_and_sessions_cannot_manage_even_with_account_use() {
    let f = Fixture::new();
    let supervisor = f.supervisor().await;
    let existing = profile("Owner-only catalogue");
    let initial = transact(&f.0, Some(existing.clone()), None).unwrap();
    let connection = f.connection();
    f.save_connection(&connection);
    let session = f.session();
    f.save_session(&session);
    for scope in [Scope::Connection(connection), Scope::Session(session)] {
        assert!(!can_manage(&scope));
        let error = supervisor
            .execution_profiles(delete(&f, existing.id, 1), scope)
            .unwrap_err();
        assert!(
            error.to_string().contains("full-access human connection"),
            "{error:#}"
        );
    }
    let mut full = f.connection();
    full.full_access = true;
    assert!(can_manage(&Scope::Connection(full)));
    assert!(can_manage(&Scope::Owner));
    let mut full_session = f.session();
    full_session.full_access = true;
    assert!(!can_manage(&Scope::Session(full_session)));
    assert_eq!(transact(&f.0, None, None).unwrap(), initial);
}

#[test]
fn scoped_catalogues_hide_unapproved_accounts_and_hidden_defaults() {
    let f = Fixture::new();
    let registry = Registry::new(f.0.join("isolated-accounts"));
    let visible = profile("Allowed");
    let hidden = profile("Private account label");
    let catalogue = ProfileCatalogue {
        revision: 9,
        profiles: vec![visible.clone(), hidden.clone()],
        default_profile_id: Some(hidden.id),
        can_manage: true,
    };
    let mut connection = f.connection();
    connection.accounts = vec![visible.account.account_id];
    let mut session = f.session();
    session.accounts = connection.accounts.clone();
    for scope in [Scope::Connection(connection), Scope::Session(session)] {
        let mut filtered = catalogue.clone();
        filter_catalogue(&mut filtered, &registry, &scope, &f.0);
        assert_eq!(filtered.profiles, vec![visible.clone()]);
        assert_eq!(filtered.default_profile_id, None);
        assert!(!filtered.can_manage);
        let encoded = serde_json::to_string(&filtered).unwrap();
        assert!(!encoded.contains(&hidden.name));
        assert!(!encoded.contains(&hidden.account.account_id.to_string()));
        assert!(!encoded.contains(&hidden.account.connection_id.to_string()));
    }
    let mut full = f.connection();
    full.full_access = true;
    for scope in [Scope::Owner, Scope::Connection(full)] {
        let mut filtered = catalogue.clone();
        filter_catalogue(&mut filtered, &registry, &scope, &f.0);
        assert_eq!(filtered, catalogue);
    }
}

#[test]
fn profile_wire_rejects_instructions_permissions_and_credentials() {
    let original = profile("Execution only");
    for field in [
        "instructions",
        "tools",
        "permissions",
        "workspace",
        "api_key",
    ] {
        let mut encoded = serde_json::to_value(&original).unwrap();
        encoded
            .as_object_mut()
            .unwrap()
            .insert(field.into(), json!("unrelated setting"));
        assert!(
            serde_json::from_value::<ExecutionProfile>(encoded).is_err(),
            "accepted {field}"
        );
    }
}

#[tokio::test]
async fn revoked_or_wrong_workspace_authority_cannot_delete_profiles() {
    let f = Fixture::new();
    let other = Fixture::new();
    let supervisor = f.supervisor().await;
    let original = profile("Default");
    let initial = transact(&f.0, Some(original.clone()), None).unwrap();
    let mut full = f.connection();
    full.full_access = true;
    f.save_connection(&full);
    let stale_scope = Scope::Connection(full.clone());
    full.revoked = true;
    f.save_connection(&full);
    assert!(
        supervisor
            .execution_profiles(delete(&f, original.id, 1), stale_scope)
            .is_err()
    );
    let scoped = f.connection();
    f.save_connection(&scoped);
    assert!(
        supervisor
            .execution_profiles(delete(&other, original.id, 1), Scope::Connection(scoped))
            .is_err()
    );
    assert_eq!(transact(&f.0, None, None).unwrap(), initial);
}

#[test]
fn exact_receipt_survives_account_validation_failure_but_new_mutations_do_not() {
    let f = Fixture::new();
    let command = save(&f, profile("Previously available account"), 0, true);
    let validations = std::cell::Cell::new(0);
    let receipt = transact_validated(&f.0, None, Some((&command, "owner")), || {
        validations.set(validations.get() + 1);
        Ok(())
    })
    .unwrap();
    assert_eq!(validations.get(), 1);
    // Simulate account removal after the original mutation committed. Receipt
    // resolution must not depend on the account remaining usable today.
    let unavailable = || -> Result<()> {
        validations.set(validations.get() + 1);
        anyhow::bail!("account no longer available")
    };
    let replay = transact_validated(&f.0, None, Some((&command, "owner")), unavailable).unwrap();
    assert_eq!(replay, receipt);
    assert_eq!(validations.get(), 1);

    let error =
        transact_validated(&f.0, None, Some((&command, "another-actor")), unavailable).unwrap_err();
    assert!(error.to_string().contains("identity conflict"), "{error:#}");
    assert_eq!(validations.get(), 1);

    let new_command = save(&f, profile("New mutation"), receipt.revision, false);
    let error =
        transact_validated(&f.0, None, Some((&new_command, "owner")), unavailable).unwrap_err();
    assert!(
        error.to_string().contains("account no longer available"),
        "{error:#}"
    );
    assert_eq!(validations.get(), 2);
    assert_eq!(transact(&f.0, None, None).unwrap(), receipt);
    // Rejection did not leave a successful receipt or consume the revision.
    let repaired =
        transact_validated(&f.0, None, Some((&new_command, "owner")), || Ok(())).unwrap();
    assert_eq!(repaired.revision, receipt.revision + 1);
    assert_eq!(repaired.profiles.len(), 2);
}
