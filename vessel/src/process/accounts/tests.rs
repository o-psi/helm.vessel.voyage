use super::super::test_support::Fixture;
use super::*;

#[test]
fn owner_and_malformed_enrollment_actors_are_strictly_separated() {
    let f = Fixture::new();
    let connection = Uuid::new_v4();
    let owner = Scope::Owner.actor(&f.0);
    assert!(enrollment_authorized(&f.0, &owner, connection));
    assert!(
        Scope::Owner
            .check(&f.0, &f.0, ProcessRight::AccountUse)
            .is_ok()
    );
    assert!(
        Scope::Owner
            .check(&f.0, Path::new("relative"), ProcessRight::AccountUse)
            .is_err()
    );
    assert!(enrollment_authorized(
        &f.0,
        &EnrollmentActor {
            principal: "execution-host-owner".into(),
            workspace: "execution-host".into()
        },
        connection
    ));
    for principal in [
        "",
        "grant",
        "grant:a:b:1",
        "grant:a:b:c:extra",
        "execution-host-owner",
        "grant:00000000-0000-0000-0000-000000000000:00000000-0000-0000-0000-000000000000:bad",
    ] {
        assert!(
            !enrollment_authorized(
                &f.0,
                &EnrollmentActor {
                    principal: principal.into(),
                    workspace: f.0.to_string_lossy().into()
                },
                connection
            ),
            "{principal}"
        );
    }
    assert!(!enrollment_authorized(
        &f.0,
        &EnrollmentActor {
            principal: "owner".into(),
            workspace: "missing-relative-directory".into()
        },
        connection
    ));
}

#[test]
fn connection_enrollment_rechecks_disk_identity_revision_scope_and_revocation() {
    let f = Fixture::new();
    let grant = f.connection();
    f.save_connection(&grant);
    let scope = Scope::Connection(grant.clone());
    let actor = scope.actor(&f.0);
    let connection = grant.enrollment_connections[0];
    assert!(enrollment_authorized(&f.0, &actor, connection));
    assert!(!enrollment_authorized(&f.0, &actor, Uuid::new_v4()));
    for change in 0..6 {
        let mut changed = grant.clone();
        match change {
            0 => changed.revoked = true,
            1 => changed.revision += 1,
            2 => changed.principal_id = Uuid::new_v4(),
            3 => changed.expires_at_ms = 0,
            4 => changed.rights.clear(),
            _ => changed.workspaces[0].path = f.0.join("elsewhere"),
        }
        f.save_connection(&changed);
        assert!(
            !enrollment_authorized(&f.0, &actor, connection),
            "change {change}"
        );
        assert!(
            scope
                .check(&f.0, &f.0, ProcessRight::AccountEnroll)
                .is_err()
        );
    }
    f.save_connection(&grant);
    assert!(scope.check(&f.0, &f.0, ProcessRight::AccountEnroll).is_ok());
}

#[test]
fn session_scope_rejects_stale_snapshots_and_limits_accounts() {
    let f = Fixture::new();
    let grant = f.session();
    f.save_session(&grant);
    let scope = Scope::Session(grant.clone());
    let registry = Registry::new(f.0.join("accounts"));
    assert!(scope.check(&f.0, &f.0, ProcessRight::AccountUse).is_ok());
    assert!(scope.account_allowed(
        &registry,
        grant.accounts[0],
        grant.enrollment_connections[0],
        &f.0
    ));
    assert!(!scope.account_allowed(&registry, Uuid::new_v4(), Uuid::new_v4(), &f.0));
    assert!(Scope::Owner.account_allowed(&registry, Uuid::new_v4(), Uuid::new_v4(), &f.0));
    let actor = scope.actor(&f.0);
    assert!(enrollment_authorized(
        &f.0,
        &actor,
        grant.enrollment_connections[0]
    ));
    for change in 0..7 {
        let mut changed = grant.clone();
        match change {
            0 => changed.accounts.clear(),
            1 => changed.enrollment_connections.clear(),
            2 => changed.revision += 1,
            3 => changed.principal_id = Uuid::new_v4(),
            4 => changed.revoked = true,
            5 => changed.rights.clear(),
            _ => changed.workspace = f.0.join("other"),
        }
        f.save_session(&changed);
        assert!(
            scope.check(&f.0, &f.0, ProcessRight::AccountUse).is_err(),
            "change {change}"
        );
    }
}

#[test]
fn derived_session_requires_current_parent_and_never_delegates_enrollment() {
    let f = Fixture::new();
    let parent = f.session();
    f.save_session(&parent);
    let mut child = parent.clone();
    child.grant_id = Uuid::new_v4();
    child.parent_grant = Some(GrantBinding {
        grant_id: parent.grant_id,
        revision: parent.revision,
        principal_id: parent.principal_id,
    });
    child.rights = vec![ProcessRight::AccountUse];
    assert!(
        current_session_scope(&f.0, &child)
            .unwrap_err()
            .to_string()
            .contains("missing participant binding")
    );
    child.rights.push(ProcessRight::AccountEnroll);
    assert!(current_session_scope(&f.0, &child).is_err());
    child.rights.pop();
    child.accounts.push(Uuid::new_v4());
    assert!(current_session_scope(&f.0, &child).is_err());
    child.accounts.pop();
    let mut revoked = parent.clone();
    revoked.revoked = true;
    f.save_session(&revoked);
    assert!(current_session_scope(&f.0, &child).is_err());
}

#[test]
fn connection_derived_actor_uses_human_binding_and_checks_attenuation() {
    let f = Fixture::new();
    let parent = f.connection();
    f.save_connection(&parent);
    let mut child = f.session();
    child.principal_id = parent.principal_id;
    child.accounts = parent.accounts.clone();
    child.enrollment_connections = parent.enrollment_connections.clone();
    child.connection_binding = Some(GrantBinding {
        grant_id: parent.grant_id,
        principal_id: parent.principal_id,
        revision: parent.revision,
    });
    assert_eq!(
        Scope::Session(child.clone()).actor(&f.0),
        Scope::Connection(parent.clone()).actor(&f.0)
    );
    assert!(current_session_scope(&f.0, &child).is_ok());
    child.enrollment_connections.push(Uuid::new_v4());
    assert!(current_session_scope(&f.0, &child).is_err());
    child.enrollment_connections.pop();
    child.connection_binding.as_mut().unwrap().revision += 1;
    assert!(current_session_scope(&f.0, &child).is_err());
}

#[tokio::test]
async fn enrollment_worker_deduplicates_active_ids_without_starting_provider_work() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let id = Uuid::new_v4();
    let handle = tokio::spawn(std::future::pending::<()>());
    let task_id = handle.id();
    s.enrollment_workers.lock().await.insert(id, handle);
    s.enrollment_worker(id, Scope::Owner.actor(&f.0), None)
        .await;
    let mut workers = s.enrollment_workers.lock().await;
    assert_eq!(workers.len(), 1);
    let handle = workers.remove(&id).unwrap();
    assert_eq!(handle.id(), task_id);
    handle.abort();
    assert!(handle.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn enrollment_worker_enforces_total_recovery_capacity() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    for _ in 0..64 {
        s.enrollment_workers
            .lock()
            .await
            .insert(Uuid::new_v4(), tokio::spawn(std::future::pending::<()>()));
    }
    let refused = Uuid::new_v4();
    s.enrollment_worker(refused, Scope::Owner.actor(&f.0), None)
        .await;
    let handles: Vec<_> = {
        let mut workers = s.enrollment_workers.lock().await;
        assert_eq!(workers.len(), 64);
        assert!(!workers.contains_key(&refused));
        workers.drain().map(|(_, h)| h).collect()
    };
    for handle in handles {
        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn enrollment_worker_reaps_finished_slots_and_settles_unknown_attempt() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let old = Uuid::new_v4();
    let handle = tokio::spawn(async {});
    while !handle.is_finished() {
        tokio::task::yield_now().await;
    }
    s.enrollment_workers.lock().await.insert(old, handle);
    let id = Uuid::new_v4();
    s.enrollment_worker(id, Scope::Owner.actor(&f.0), None)
        .await;
    let handle = {
        let mut workers = s.enrollment_workers.lock().await;
        assert!(!workers.contains_key(&old));
        assert_eq!(workers.len(), 1);
        workers.remove(&id).unwrap()
    };
    handle.await.unwrap();
    assert!(s.devices.resume_candidates().unwrap().is_empty());
}

#[tokio::test]
async fn rejected_enrollment_start_worker_leaves_no_recoverable_attempt() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let id = Uuid::new_v4();
    let actor = Scope::Owner.actor(&f.0);
    s.enrollment_worker(
        id,
        actor.clone(),
        Some(EnrollmentRequest {
            command_id: Uuid::new_v4(),
            enrollment_id: id,
            connection_id: Uuid::new_v4(),
            alias: "offline".into(),
            label: "Offline".into(),
            actor,
        }),
    )
    .await;
    let handle = s.enrollment_workers.lock().await.remove(&id).unwrap();
    handle.await.unwrap();
    assert!(s.devices.resume_candidates().unwrap().is_empty());
}

#[tokio::test]
async fn empty_enrollment_recovery_is_idempotent_and_spawns_nothing() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    s.resume_enrollments().await.unwrap();
    s.resume_enrollments().await.unwrap();
    assert!(s.enrollment_workers.lock().await.is_empty());
}

#[tokio::test]
async fn scoped_enrollment_operations_recheck_revocation_before_device_access() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let grant = f.session();
    let mut revoked = grant.clone();
    revoked.revoked = true;
    f.save_session(&revoked);
    let id = Uuid::new_v4();
    let commands = [
        VesselCommand::PrivateAccountEnrollment {
            enrollment_id: id,
            workspace: f.0.clone(),
        },
        VesselCommand::CancelAccountEnrollment {
            command_id: Uuid::new_v4(),
            enrollment_id: id,
            workspace: f.0.clone(),
        },
        VesselCommand::EnrollAccount {
            command_id: Uuid::new_v4(),
            enrollment_id: id,
            workspace: f.0.clone(),
            connection_id: grant.enrollment_connections[0],
            alias: "offline".into(),
            label: "Offline".into(),
        },
        VesselCommand::ResolveAccountEnrollment {
            command_id: Uuid::new_v4(),
            enrollment_id: id,
            workspace: f.0.clone(),
            connection_id: grant.enrollment_connections[0],
            alias: "offline".into(),
            label: "Offline".into(),
        },
    ];
    for command in commands {
        assert!(
            s.host_accounts(command, Scope::Session(grant.clone()))
                .await
                .is_err()
        );
    }
    assert!(s.enrollment_workers.lock().await.is_empty());
    assert!(s.devices.resume_candidates().unwrap().is_empty());
}

#[tokio::test]
async fn settings_replay_freezes_snapshot_even_when_original_base_disappears() {
    // Exercise private snapshot preparation, not public account authorization.
    // The public StartSettings entry point requires a validated host account.
    async fn prepare(s: &Supervisor, request: VesselCommand) -> Result<Value> {
        let VesselCommand::StartSettings {
            command_id,
            session_id,
            workspace,
            config_path,
            settings,
            binding,
        } = request.clone()
        else {
            panic!("expected settings request");
        };
        s.start_settings(
            command_id,
            session_id,
            workspace,
            config_path,
            settings,
            binding,
            request,
        )
        .await
    }

    let f = Fixture::new();
    let s = f.supervisor().await;
    let mut r = f.registration();
    r.state = ProcessState::Stopped;
    let base = f.0.join("settings-base.json");
    let config = voyage_runtime::Config::default();
    let launch = voyage_runtime::launch_config::LaunchConfig::capture(&config, &f.0).unwrap();
    store::save_bounded(&base, &launch, 1024 * 1024).unwrap();
    let request = VesselCommand::StartSettings {
        command_id: r.command_id, session_id: r.session_id, workspace: f.0.clone(),
        config_path: Some(base.clone()),
        settings: serde_json::from_value(json!({"model":"offline-retained-model", "max_output_tokens":512, "reasoning_effort":null, "service_tier":null})).unwrap(),
        binding: None,
    };
    // This accountless base is valid only for the private preparation fixture.
    // Public admission must still reject it before recording any creation.
    let error = s
        .host_accounts(request.clone(), Scope::Owner)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "configuration has no account");
    assert!(
        super::super::database::creation_receipt(&f.0, r.command_id)
            .await
            .unwrap()
            .is_none()
    );
    // Simulate prior durable admission/creation to exercise snapshot preparation
    // and exact replay without consulting host accounts or starting a child.
    super::super::database::admit(&f.0, &r, serde_json::to_vec(&request).unwrap())
        .await
        .unwrap();
    let info = ProcessInfo::from(&r);
    super::super::database::settle_creation(&f.0, r.command_id, &info)
        .await
        .unwrap();
    // Even an exact settled receipt cannot bypass public account authorization.
    assert_eq!(
        s.host_accounts(request.clone(), Scope::Owner)
            .await
            .unwrap_err()
            .to_string(),
        "configuration has no account"
    );
    let response = prepare(&s, request.clone()).await.unwrap();
    assert_eq!(response["session_id"], r.session_id.to_string());
    assert_eq!(response["state"], "stopped");
    let retained =
        f.0.join("account-launch")
            .join(format!("{}-{}-settings.json", r.session_id, r.command_id));
    let bytes = std::fs::read(&retained).unwrap();
    let snapshot: voyage_runtime::launch_config::LaunchConfig =
        serde_json::from_slice(&bytes).unwrap();
    let config = snapshot.resolve(&f.0).unwrap();
    assert_eq!(config.model, "offline-retained-model");
    assert_eq!(config.max_tokens, 512);
    assert_eq!(config.reasoning_effort, None);
    assert_eq!(config.service_tier, None);
    std::fs::remove_file(base).unwrap();
    assert_eq!(prepare(&s, request.clone()).await.unwrap(), response);
    assert_eq!(std::fs::read(&retained).unwrap(), bytes);
    let mut changed = serde_json::to_value(&request).unwrap();
    changed["settings"]["model"] = json!("different");
    assert!(
        prepare(&s, serde_json::from_value(changed).unwrap())
            .await
            .is_err()
    );
    assert_eq!(std::fs::read(&retained).unwrap(), bytes);
    assert_eq!(
        super::super::database::registration(&f.0, r.session_id)
            .await
            .unwrap()
            .incarnation,
        r.incarnation
    );
}

#[test]
fn owner_connection_accounts_and_enrollment_are_dynamic_and_revocable() {
    let f = Fixture::new();
    let mut grant = f.connection();
    grant.full_access = true;
    grant.rights = ProcessRight::all();
    grant.accounts.clear();
    grant.enrollment_connections.clear();
    grant.workspaces.clear();
    f.save_connection(&grant);
    let scope = Scope::Connection(grant.clone());
    let workspace = f.0.join("later");
    crate::process::registry::private_directory(&workspace).unwrap();
    scope
        .check(&f.0, &workspace, ProcessRight::AccountUse)
        .unwrap();
    let registry = Registry::new(f.0.join("accounts"));
    let connection = Uuid::new_v4();
    assert!(scope.account_allowed(&registry, Uuid::new_v4(), connection, &workspace));
    assert!(scope.connection_allowed(connection));
    assert!(enrollment_authorized(&f.0, &scope.actor(&f.0), connection));
    grant.revoked = true;
    f.save_connection(&grant);
    assert!(
        scope
            .check(&f.0, &workspace, ProcessRight::AccountUse)
            .is_err()
    );
    assert!(!enrollment_authorized(&f.0, &scope.actor(&f.0), connection));
}
