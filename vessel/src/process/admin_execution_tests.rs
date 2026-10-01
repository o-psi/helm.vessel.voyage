//! Exact metadata checks only. No configured host user, Root child or provider is used.
use super::*;
use crate::process::{identity_accounts::test_fixtures as f, test_support::Fixture};

fn provision_record() -> Provision {
    Provision {
        schema: 1,
        identity: f::identity(AuthorityClass::Administrator),
        config_path: "/fixture/control/administrator-config/launch.json".into(),
        data_directory: "/fixture/control/administrator-data/accounts".into(),
        config_directory: "/fixture/control/administrator-config/policy".into(),
        workspace_roots: vec!["/fixture/work".into()],
    }
}
fn binding(
    identity: &ConfiguredExecutionIdentity,
    registration: &ProcessRegistration,
    facts: &ReviewFacts,
) -> ExecutionBinding {
    ExecutionBinding {
        session_id: registration.session_id,
        incarnation: registration.incarnation,
        identity: identity.identity.clone(),
        account_context: identity.account_context.clone(),
        peer_uids: ProcessPeerUids {
            supervisor: 0,
            runtime: identity.uid,
        },
        administrator_grant_id: Some(Uuid::new_v4()),
        host_identity_digest: facts.host_identity_digest.clone(),
        policy_digest: facts.policy_digest.clone(),
    }
}

#[test]
fn capability_observation_preserves_exact_fields_and_refuses_missing_extra_oversized_or_non_utf8_reports()
 {
    let lines = [
        "CapEff:\t0000000000000000",
        "CapPrm:\t0000000000000000",
        "CapInh:\t0000000000000000",
        "CapBnd:\t000001ffffffffff",
        "NoNewPrivs:\t1",
    ];
    let body = format!("Name:\tfixture\n{}\nUid:\t1000\n", lines.join("\n"));
    assert_eq!(authority_status(body.as_bytes()).unwrap(), lines);
    for missing in 0..lines.len() {
        let value = lines
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != missing)
            .map(|(_, line)| *line)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(authority_status(value.as_bytes()).is_err());
    }
    assert!(authority_status(format!("{body}CapEff:\t1\n").as_bytes()).is_err());
    assert!(authority_status(&vec![b'x'; 16385]).is_err());
    assert!(authority_status(&[0xff]).is_err());
    let changed = body.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0");
    assert_ne!(
        hash(
            b"host",
            &serde_json::to_vec(&authority_status(body.as_bytes()).unwrap()).unwrap()
        ),
        hash(
            b"host",
            &serde_json::to_vec(&authority_status(changed.as_bytes()).unwrap()).unwrap()
        )
    );
}

#[test]
fn administrator_provision_requires_explicit_class_enabled_root_uid_and_bounded_workspace_inventory()
 {
    provision_shape(&provision_record()).unwrap();
    for change in 0..7 {
        let mut record = provision_record();
        match change {
            0 => record.schema = 0,
            1 => record.identity.enabled = false,
            2 => record.identity.authority = AuthorityClass::Ordinary,
            3 => record.identity.authority = AuthorityClass::Unknown,
            4 => record.identity.uid = 1000,
            5 => record.workspace_roots.clear(),
            _ => record.workspace_roots = vec!["/fixture/work".into(); 33],
        };
        assert!(provision_shape(&record).is_err(), "change {change}");
    }
    let mut record = provision_record();
    record.workspace_roots = vec!["/fixture/work".into(); 32];
    provision_shape(&record).unwrap();
}

#[test]
fn administrator_namespace_shape_cannot_select_a_login_or_sibling_namespace() {
    let root = Path::new("/fixture/control");
    provision_namespace(root, &provision_record()).unwrap();
    for path in [
        "/root/.local/share",
        "/fixture/control/administrator-data-other",
        "/other/administrator-data",
    ] {
        let mut record = provision_record();
        record.data_directory = path.into();
        assert!(provision_namespace(root, &record).is_err());
    }
    for path in [
        "/root/.config",
        "/fixture/control/administrator-config-other",
        "/other/administrator-config",
    ] {
        let mut record = provision_record();
        record.config_directory = path.into();
        assert!(provision_namespace(root, &record).is_err());
    }
}

#[test]
fn namespace_pin_binds_exact_session_command_identity_and_entire_provision() {
    let record = provision_record();
    let registration = f::registration();
    let original = NamespacePin {
        schema: 1,
        session: registration.session_id,
        command: registration.command_id,
        identity: record.identity.identity.clone(),
        provision_digest: provision_digest(&record).unwrap(),
    };
    namespace_current(&original, &registration, &record.identity, &record).unwrap();
    for change in 0..6 {
        let mut pin: NamespacePin =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        match change {
            0 => pin.schema = 2,
            1 => pin.session = Uuid::new_v4(),
            2 => pin.command = Uuid::new_v4(),
            3 => pin.identity.id = Uuid::new_v4(),
            4 => pin.identity.revision = NonZeroU64::new(2).unwrap(),
            _ => pin.provision_digest = "f".repeat(64),
        };
        assert!(namespace_current(&pin, &registration, &record.identity, &record).is_err());
    }
    for change in 0..10 {
        let mut changed = record.clone();
        match change {
            0 => {
                changed.config_path = "/fixture/control/administrator-config/different.json".into()
            }
            1 => changed.data_directory = "/fixture/control/administrator-data/other".into(),
            2 => changed.config_directory = "/fixture/control/administrator-config/other".into(),
            3 => changed.workspace_roots.push("/fixture/other".into()),
            4 => changed.identity.account_context.id = Uuid::new_v4(),
            5 => changed.identity.account_context.revision = NonZeroU64::new(2).unwrap(),
            6 => changed.identity.home = "/fixture/other-home".into(),
            7 => changed.identity.supplementary_groups.push(30),
            8 => changed.identity.label = "Different reviewed label".into(),
            _ => changed.schema = 2,
        };
        assert_ne!(
            provision_digest(&changed).unwrap(),
            original.provision_digest
        );
        assert!(namespace_current(&original, &registration, &changed.identity, &changed).is_err());
    }
    assert_ne!(
        hash(b"administrator", b"same bytes"),
        hash(b"ordinary", b"same bytes")
    );
}

#[test]
fn identity_preflight_requires_reported_uid_gid_and_exact_normalized_groups() {
    let mut record = provision_record();
    record.identity.supplementary_groups = vec![20, 10, 20];
    let mut facts = f::config_facts(&record.identity);
    facts.supplementary_groups = vec![10, 20];
    preflight_current(&record, &facts).unwrap();
    for change in 0..5 {
        let mut changed = facts.clone();
        match change {
            0 => changed.uid = 1000,
            1 => changed.gid = 1000,
            2 => changed.supplementary_groups = vec![20, 10],
            3 => changed.supplementary_groups = vec![10, 20, 20],
            _ => changed.supplementary_groups = vec![10],
        };
        assert!(preflight_current(&record, &changed).is_err());
    }
}

#[test]
fn restart_may_change_process_token_and_command_but_not_reviewed_session_namespace_or_peer_identity()
 {
    let previous = f::registration();
    let mut next = previous.clone();
    next.incarnation = Uuid::new_v4();
    next.command_id = Uuid::new_v4();
    next.token = "new-private-process-token".into();
    next.restart_from = Some(previous.incarnation);
    restart_current(&previous, &next).unwrap();
    for change in 0..5 {
        let mut changed = next.clone();
        match change {
            0 => changed.session_id = Uuid::new_v4(),
            1 => changed.restart_from = None,
            2 => changed.restart_from = Some(Uuid::new_v4()),
            3 => changed.config_path = Some("/fixture/other-config".into()),
            _ => {
                changed.peer_uids = Some(ProcessPeerUids {
                    supervisor: 0,
                    runtime: 1001,
                })
            }
        };
        assert!(restart_current(&previous, &changed).is_err());
    }
}

#[test]
fn running_administrator_metadata_requires_every_reviewed_binding_and_account_context() {
    let identity = f::identity(AuthorityClass::Administrator);
    let registration = f::registration();
    let facts = f::facts(&identity, &registration, &f::connection());
    let binding = binding(&identity, &registration, &facts);
    running_review_current(&registration, &identity, &binding, &facts).unwrap();
    for change in 0..6 {
        let mut changed = binding.clone();
        match change {
            0 => changed.session_id = Uuid::new_v4(),
            1 => changed.incarnation = Uuid::new_v4(),
            2 => changed.identity.id = Uuid::new_v4(),
            3 => changed.account_context.id = Uuid::new_v4(),
            4 => changed.host_identity_digest = "0".repeat(64),
            _ => changed.policy_digest = "0".repeat(64),
        };
        assert!(running_review_current(&registration, &identity, &changed, &facts).is_err());
    }
    for change in 0..4 {
        let mut changed = facts.clone();
        match change {
            0 => changed.session_id = Uuid::new_v4(),
            1 => changed.identity.id = Uuid::new_v4(),
            2 => changed.account_context.id = Uuid::new_v4(),
            _ => changed.workspace = "/fixture/different-work".into(),
        };
        assert!(running_review_current(&registration, &identity, &binding, &changed).is_err());
    }
}

#[test]
fn readiness_requires_matching_reported_identity_and_release_and_keeps_uncertain_cleanup_obligations()
 {
    let identity = f::identity(AuthorityClass::Administrator);
    let registration = f::registration();
    let facts = f::facts(&identity, &registration, &f::connection());
    let observed = f::observed(&identity, facts.incarnation);
    assert!(matches!(
        observed_launch(&facts, Ok(observed.clone())),
        ExecutionOutcome::Ready { .. }
    ));
    for change in 0..3 {
        let result = if change == 2 {
            Err(anyhow::anyhow!(
                "private /fixture/account details must not escape"
            ))
        } else {
            let mut changed = observed.clone();
            if change == 0 {
                changed.identity.id = Uuid::new_v4();
            } else {
                changed.release_digest = "0".repeat(64);
            }
            Ok(changed)
        };
        let outcome = observed_launch(&facts, result);
        assert_eq!(
            outcome,
            ExecutionOutcome::Unconfirmed {
                cleanup_obligations: vec![facts.session_id]
            }
        );
        assert!(
            !serde_json::to_string(&outcome)
                .unwrap()
                .contains("private /fixture")
        );
    }
}

#[tokio::test]
async fn ordinary_identity_uses_no_administrator_namespace_review_or_root_fallback() {
    let root = Fixture::new();
    let ordinary = f::identity(AuthorityClass::Ordinary);
    let previous = root.registration();
    let mut next = previous.clone();
    next.incarnation = Uuid::new_v4();
    assert!(
        runtime_namespace(&root.0, &previous, &ordinary)
            .unwrap()
            .is_none()
    );
    carry_namespace(&root.0, &previous, &next, &ordinary).unwrap();
    verify_running(&root.0, &next, &ordinary).await.unwrap();
    assert!(!root.0.join("administrator-launches").exists());
    assert!(!root.0.join("administrator-data").exists());
}

#[tokio::test]
async fn public_nil_review_and_unprovisioned_root_paths_create_no_administrator_effects() {
    let root = Fixture::new();
    let supervisor = root.supervisor().await;
    let grant = root.connection();
    let result = supervisor
        .execution_operation(
            &grant,
            ExecutionOperation::Prepare {
                review_id: Uuid::nil(),
                command_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                workspace: root.0.clone(),
                identity: f::identity(AuthorityClass::Administrator).identity,
            },
        )
        .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("nil execution review")
    );
    assert!(provision(&root.0).is_err());
    assert!(observation(&root.0, Uuid::new_v4(), Uuid::new_v4()).is_err());
    let db = database::open(&root.0).unwrap();
    assert_eq!(
        db.query_row("SELECT count(*) FROM voyages", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert!(!root.0.join("administrator-launches").exists());
    assert!(!root.0.join("guardians").exists());
}
