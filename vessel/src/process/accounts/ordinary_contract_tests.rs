//! Default-host API contracts use a child process with entirely private HOME/XDG roots.
use super::super::super::test_support::Fixture;
use super::*;

#[test]
fn isolated_ordinary_account_contracts() {
    const MODE: &str = "VESSEL_ORDINARY_ACCOUNT_CONTRACT_CHILD";
    if std::env::var_os(MODE).is_none() {
        let root = tempfile::tempdir().unwrap();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact","process::accounts::tests::ordinary_contract_tests::isolated_ordinary_account_contracts","--nocapture"])
            .env(MODE,"1").stdin(std::process::Stdio::null());
        for name in [
            "HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_CACHE_HOME",
        ] {
            let path = root.path().join(name);
            std::fs::create_dir(&path).unwrap();
            command.env(name, path);
        }
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated account contract deadline elapsed");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        return;
    }
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let f = Fixture::new();
            let mut supervisor = f.supervisor().await;
            supervisor.devices = device_service(f.0.clone()).unwrap();
            let registry = Registry::default_host().unwrap();
            let first_connection = registry
                .add_connection(
                    "First synthetic API".into(),
                    "https://api.openai.com/v1".into(),
                    vec![Transport::OpenaiResponses, Transport::OpenaiChat],
                )
                .unwrap();
            let second_connection = registry
                .add_connection(
                    "Second synthetic API".into(),
                    "https://api.anthropic.com".into(),
                    vec![Transport::Anthropic],
                )
                .unwrap();
            let first = registry
                .add_api(
                    first_connection.id,
                    "first".into(),
                    "First".into(),
                    voyage_runtime::accounts::ApiKeyInput::Stored("synthetic-only-first".into()),
                )
                .unwrap();
            let second = registry
                .add_api(
                    second_connection.id,
                    "second".into(),
                    "Second".into(),
                    voyage_runtime::accounts::ApiKeyInput::Stored("synthetic-only-second".into()),
                )
                .unwrap();
            let binding = registry
                .freeze(first.id, Transport::OpenaiResponses)
                .unwrap();
            let list = |transport| VesselCommand::Accounts {
                workspace: f.0.clone(),
                transport,
            };
            let owner = supervisor
                .host_accounts(list(None), Scope::Owner)
                .await
                .unwrap();
            assert_eq!(owner["accounts"].as_array().unwrap().len(), 2);
            assert_eq!(owner["can_set_default"], true);
            assert!(!owner.to_string().contains("synthetic-only"));
            let mut connection = f.connection();
            connection.accounts = vec![first.id];
            let enrollment_connection = registry.ensure_chatgpt_connection().unwrap();
            connection.enrollment_connections = vec![first_connection.id, enrollment_connection.id];
            f.save_connection(&connection);
            let scope = Scope::Connection(connection.clone());
            let restricted = supervisor
                .host_accounts(list(None), scope.clone())
                .await
                .unwrap();
            assert_eq!(restricted["accounts"].as_array().unwrap().len(), 1);
            assert_eq!(restricted["accounts"][0]["id"], json!(first.id));
            assert_eq!(restricted["can_set_default"], false);
            assert!(!restricted.to_string().contains(&second.id.to_string()));
            let filtered = supervisor
                .host_accounts(list(Some(Transport::Anthropic)), scope.clone())
                .await
                .unwrap();
            assert!(filtered["accounts"].as_array().unwrap().is_empty());
            assert!(filtered["connections"].as_array().unwrap().is_empty());
            let command_id = Uuid::new_v4();
            let set_default = VesselCommand::AccountSetDefault {
                command_id,
                workspace: f.0.clone(),
                account: binding.clone(),
                expected_revision: 0,
            };
            assert!(
                supervisor
                    .host_accounts(set_default.clone(), scope.clone())
                    .await
                    .is_err()
            );
            let selected = supervisor
                .host_accounts(set_default.clone(), Scope::Owner)
                .await
                .unwrap();
            assert_eq!(selected["default_account"]["account_id"], json!(first.id));
            assert_eq!(
                supervisor
                    .host_accounts(set_default, Scope::Owner)
                    .await
                    .unwrap(),
                selected
            );
            let stale = VesselCommand::AccountSetDefault {
                command_id: Uuid::new_v4(),
                workspace: f.0.clone(),
                account: binding.clone(),
                expected_revision: 0,
            };
            assert!(supervisor.host_accounts(stale, Scope::Owner).await.is_err());
            let usage = VesselCommand::AccountUsage {
                workspace: f.0.clone(),
                account: binding.clone(),
                refresh: false,
            };
            let cached = supervisor
                .host_accounts(usage.clone(), scope.clone())
                .await
                .unwrap();
            assert!(!cached.to_string().contains("synthetic-only"));
            let mut session = f.session();
            session.accounts = vec![first.id];
            f.save_session(&session);
            assert!(
                supervisor
                    .host_accounts(usage, Scope::Session(session))
                    .await
                    .is_err()
            );
            // API transport refresh remains metadata-only: no OAuth refresh/inference is requested.
            supervisor
                .host_accounts(
                    VesselCommand::AccountUsage {
                        workspace: f.0.clone(),
                        account: binding.clone(),
                        refresh: true,
                    },
                    scope.clone(),
                )
                .await
                .unwrap();
            let defaults = supervisor
                .host_accounts(
                    VesselCommand::AccountDefaults {
                        workspace: f.0.clone(),
                    },
                    scope.clone(),
                )
                .await
                .unwrap();
            assert_eq!(defaults["account"]["account_id"], json!(first.id));
            assert!(!defaults.to_string().contains("synthetic-only"));
            let enrollment_id = Uuid::new_v4();
            let request = VesselCommand::ResolveAccountEnrollment {
                command_id: Uuid::new_v4(),
                enrollment_id,
                workspace: f.0.clone(),
                connection_id: enrollment_connection.id,
                alias: "new".into(),
                label: "New".into(),
            };
            let resolved = supervisor
                .host_accounts(request, scope.clone())
                .await
                .unwrap();
            assert!(!resolved.to_string().contains("synthetic-only"));
            let private = supervisor
                .host_accounts(
                    VesselCommand::PrivateAccountEnrollment {
                        enrollment_id,
                        workspace: f.0.clone(),
                    },
                    scope.clone(),
                )
                .await
                .unwrap();
            assert!(private["user_code"].is_null());
            assert!(private["verification_uri"].is_null());
            let cancellation = VesselCommand::CancelAccountEnrollment {
                command_id: Uuid::new_v4(),
                enrollment_id,
                workspace: f.0.clone(),
            };
            let cancelled = supervisor
                .host_accounts(cancellation.clone(), scope.clone())
                .await
                .unwrap();
            assert_eq!(
                supervisor
                    .host_accounts(cancellation, scope.clone())
                    .await
                    .unwrap(),
                cancelled
            );
            for command in [
                VesselCommand::ResolveAccountEnrollment {
                    command_id: Uuid::new_v4(),
                    enrollment_id: Uuid::new_v4(),
                    workspace: f.0.clone(),
                    connection_id: second_connection.id,
                    alias: "new".into(),
                    label: "New".into(),
                },
                VesselCommand::PrivateAccountEnrollment {
                    enrollment_id: Uuid::new_v4(),
                    workspace: f.0.clone(),
                },
                VesselCommand::CancelAccountEnrollment {
                    command_id: Uuid::new_v4(),
                    enrollment_id: Uuid::new_v4(),
                    workspace: f.0.clone(),
                },
            ] {
                assert!(
                    supervisor
                        .host_accounts(command, scope.clone())
                        .await
                        .is_err()
                );
            }
            // Creation envelope validation refuses before retained launch files,
            // default capture or inference. No failed mutation is repeated.
            for resolve in [false, true] {
                for (command_id, session_id) in
                    [(Uuid::nil(), Uuid::new_v4()), (Uuid::new_v4(), Uuid::nil())]
                {
                    let command = if resolve {
                        VesselCommand::ResolveStartAccount {
                            command_id,
                            session_id,
                            workspace: f.0.clone(),
                            config_path: None,
                            account: binding.clone(),
                            model: "fixture".into(),
                            reasoning_effort: None,
                            service_tier: None,
                        }
                    } else {
                        VesselCommand::StartAccount {
                            command_id,
                            session_id,
                            workspace: f.0.clone(),
                            config_path: None,
                            account: binding.clone(),
                            model: "fixture".into(),
                            reasoning_effort: None,
                            service_tier: None,
                        }
                    };
                    assert!(
                        supervisor
                            .host_accounts(command, Scope::Owner)
                            .await
                            .unwrap_err()
                            .to_string()
                            .contains("nil creation identity")
                    );
                }
            }
            connection.rights.push(ProcessRight::Create);
            f.save_connection(&connection);
            let scoped = VesselCommand::StartAccount {
                command_id: Uuid::new_v4(),
                session_id: Uuid::new_v4(),
                workspace: f.0.clone(),
                config_path: Some(f.0.join("private-host-config")),
                account: binding.clone(),
                model: "fixture".into(),
                reasoning_effort: None,
                service_tier: None,
            };
            assert!(
                supervisor
                    .host_accounts(scoped, Scope::Connection(connection.clone()))
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("owner-local authority")
            );
            assert!(!f.0.join("account-launch").exists());
            assert!(
                super::super::super::database::catalogue(&f.0)
                    .await
                    .unwrap()
                    .is_empty()
            );
            let revision = registry.list(|_| true).unwrap().0;
            connection.revoked = true;
            f.save_connection(&connection);
            assert!(supervisor.host_accounts(list(None), scope).await.is_err());
            assert_eq!(registry.list(|_| true).unwrap().0, revision);
            assert!(supervisor.enrollment_workers.lock().await.is_empty());
        });
}
