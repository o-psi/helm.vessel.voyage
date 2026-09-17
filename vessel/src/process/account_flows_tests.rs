//! Coordinator account flows over an explicitly isolated registry. No default_host,
//! provider endpoint override, environment mutation, or credential reads.
use super::{accounts::Scope, database, registry, test_support::Fixture};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;
use voyage_protocol::{accounts::*, process::*};
use voyage_runtime::accounts::{Registry, device::DeviceService};

fn native(registry: &Registry) -> Uuid {
    registry.ensure_chatgpt_connection().unwrap().id
}

fn enrollment(
    workspace: &std::path::Path,
    connection_id: Uuid,
    command_id: Uuid,
    enrollment_id: Uuid,
    resolve: bool,
) -> VesselCommand {
    if resolve {
        VesselCommand::ResolveAccountEnrollment {
            command_id,
            enrollment_id,
            workspace: workspace.to_owned(),
            connection_id,
            alias: "offline-fenced".into(),
            label: "Offline fenced enrollment".into(),
        }
    } else {
        VesselCommand::EnrollAccount {
            command_id,
            enrollment_id,
            workspace: workspace.to_owned(),
            connection_id,
            alias: "offline-fenced".into(),
            label: "Offline fenced enrollment".into(),
        }
    }
}

async fn drain(s: &super::service::Supervisor) {
    let workers: Vec<_> = s
        .enrollment_workers
        .lock()
        .await
        .drain()
        .map(|(_, worker)| worker)
        .collect();
    for worker in workers {
        worker.await.unwrap();
    }
}

#[tokio::test]
async fn enrollment_resolution_late_start_private_status_cancel_and_restart() {
    let f = Fixture::new();
    let registry = Registry::new(f.0.join("accounts"));
    let connection = native(&registry);
    let mut s = f.supervisor().await;
    s.devices = DeviceService::new(registry.clone(), Arc::new(|_, _| true));
    let command = Uuid::new_v4();
    let id = Uuid::new_v4();
    let resolve = enrollment(&f.0, connection, command, id, true);
    let result = s
        .host_accounts(resolve.clone(), Scope::Owner)
        .await
        .unwrap();
    let status: EnrollmentStatus = serde_json::from_value(result.clone()).unwrap();
    assert_eq!(status.state, EnrollmentState::Cancelled);
    assert!(!status.effects_may_have_occurred);
    assert_eq!(status.enrollment_id, id);
    assert_eq!(status.account_id, None);
    assert_eq!(
        s.host_accounts(resolve.clone(), Scope::Owner)
            .await
            .unwrap(),
        result
    );
    let private_command = VesselCommand::PrivateAccountEnrollment {
        enrollment_id: id,
        workspace: f.0.clone(),
    };
    let private = s
        .host_accounts(private_command.clone(), Scope::Owner)
        .await
        .unwrap();
    assert_eq!(private["status"], result);
    assert!(private["user_code"].is_null());
    assert!(private["verification_uri"].is_null());
    assert!(private.get("failure").is_none());
    // A resolved original cannot reach DeviceService::provider, even when the
    // coordinator creates its ordinary enrollment worker for a late start.
    let late = s
        .host_accounts(
            enrollment(&f.0, connection, command, id, false),
            Scope::Owner,
        )
        .await
        .unwrap();
    assert_eq!(late, result);
    drain(&s).await;
    assert!(s.devices.resume_candidates().unwrap().is_empty());
    let cancel_id = Uuid::new_v4();
    let cancel = VesselCommand::CancelAccountEnrollment {
        command_id: cancel_id,
        enrollment_id: id,
        workspace: f.0.clone(),
    };
    for _ in 0..2 {
        assert_eq!(
            s.host_accounts(cancel.clone(), Scope::Owner).await.unwrap(),
            result
        );
    }
    // Private status is deliberately not journaled; neither resolution nor a
    // late enrollment invents a public Vessel command receipt.
    assert!(
        !registry::command_record(&f.0, command, &resolve, false)
            .await
            .unwrap()
    );
    assert!(
        !registry::command_record(&f.0, cancel_id, &cancel, false)
            .await
            .unwrap()
    );
    drop(s);
    let mut s = f.supervisor().await;
    s.devices = DeviceService::new(registry, Arc::new(|_, _| true));
    s.resume_enrollments().await.unwrap();
    assert!(s.enrollment_workers.lock().await.is_empty());
    assert_eq!(
        s.host_accounts(private_command, Scope::Owner)
            .await
            .unwrap(),
        private
    );
    assert_eq!(
        s.host_accounts(resolve, Scope::Owner).await.unwrap(),
        result
    );
}

#[tokio::test]
async fn enrollment_identity_conflicts_cannot_alias_cancel_or_change_actor() {
    let f = Fixture::new();
    let registry = Registry::new(f.0.join("accounts"));
    let connection = native(&registry);
    let mut s = f.supervisor().await;
    s.devices = DeviceService::new(registry, Arc::new(|_, _| true));
    let command = Uuid::new_v4();
    let id = Uuid::new_v4();
    let resolve = enrollment(&f.0, connection, command, id, true);
    let expected = s
        .host_accounts(resolve.clone(), Scope::Owner)
        .await
        .unwrap();
    for field in [
        "alias",
        "label",
        "connection_id",
        "command_id",
        "enrollment_id",
    ] {
        let mut value = serde_json::to_value(&resolve).unwrap();
        value[field] = if matches!(field, "alias" | "label") {
            json!("changed")
        } else {
            json!(Uuid::new_v4())
        };
        assert!(
            s.host_accounts(serde_json::from_value(value).unwrap(), Scope::Owner)
                .await
                .is_err(),
            "{field}"
        );
    }
    let cancel_id = Uuid::new_v4();
    s.host_accounts(
        VesselCommand::CancelAccountEnrollment {
            command_id: cancel_id,
            enrollment_id: id,
            workspace: f.0.clone(),
        },
        Scope::Owner,
    )
    .await
    .unwrap();
    assert!(
        s.host_accounts(
            enrollment(&f.0, connection, cancel_id, Uuid::new_v4(), true),
            Scope::Owner
        )
        .await
        .is_err()
    );
    let mut grant = f.connection();
    grant.enrollment_connections = vec![connection];
    f.save_connection(&grant);
    // Same host and workspace are not the same actor identity.
    assert!(
        s.host_accounts(resolve.clone(), Scope::Connection(grant.clone()))
            .await
            .is_err()
    );
    assert!(
        s.host_accounts(
            VesselCommand::PrivateAccountEnrollment {
                enrollment_id: id,
                workspace: f.0.clone()
            },
            Scope::Connection(grant)
        )
        .await
        .is_err()
    );
    assert_eq!(
        s.host_accounts(resolve, Scope::Owner).await.unwrap(),
        expected
    );
    assert!(s.enrollment_workers.lock().await.is_empty());
}

#[tokio::test]
async fn scoped_enrollment_is_reauthorized_after_revocation_and_disk_revision_change() {
    for session_scope in [false, true] {
        let f = Fixture::new();
        let registry = Registry::new(f.0.join("accounts"));
        let connection = native(&registry);
        let mut s = f.supervisor().await;
        s.devices = DeviceService::new(registry, Arc::new(|_, _| true));
        let mut human = f.connection();
        human.enrollment_connections = vec![connection];
        f.save_connection(&human);
        let mut session = f.session();
        session.enrollment_connections = vec![connection];
        f.save_session(&session);
        let scope = if session_scope {
            Scope::Session(session.clone())
        } else {
            Scope::Connection(human.clone())
        };
        let command = Uuid::new_v4();
        let id = Uuid::new_v4();
        let resolve = enrollment(&f.0, connection, command, id, true);
        assert_eq!(
            s.host_accounts(resolve.clone(), scope.clone())
                .await
                .unwrap()["state"],
            "cancelled"
        );
        let private = VesselCommand::PrivateAccountEnrollment {
            enrollment_id: id,
            workspace: f.0.clone(),
        };
        assert!(
            s.host_accounts(private.clone(), scope.clone())
                .await
                .is_ok()
        );
        if session_scope {
            session.revision += 1;
            f.save_session(&session);
        } else {
            human.revoked = true;
            f.save_connection(&human);
        }
        for request in [
            resolve,
            private,
            enrollment(&f.0, connection, command, id, false),
            VesselCommand::CancelAccountEnrollment {
                command_id: Uuid::new_v4(),
                enrollment_id: id,
                workspace: f.0.clone(),
            },
        ] {
            assert!(s.host_accounts(request, scope.clone()).await.is_err());
        }
        assert!(s.enrollment_workers.lock().await.is_empty());
    }
}

#[tokio::test]
async fn enrollment_authority_denial_precedes_worker_and_registry_effects() {
    let f = Fixture::new();
    let registry = Registry::new(f.0.join("accounts"));
    let connection = native(&registry);
    let mut s = f.supervisor().await;
    // Count authorization attempts: denied provider admission must still perform
    // the actual service authorization rather than a test-only short circuit.
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = checks.clone();
    s.devices = DeviceService::new(
        registry,
        Arc::new(move |_, _| {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            false
        }),
    );
    let command = Uuid::new_v4();
    let id = Uuid::new_v4();
    assert!(
        s.host_accounts(
            enrollment(&f.0, connection, command, id, true),
            Scope::Owner
        )
        .await
        .is_err()
    );
    assert!(
        s.host_accounts(
            enrollment(&f.0, connection, command, id, false),
            Scope::Owner
        )
        .await
        .is_err()
    );
    drain(&s).await;
    assert_eq!(checks.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert!(s.devices.resume_candidates().unwrap().is_empty());
    assert!(
        s.host_accounts(
            VesselCommand::PrivateAccountEnrollment {
                enrollment_id: id,
                workspace: f.0.clone()
            },
            Scope::Owner
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn worker_capacity_refuses_even_a_fenced_late_start_without_changing_status() {
    let f = Fixture::new();
    let registry = Registry::new(f.0.join("accounts"));
    let connection = native(&registry);
    let mut s = f.supervisor().await;
    s.devices = DeviceService::new(registry, Arc::new(|_, _| true));
    let command = Uuid::new_v4();
    let id = Uuid::new_v4();
    let resolve = enrollment(&f.0, connection, command, id, true);
    let expected = s
        .host_accounts(resolve.clone(), Scope::Owner)
        .await
        .unwrap();
    for _ in 0..64 {
        s.enrollment_workers
            .lock()
            .await
            .insert(Uuid::new_v4(), tokio::spawn(std::future::pending()));
    }
    let error = s
        .host_accounts(
            enrollment(&f.0, connection, command, id, false),
            Scope::Owner,
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("worker capacity"));
    assert_eq!(s.enrollment_workers.lock().await.len(), 64);
    let workers: Vec<_> = s
        .enrollment_workers
        .lock()
        .await
        .drain()
        .map(|(_, worker)| worker)
        .collect();
    for worker in workers {
        worker.abort();
        assert!(worker.await.unwrap_err().is_cancelled());
    }
    assert_eq!(
        s.host_accounts(resolve, Scope::Owner).await.unwrap(),
        expected
    );
}

fn binding() -> AccountBinding {
    AccountBinding {
        account_id: Uuid::new_v4(),
        connection_id: Uuid::new_v4(),
        connection_revision: 1,
        identity_generation: 1,
        transport: Transport::OpenaiResponses,
    }
}

#[tokio::test]
async fn account_start_resolution_fences_exact_binding_without_host_registry_access() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let command_id = Uuid::new_v4();
    let session_id = Uuid::new_v4();
    let account = binding();
    let resolve = VesselCommand::ResolveStartAccount {
        command_id,
        session_id,
        workspace: f.0.clone(),
        config_path: None,
        account: account.clone(),
        model: "offline-model".into(),
        reasoning_effort: Some("high".into()),
        service_tier: None,
    };
    for _ in 0..2 {
        assert_eq!(
            s.host_accounts(resolve.clone(), Scope::Owner)
                .await
                .unwrap()["status"],
            "not_admitted"
        );
    }
    for field in ["model", "reasoning_effort", "service_tier", "account"] {
        let mut changed = serde_json::to_value(&resolve).unwrap();
        changed[field] = if field == "account" {
            serde_json::to_value(binding()).unwrap()
        } else {
            json!("changed")
        };
        assert!(
            s.host_accounts(serde_json::from_value(changed).unwrap(), Scope::Owner)
                .await
                .is_err(),
            "{field}"
        );
    }
    assert!(s.registrations.lock().await.unwrap().is_empty());
    assert!(!registry::directory(&f.0, session_id).exists());
    let original = VesselCommand::StartAccount {
        command_id,
        session_id,
        workspace: f.0.clone(),
        config_path: None,
        account,
        model: "offline-model".into(),
        reasoning_effort: Some("high".into()),
        service_tier: None,
    };
    assert!(
        registry::command_record(&f.0, command_id, &original, false)
            .await
            .unwrap()
    );
    assert!(
        database::creation_receipt(&f.0, command_id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn start_settings_validates_scope_and_base_before_any_account_resolution() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    let command = |id, base, settings| VesselCommand::StartSettings {
        command_id: id,
        session_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        config_path: base,
        settings,
        binding: None,
    };
    for scope in [
        Scope::Session(f.session()),
        Scope::Connection(f.connection()),
    ] {
        assert!(
            s.host_accounts(command(Uuid::new_v4(), None, Default::default()), scope)
                .await
                .is_err()
        );
    }
    for base in [std::path::PathBuf::from("relative"), f.0.join("missing")] {
        let request = command(Uuid::new_v4(), Some(base), Default::default());
        assert!(s.host_accounts(request, Scope::Owner).await.is_err());
    }
    let malformed = f.0.join("malformed.json");
    super::access::store::save_bounded(&malformed, &json!({"version":999}), 1024).unwrap();
    assert!(
        s.host_accounts(
            command(Uuid::new_v4(), Some(malformed), Default::default()),
            Scope::Owner
        )
        .await
        .is_err()
    );
    let base = f.0.join("base.json");
    let config = voyage_runtime::Config::default();
    let launch = voyage_runtime::launch_config::LaunchConfig::capture(&config, &f.0).unwrap();
    super::access::store::save_bounded(&base, &launch, 1024 * 1024).unwrap();
    let settings = serde_json::from_value(json!({"command_timeout_secs":0})).unwrap();
    let request = command(Uuid::new_v4(), Some(base), settings);
    assert!(s.host_accounts(request, Scope::Owner).await.is_err());
    assert!(s.registrations.lock().await.unwrap().is_empty());
}

#[tokio::test]
async fn account_command_scope_and_identity_validation_is_side_effect_free() {
    let f = Fixture::new();
    let s = f.supervisor().await;
    for nil_command in [false, true] {
        let command_id = if nil_command {
            Uuid::nil()
        } else {
            Uuid::new_v4()
        };
        let session_id = if nil_command {
            Uuid::new_v4()
        } else {
            Uuid::nil()
        };
        let request = VesselCommand::StartAccount {
            command_id,
            session_id,
            workspace: f.0.clone(),
            config_path: None,
            account: binding(),
            model: "offline".into(),
            reasoning_effort: None,
            service_tier: None,
        };
        assert!(s.host_accounts(request, Scope::Owner).await.is_err());
    }
    let grant = f.connection();
    f.save_connection(&grant);
    let scope = Scope::Connection(grant);
    let request = VesselCommand::StartAccount {
        command_id: Uuid::new_v4(),
        session_id: Uuid::new_v4(),
        workspace: f.0.clone(),
        config_path: Some(f.0.join("never-read")),
        account: binding(),
        model: "offline".into(),
        reasoning_effort: None,
        service_tier: None,
    };
    assert!(s.host_accounts(request, scope).await.is_err());
    assert!(
        s.host_accounts(
            VesselCommand::DiscoverModels {
                workspace: f.0.clone(),
                configuration: json!({})
            },
            Scope::Owner
        )
        .await
        .is_err()
    );
    assert!(s.registrations.lock().await.unwrap().is_empty());
}
