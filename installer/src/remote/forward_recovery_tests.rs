use super::*;
use crate::fixture_tests::Fixture;
fn review(f: &Fixture) -> Review {
    Review {
        schema_version: 1,
        operation_id: "11111111-1111-4111-8111-111111111111".into(),
        original_operation: "22222222-2222-4222-8222-222222222222".into(),
        original_sha256: "a".repeat(64),
        old_release: "b".repeat(64),
        metadata_sha256: "c".repeat(64),
        running_release: "d".repeat(64),
        running_manifest_sha256: "e".repeat(64),
        target_release: "f".repeat(64),
        target_manifest_sha256: "0".repeat(64),
        staged_bin: f.root.join("stage/bin"),
        activation: service::Activation {
            active: true,
            enabled: false,
            unit_file_state: "disabled".into(),
            definition: Some("reviewed unit".into()),
            state: f.root.join("state"),
        },
        accounts: f.root.join("accounts"),
        source_evidence: serde_json::json!({"canonical_sha256":"pin","sessions":[]}),
        gateway_unit: "gateway.service".into(),
        gateway_path: f.root.join("units/gateway.service"),
        gateway_original: "old".into(),
        gateway_repaired: "new".into(),
        gateway_effective: "effective".into(),
        gateway_credentials: GatewayCredentials {
            effective_environment: String::new(),
            drop_in: None,
        },
        gateway_enablement: "disabled".into(),
        public_origin: "https://helm.invalid".into(),
    }
}
#[test]
fn positional_gateway_recovery_restores_flag_and_persistent_path_preserving_all_other_configuration()
 {
    let content = "[Service]\nEnvironmentFile=/private/gateway.env\nExecStart=/home/user/releases/approved/bin/vessel --bind 127.0.0.1:9480 --process-directory /home/user/state https://helm.invalid\nRestart=on-failure\n";
    let fixed = gateway_repair(
        content,
        Path::new("/home/user/releases/approved/bin/vessel"),
        Path::new("/home/user/install/current/bin/vessel"),
        "https://helm.invalid",
    )
    .unwrap();
    assert_eq!(
        fixed,
        "[Service]\nEnvironmentFile=/private/gateway.env\nExecStart=/home/user/install/current/bin/vessel --bind 127.0.0.1:9480 --process-directory /home/user/state --public-origin https://helm.invalid\nRestart=on-failure\n"
    );
}
#[test]
fn flagged_gateway_recovery_retains_origin_and_quoted_executable_semantics() {
    let content = "[Service]\nExecStart=:\"/home/user/releases/approved/bin/vessel\" --public-origin https://helm.invalid --process-directory /home/user/state --bind 127.0.0.1:9480\n";
    assert_eq!(
        gateway_repair(
            content,
            Path::new("/home/user/releases/approved/bin/vessel"),
            Path::new("/home/user/install/current/bin/vessel"),
            "https://helm.invalid"
        )
        .unwrap(),
        "[Service]\nExecStart=:/home/user/install/current/bin/vessel --public-origin https://helm.invalid --process-directory /home/user/state --bind 127.0.0.1:9480\n"
    );
}
#[test]
fn gateway_recovery_refuses_noncanonical_or_credential_origins_and_unmatched_commands() {
    let executable = Path::new("/home/user/approved/vessel");
    let next = Path::new("/home/user/current/vessel");
    for origin in [
        "http://helm.invalid",
        "https://user:secret@helm.invalid",
        "https://helm.invalid/path",
        "https://helm.invalid/?secret=1",
        "https://helm.invalid/#fragment",
    ] {
        assert!(
            gateway_repair(
                &format!(
                    "ExecStart={} --bind 127.0.0.1:9480 {origin}\n",
                    executable.display()
                ),
                executable,
                next,
                origin
            )
            .is_err()
        );
    }
    for content in [
        "ExecStart=/other/vessel --bind 127.0.0.1:9480 https://helm.invalid\n",
        "ExecStart=/home/user/approved/vessel https://helm.invalid --database unknown\n",
        "ExecStart=/home/user/approved/vessel https://helm.invalid\nExecStart=/other/vessel https://helm.invalid\n",
        "ExecStart=/home/user/approved/vessel --public-origin https://helm.invalid https://helm.invalid\n",
    ] {
        assert!(gateway_repair(content, executable, next, "https://helm.invalid").is_err());
    }
}
#[test]
fn actual_gateway_argument_fences_refuse_other_state_duplicate_origin_and_system_scope() {
    let state = Path::new("/home/user/state");
    for args in [
        "vessel\0--process-directory\0/home/user/other\0https://helm.invalid\0",
        "vessel\0--process-directory\0/home/user/state\0--public-origin\0https://helm.invalid\0https://helm.invalid\0",
        "vessel\0--process-directory\0/home/user/state\0--system-gateway-socket\0unknown\0https://helm.invalid\0",
        "vessel\0--process-directory\0/home/user/state\0--allow-insecure-loopback\0https://helm.invalid\0",
    ] {
        assert!(gateway_arguments(args.as_bytes(), state, "https://helm.invalid").is_err());
    }
    gateway_arguments(
        b"vessel\0--process-directory\0/home/user/state\0https://helm.invalid\0",
        state,
        "https://helm.invalid",
    )
    .unwrap();
}
#[test]
fn review_hash_pins_all_authority_context_and_mutation_target_before_any_manager_effect() {
    let f = Fixture::new();
    let baseline = review(&f);
    let approved = review_hash(&baseline).unwrap();
    for variant in 0..8 {
        let mut changed = review(&f);
        match variant {
            0 => changed.original_sha256 = "9".repeat(64),
            1 => changed.metadata_sha256 = "8".repeat(64),
            2 => changed.target_release = "7".repeat(64),
            3 => changed.accounts = f.root.join("other"),
            4 => changed.activation.state = f.root.join("other"),
            5 => changed.gateway_repaired = "unreviewed".into(),
            6 => changed.gateway_enablement = "enabled".into(),
            _ => changed.source_evidence = serde_json::json!({"canonical_sha256":"changed"}),
        };
        let mut record = Recovery {
            review: changed,
            phase: "reviewed".into(),
            proof: None,
        };
        assert!(apply(&mut record, &approved).is_err());
        assert_eq!(record.phase, "reviewed");
    }
    f.done();
}
#[test]
fn original_receipt_mutation_is_refused_and_not_rewritten_or_replayed() {
    let f = Fixture::new();
    let mut reviewed = review(&f);
    let original_path = path(&reviewed.original_operation).unwrap();
    fs::write(&original_path, b"original uncertain receipt").unwrap();
    reviewed.original_sha256 = files::hash(&original_path).unwrap();
    original_unchanged(&reviewed).unwrap();
    fs::write(&original_path, b"independent mutation").unwrap();
    assert!(original_unchanged(&reviewed).is_err());
    assert_eq!(fs::read(original_path).unwrap(), b"independent mutation");
    f.done();
}
#[test]
fn uncertain_recovery_phase_never_replays_effects_even_with_original_review_hash() {
    let f = Fixture::new();
    for phase in [
        "applying",
        "quiescent",
        "source-reconciled",
        "committing",
        "unconfirmed",
        "complete",
    ] {
        let reviewed = review(&f);
        let approved = review_hash(&reviewed).unwrap();
        let mut record = Recovery {
            review: reviewed,
            phase: phase.into(),
            proof: None,
        };
        assert!(apply(&mut record, &approved).is_err());
        assert_eq!(record.phase, phase);
    }
    f.done();
}
#[test]
fn live_supervisor_namespace_requires_one_exact_directory_argument() {
    let state = Path::new("/reviewed/state");
    for args in [
        "vessel\0local-serve\0--directory\0/other/state\0",
        "vessel\0local-serve\0--directory\0/reviewed/state\0--directory\0/other/state\0",
        "vessel\0local-serve\0",
    ] {
        assert!(!legacy_namespace_matches_fields(
            state,
            Path::new("/accounts"),
            args.as_bytes()
        ));
    }
    assert!(legacy_namespace_matches_fields(
        state,
        Path::new("/accounts"),
        b"vessel\0local-serve\0--directory\0/reviewed/state\0"
    ));
}

#[test]
fn gateway_health_observation_accepts_only_one_literal_loopback_nonzero_bind() {
    for args in [
        "vessel\0--bind\x000.0.0.0:9480\0",
        "vessel\0--bind\0localhost:9480\0",
        "vessel\0--bind\x00127.0.0.1:0\0",
        "vessel\0--bind\x00127.0.0.1:9480\0--bind\x00127.0.0.1:9481\0",
        "vessel\0",
    ] {
        assert!(gateway_address(args.as_bytes()).is_err());
    }
    assert_eq!(
        gateway_address(b"vessel\0--bind\x00127.0.0.1:9480\0").unwrap(),
        "127.0.0.1:9480".parse::<std::net::SocketAddr>().unwrap()
    );
}

#[test]
fn complete_label_without_pinned_forward_proof_cannot_supersede_original_uncertainty() {
    let f = Fixture::new();
    let mut reviewed = review(&f);
    let original_path = path(&reviewed.original_operation).unwrap();
    let original = Record {
        operation_id: reviewed.original_operation.clone(),
        channel: "nightly".into(),
        phase: "unconfirmed".into(),
        message: "unknown".into(),
        created_at: 1,
        updated_at: 1,
        current_release: reviewed.old_release.clone(),
        release_id: Some(reviewed.running_release.clone()),
        version: None,
        description: None,
        bin_dir: None,
        staging_root: None,
        gateways: vec![],
        contracts_sha256: None,
        supervisor_activation: None,
        legacy_mode: false,
        legacy_proof: None,
        legacy_accounts: None,
    };
    files::atomic_json(&original_path, &original).unwrap();
    reviewed.original_sha256 = files::hash(&original_path).unwrap();
    let mut record = Recovery {
        review: reviewed,
        phase: "reviewed".into(),
        proof: None,
    };
    save_recovery(&mut record, "unconfirmed").unwrap();
    assert!(!supersedes(&original).unwrap());
    save_recovery(&mut record, "complete").unwrap();
    assert!(supersedes(&original).is_err());
    assert_eq!(
        files::hash(&original_path).unwrap(),
        record.review.original_sha256
    );
    f.done();
}

fn source_review() -> serde_json::Value {
    serde_json::json!({"state_sha256":"a".repeat(64),"review_state_sha256":"b".repeat(64),"canonical_sha256":"c".repeat(64),"accounts_sha256":"d".repeat(64),"sessions":["11111111-1111-4111-8111-111111111111"],"session_count":1,"recovery_mode":"forward-existing-schema2"})
}
#[test]
fn semantic_notification_review_tolerates_only_raw_header_drift_before_quiescence() {
    let approved = source_review();
    let mut observed = approved.clone();
    observed["state_sha256"] = "e".repeat(64).into();
    ensure_reviewed_source(&approved, &observed, false).unwrap();
    let object = observed.as_object_mut().unwrap();
    object.remove("sessions");
    object.insert("backup_sha256".into(), "f".repeat(64).into());
    ensure_reviewed_source(&approved, &observed, true).unwrap();
    assert_eq!(approved["state_sha256"], "a".repeat(64)); // Review is never rewritten.
}
#[test]
fn semantic_review_refuses_logical_clock_schema_accounts_context_and_missing_claim_changes() {
    let approved = source_review();
    for claim in [
        "review_state_sha256",
        "canonical_sha256",
        "accounts_sha256",
        "session_count",
        "sessions",
        "recovery_mode",
    ] {
        let mut changed = approved.clone();
        changed[claim] = serde_json::json!("independent effect");
        assert!(ensure_reviewed_source(&approved, &changed, false).is_err());
    }
    for missing in ["review_state_sha256", "state_sha256", "accounts_sha256"] {
        let mut changed = approved.clone();
        changed.as_object_mut().unwrap().remove(missing);
        assert!(ensure_reviewed_source(&approved, &changed, false).is_err());
    }
    let mut unknown = approved.clone();
    unknown["unreviewed_authority"] = serde_json::json!(true);
    assert!(ensure_reviewed_source(&approved, &unknown, false).is_err());
    let mut old_review = approved.clone();
    old_review
        .as_object_mut()
        .unwrap()
        .remove("review_state_sha256");
    assert!(ensure_reviewed_source(&old_review, &approved, false).is_err());
}

fn observed_format_fixture(f: &Fixture) -> (Manifest, Manifest, serde_json::Value, String) {
    let source = Manifest::inspect(&crate::fixture_tests::release(f, "source", "1.0.3")).unwrap();
    let mut target =
        Manifest::inspect(&crate::fixture_tests::release(f, "target", "1.0.3")).unwrap();
    target.update_compatibility = Some(crate::install::release::UpdateCompatibility {
        schema_version: 1,
        formats: current_forward_formats().unwrap(),
        implementation_sha256: "a".repeat(64),
        build_inputs_sha256: "b".repeat(64),
    });
    let executing = target.binaries["voyage-installer"].sha256.clone();
    let evidence = serde_json::json!({"recovery_mode":"forward-existing-schema2", "session_count":2,
        "observed_formats":{"catalogue_schema":2,"journal_schemas":[12,20],"process_protocols":[1]}});
    (source, target, evidence, executing)
}

#[test]
fn undeclared_source_forward_admission_preserves_manifest_and_never_supplies_rollback_readers() {
    let f = Fixture::new();
    let (source, target, evidence, executing) = observed_format_fixture(&f);
    let original = serde_json::to_vec(&source).unwrap();
    forward_formats(&source, &target, &evidence, &executing).unwrap();
    assert_eq!(serde_json::to_vec(&source).unwrap(), original);
    assert!(source.update_compatibility.is_none());
    assert!(rollback_formats(&source, &target).is_err());
}

#[test]
fn undeclared_forward_admission_refuses_a_different_installer_and_every_changed_current_contract() {
    let f = Fixture::new();
    let (source, target, evidence, executing) = observed_format_fixture(&f);
    assert!(forward_formats(&source, &target, &evidence, &"0".repeat(64)).is_err());
    for name in [
        "catalogue_read",
        "catalogue_write",
        "journal_read",
        "journal_write",
        "process_protocol",
        "vessel_protocol",
        "execution_identity",
    ] {
        let mut changed = target.clone();
        changed
            .update_compatibility
            .as_mut()
            .unwrap()
            .formats
            .insert(name.into(), vec![999]);
        assert!(
            forward_formats(&source, &changed, &evidence, &executing).is_err(),
            "{name}"
        );
    }
    let mut missing = target;
    missing.update_compatibility = None;
    assert!(forward_formats(&source, &missing, &evidence, &executing).is_err());
}

#[test]
fn undeclared_forward_admission_requires_complete_actual_formats_and_exact_forward_mode() {
    let f = Fixture::new();
    let (source, target, evidence, executing) = observed_format_fixture(&f);
    for observed in [
        serde_json::Value::Null,
        serde_json::json!({"catalogue_schema":1,"journal_schemas":[12,20],"process_protocols":[1]}),
        serde_json::json!({"catalogue_schema":3,"journal_schemas":[12,20],"process_protocols":[1]}),
        serde_json::json!({"catalogue_schema":2,"journal_schemas":[12,21],"process_protocols":[1]}),
        serde_json::json!({"catalogue_schema":2,"journal_schemas":[12,12],"process_protocols":[1]}),
        serde_json::json!({"catalogue_schema":2,"journal_schemas":[],"process_protocols":[1]}),
        serde_json::json!({"catalogue_schema":2,"journal_schemas":[12],"process_protocols":[]}),
        serde_json::json!({"catalogue_schema":2,"journal_schemas":[12],"process_protocols":[2]}),
        serde_json::json!({"catalogue_schema":2,"journal_schemas":[12],"process_protocols":[1],"extra":true}),
    ] {
        let mut changed = evidence.clone();
        changed["observed_formats"] = observed;
        assert!(forward_formats(&source, &target, &changed, &executing).is_err());
    }
    let mut wrong_mode = evidence;
    wrong_mode["recovery_mode"] = serde_json::json!("restore");
    assert!(forward_formats(&source, &target, &wrong_mode, &executing).is_err());
}

#[test]
fn declared_source_forward_recovery_retains_ordinary_rollback_guard_without_fallback() {
    let f = Fixture::new();
    let (mut source, target, evidence, executing) = observed_format_fixture(&f);
    source.update_compatibility = target.update_compatibility.clone();
    forward_formats(&source, &target, &serde_json::Value::Null, "unneeded").unwrap();
    source
        .update_compatibility
        .as_mut()
        .unwrap()
        .formats
        .insert("catalogue_read".into(), vec![1]);
    assert!(forward_formats(&source, &target, &evidence, &executing).is_err());
}

#[test]
fn gateway_credential_dropin_is_exact_private_owned_and_context_pinned() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let unit = f.root.join("gateway.service");
    let directory = f.root.join("gateway.service.d");
    fs::create_dir(&directory).unwrap();
    let path = directory.join("credentials.conf");
    let content = "[Service]\nEnvironment=VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key\n";
    fs::write(&path, content).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let paths = path.to_str().unwrap();
    let env = "VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key";
    let live = b"HOME=/private\0VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key\0";
    let key = Some(std::ffi::OsStr::new("/run/owned/key"));
    let pinned = reviewed_gateway_credentials(paths, &unit, env, live, key).unwrap();
    assert_eq!(
        pinned,
        reviewed_gateway_credentials(paths, &unit, env, live, key).unwrap()
    );
    assert_eq!(pinned.drop_in.as_ref().unwrap().key_path, "/run/owned/key");
    assert!(
        reviewed_gateway_credentials(&format!("{paths} {paths}"), &unit, env, live, key).is_err()
    );
    assert!(
        reviewed_gateway_credentials(paths, &unit, "VOYAGE_CREDENTIAL_KEY_FILE=/other", live, key)
            .is_err()
    );
    assert!(
        reviewed_gateway_credentials(
            paths,
            &unit,
            env,
            b"VOYAGE_CREDENTIAL_KEY_FILE=/other\0",
            key
        )
        .is_err()
    );
    assert!(reviewed_gateway_credentials(paths, &unit, env, live, None).is_err());
    assert!(reviewed_gateway_credentials(paths,&unit,env,b"VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key\0VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key\0",key).is_err());
    for bad in [
        "[Service]\nExecStart=/arbitrary\n",
        "[Service]\nEnvironment=OTHER=secret\n",
        "[Service]\nEnvironment=VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key\nRestart=always\n",
        "[Service]\nEnvironment=VOYAGE_CREDENTIAL_KEY_FILE=/run/owned/key\nEnvironment=VOYAGE_CREDENTIAL_KEY_FILE=/other\n",
    ] {
        fs::write(&path, bad).unwrap();
        assert!(reviewed_gateway_credentials(paths, &unit, env, live, key).is_err());
    }
    fs::write(&path, content).unwrap();
    let before = credential_drop_in(&path, &unit).unwrap();
    assert!(
        credential_drop_in_for_owner(&path, &unit, unsafe { libc::geteuid() } + 1).is_err(),
        "unowned override cannot be reviewed"
    );
    fs::rename(&path, directory.join("retained.conf")).unwrap();
    fs::write(&path, content).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_ne!(
        before,
        credential_drop_in(&path, &unit).unwrap(),
        "replacement inode cannot reuse approval"
    );
    fs::write(&path, content.replace("/run/owned/key", "/run/owned/new")).unwrap();
    assert_ne!(
        pinned.drop_in.unwrap(),
        credential_drop_in(&path, &unit).unwrap(),
        "changed bytes and namespace cannot reuse approval"
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(credential_drop_in(&path, &unit).is_err());
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(directory.join("retained.conf"), &path).unwrap();
    assert!(credential_drop_in(&path, &unit).is_err());
    fs::remove_file(&path).unwrap();
    fs::hard_link(directory.join("retained.conf"), &path).unwrap();
    assert!(credential_drop_in(&path, &unit).is_err());
    assert!(credential_drop_in(&path, &f.root.join("other.service")).is_err());
}

#[test]
fn copied_recovery_receipt_refuses_apply_status_and_supersession_without_effects() {
    let f = Fixture::new();
    let record = Recovery {
        review: review(&f),
        phase: "complete".into(),
        proof: None,
    };
    let other = "33333333-3333-4333-8333-333333333333";
    let path = record_path(other).unwrap();
    files::atomic_json(&path, &record).unwrap();
    let before = fs::read(&path).unwrap();
    assert!(
        load_recovery(&path)
            .err()
            .unwrap()
            .to_string()
            .contains("identity conflict")
    );
    for request in [
        vec!["status".into(), other.into()],
        vec![
            "apply".into(),
            other.into(),
            "--review".into(),
            review_hash(&record.review).unwrap(),
        ],
    ] {
        assert!(
            run(&request)
                .unwrap_err()
                .to_string()
                .contains("identity conflict")
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
