//! Ordinary UserFile authority -> current policy -> actual filesystem tools.
//! No Broker/root-bound scope, runtime process, provider or human credentials.
use super::*;
use crate::tools::ToolRegistry;
use serde_json::json;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use voyage_protocol::process::{
    ApprovedWorkspace, ConnectionGrant, ParticipantBinding, ParticipantGrantBinding, ProcessState,
    RuntimeCommand, TerminalOperation,
};

struct Fixture {
    _root: tempfile::TempDir,
    root: PathBuf,
    workspace: PathBuf,
    directory: PathBuf,
    path: PathBuf,
    grant: ProcessGrant,
    registration: ProcessRegistration,
    actor: LocalActor,
}
fn save(path: &Path, value: &impl serde::Serialize) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
fn binding(grant: &ProcessGrant) -> GrantBinding {
    GrantBinding {
        grant_id: grant.grant_id,
        principal_id: grant.principal_id,
        revision: grant.revision,
    }
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let workspace = root.join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(
            workspace.join("input"),
            "owned readable canonical 世界".as_bytes(),
        )
        .unwrap();
        let grant = ProcessGrant {
            full_access: false,
            grant_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            workspace: workspace.clone(),
            revision: 1,
            rights: ProcessRight::all(),
            accounts: vec![],
            enrollment_connections: vec![],
            expires_at_ms: u64::MAX,
            revoked: false,
            token_hash: "synthetic-not-a-credential".into(),
            parent_grant: None,
            connection_binding: None,
            participant_binding: None,
        };
        let directory = root.join("sessions").join(grant.session_id.to_string());
        std::fs::create_dir_all(&directory).unwrap();
        let path = root
            .join("access/grants")
            .join(format!("{}.json", grant.grant_id));
        save(&path, &grant);
        let registration = ProcessRegistration {
            executable: None,
            protocol: 1,
            session_id: grant.session_id,
            incarnation: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            restart_from: None,
            initialize: None,
            config_path: None,
            token: "synthetic-registration-not-transport".into(),
            peer_uids: None,
            workspace: workspace.clone(),
            state: ProcessState::Live,
            name: None,
        };
        let actor = LocalActor {
            installation_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
        };
        Self {
            _root: temporary,
            root,
            workspace,
            directory,
            path,
            grant,
            registration,
            actor,
        }
    }
    fn request(&self, command: RuntimeCommand) -> RuntimeRequest {
        RuntimeRequest {
            protocol: 1,
            session_id: self.grant.session_id,
            incarnation: self.registration.incarnation,
            token: self.registration.token.clone(),
            authorization: Some(binding(&self.grant)),
            scope_authority: None,
            command,
        }
    }
    fn authorize(&self, command: RuntimeCommand) -> Result<Authorization> {
        authorize_parts(
            self.actor,
            &self.registration,
            &self.request(command),
            &self.directory,
        )
    }
    fn context(&self, authorization: &Authorization, read_only: bool) -> crate::tools::ToolContext {
        let mut context = crate::tools::reliability_tests::context(&self.workspace);
        let config = crate::Config {
            access: Some(if read_only {
                crate::config::AccessMode::ReadOnly
            } else {
                crate::config::AccessMode::Unrestricted
            }),
            ..Default::default()
        };
        context.policy = Arc::new(
            crate::policy::Policy::new(&config, self.workspace.clone())
                .unwrap()
                .with_execution_authority(authorization.authority.clone().unwrap()),
        );
        context
    }
    fn tools() -> ToolRegistry {
        let mut tools = ToolRegistry::default();
        tools.register(crate::tools::ReadFile);
        tools.register(crate::tools::WriteFile);
        tools
    }
    async fn no_effect(&self, context: &crate::tools::ToolContext) {
        let tools = Self::tools();
        let before = std::fs::read(self.workspace.join("input")).unwrap();
        assert!(
            tools
                .execute("read_file", json!({"path":"input"}), context)
                .await
                .is_err()
        );
        assert!(
            tools
                .execute(
                    "write_file",
                    json!({"path":"late-output","content":"must not publish"}),
                    context
                )
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(self.workspace.join("input")).unwrap(), before);
        assert!(!self.workspace.join("late-output").exists());
    }
    fn connection(&mut self, owner: bool) -> (PathBuf, ConnectionGrant) {
        self.grant.full_access = owner;
        let parent = ConnectionGrant {
            full_access: owner,
            schema_version: 1,
            grant_id: Uuid::new_v4(),
            principal_id: self.grant.principal_id,
            vessel_id: Uuid::new_v4(),
            revision: 1,
            rights: ProcessRight::all(),
            accounts: vec![],
            enrollment_connections: vec![],
            expires_at_ms: u64::MAX,
            revoked: false,
            token_hash: "synthetic-parent".into(),
            workspaces: if owner {
                vec![]
            } else {
                vec![ApprovedWorkspace {
                    id: Uuid::new_v4(),
                    name: "owned workspace".into(),
                    path: self.workspace.clone(),
                    provider_ready: None,
                }]
            },
        };
        self.grant.connection_binding = Some(GrantBinding {
            grant_id: parent.grant_id,
            principal_id: parent.principal_id,
            revision: parent.revision,
        });
        let path = self
            .root
            .join("access/connections")
            .join(format!("{}.json", parent.grant_id));
        save(&path, &parent);
        save(
            &self.root.join("identity/key.json"),
            &json!({"vessel_id":parent.vessel_id}),
        );
        save(&self.path, &self.grant);
        (path, parent)
    }
}
#[tokio::test]
async fn admitted_userfile_authority_checks_current_identity_before_real_read_and_write() {
    let f = Fixture::new();
    let authorized = f.authorize(RuntimeCommand::Snapshot).unwrap();
    assert_eq!(authorized.actor.installation_id, f.actor.installation_id);
    assert_eq!(authorized.actor.principal_id, f.grant.principal_id);
    let context = f.context(&authorized, false);
    let tools = Fixture::tools();
    assert!(
        tools
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .unwrap()
            .contains("owned readable canonical 世界")
    );
    tools
        .execute(
            "write_file",
            json!({"path":"first-output","content":"accepted once"}),
            &context,
        )
        .await
        .unwrap();
    for case in 0..5 {
        let mut changed = f.grant.clone();
        match case {
            0 => changed.revoked = true,
            1 => changed.expires_at_ms = 0,
            2 => changed.revision += 1,
            3 => changed.principal_id = Uuid::new_v4(),
            _ => changed.rights.retain(|r| *r != ProcessRight::Execute),
        };
        save(&f.path, &changed);
        f.no_effect(&context).await;
        assert_eq!(
            std::fs::read(f.workspace.join("first-output")).unwrap(),
            b"accepted once"
        );
    }
}
#[tokio::test]
async fn held_connection_derived_policy_refuses_changed_parent_without_replacing_workspace_or_result()
 {
    let mut f = Fixture::new();
    let (path, parent) = f.connection(false);
    let authorized = f.authorize(RuntimeCommand::Snapshot).unwrap();
    let context = f.context(&authorized, false);
    assert!(
        Fixture::tools()
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .is_ok()
    );
    for case in 0..7 {
        let mut changed = parent.clone();
        match case {
            0 => changed.revoked = true,
            1 => changed.revision += 1,
            2 => changed.expires_at_ms = 0,
            3 => changed.principal_id = Uuid::new_v4(),
            4 => changed.vessel_id = Uuid::new_v4(),
            5 => changed.workspaces.clear(),
            _ => changed.rights.retain(|r| *r != ProcessRight::Execute),
        };
        save(&path, &changed);
        f.no_effect(&context).await;
        assert!(
            authorize_parts(
                f.actor,
                &f.registration,
                &f.request(RuntimeCommand::Snapshot),
                &f.directory
            )
            .is_err()
        );
    }
    save(&path, &parent);
    assert!(
        Fixture::tools()
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn participant_parent_and_accepted_binding_remain_current_at_tool_dispatch_without_new_assignment()
 {
    let mut f = Fixture::new();
    let mut parent = f.grant.clone();
    parent.session_id = Uuid::new_v4();
    parent.grant_id = Uuid::new_v4();
    let parent_path = f
        .root
        .join("access/grants")
        .join(format!("{}.json", parent.grant_id));
    save(&parent_path, &parent);
    let accepted = ParticipantBinding {
        binding_id: Uuid::new_v4(),
        revision: 1,
        parent_vessel_id: Uuid::new_v4(),
        parent_session_id: parent.session_id,
        principal_id: f.grant.principal_id,
        workspace: f.workspace.clone(),
        config_path: None,
        max_context_bytes: 4096,
        max_assignments: 1,
        expires_at_ms: u64::MAX,
        revoked: false,
        cancel_existing: false,
    };
    let accepted_path = f
        .root
        .join("participants/bindings")
        .join(format!("{}.json", accepted.binding_id));
    save(&accepted_path, &accepted);
    f.grant.parent_grant = Some(binding(&parent));
    f.grant.participant_binding = Some(ParticipantGrantBinding {
        binding_id: accepted.binding_id,
        revision: accepted.revision,
    });
    save(&f.path, &f.grant);
    let authorization = f.authorize(RuntimeCommand::Snapshot).unwrap();
    let context = f.context(&authorization, false);
    assert!(
        Fixture::tools()
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .is_ok()
    );
    for case in 0..4 {
        let mut changed = accepted.clone();
        match case {
            0 => changed.cancel_existing = true,
            1 => changed.expires_at_ms = 0,
            2 => changed.principal_id = Uuid::new_v4(),
            _ => changed.parent_session_id = Uuid::new_v4(),
        };
        save(&accepted_path, &changed);
        f.no_effect(&context).await;
    }
    let mut advanced = accepted.clone();
    advanced.revision += 1;
    save(&accepted_path, &advanced);
    assert!(
        Fixture::tools()
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .is_ok()
    );
    parent.rights.retain(|r| *r != ProcessRight::Execute);
    save(&parent_path, &parent);
    f.no_effect(&context).await;
}
#[tokio::test]
async fn scoped_workspace_observation_uses_workspace_read_without_granting_execute_or_write() {
    let mut f = Fixture::new();
    f.grant.rights = vec![ProcessRight::WorkspaceRead];
    save(&f.path, &f.grant);
    let authorization = f
        .authorize(RuntimeCommand::WorkspaceFile {
            path: "input".into(),
        })
        .unwrap();
    let context = f.context(&authorization, true);
    assert!(
        Fixture::tools()
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .unwrap()
            .contains("owned readable canonical 世界")
    );
    assert!(
        Fixture::tools()
            .execute(
                "write_file",
                json!({"path":"late-output","content":"forbidden"}),
                &context
            )
            .await
            .is_err()
    );
    assert!(!f.workspace.join("late-output").exists());
    assert!(f.authorize(RuntimeCommand::Snapshot).is_err());
    f.grant.rights.clear();
    save(&f.path, &f.grant);
    f.no_effect(&context).await;
}
#[tokio::test]
async fn grant_storage_replacement_cannot_turn_held_policy_into_cached_authority() {
    let f = Fixture::new();
    let authorized = f.authorize(RuntimeCommand::Snapshot).unwrap();
    let context = f.context(&authorized, false);
    let retained = std::fs::read(&f.path).unwrap();
    std::fs::remove_file(&f.path).unwrap();
    f.no_effect(&context).await;
    save(&f.path, &f.grant);
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o644)).unwrap();
    f.no_effect(&context).await;
    std::fs::set_permissions(&f.path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let alias = f.root.join("alias-grant");
    std::fs::hard_link(&f.path, &alias).unwrap();
    f.no_effect(&context).await;
    std::fs::remove_file(&alias).unwrap();
    std::fs::remove_file(&f.path).unwrap();
    std::os::unix::fs::symlink(&alias, &f.path).unwrap();
    f.no_effect(&context).await;
    std::fs::remove_file(&f.path).unwrap();
    save(&f.path, &f.grant);
    assert_eq!(std::fs::read(&f.path).unwrap(), retained);
    assert!(
        Fixture::tools()
            .execute("read_file", json!({"path":"input"}), &context)
            .await
            .is_ok()
    );
}
#[test]
fn private_terminal_and_participant_cancellation_require_all_admission_rights_before_dispatch() {
    let mut f = Fixture::new();
    let terminal = RuntimeCommand::Terminal {
        run_id: Uuid::new_v4(),
        terminal_id: Uuid::new_v4(),
        operation: TerminalOperation::Snapshot,
    };
    f.authorize(terminal.clone()).unwrap();
    for right in [ProcessRight::Terminal, ProcessRight::Execute] {
        let mut changed = f.grant.clone();
        changed.rights.retain(|r| *r != right);
        save(&f.path, &changed);
        assert!(f.authorize(terminal.clone()).is_err());
    }
    save(&f.path, &f.grant);
    let assignment = RuntimeCommand::AssignmentObserve {
        run_id: Uuid::new_v4(),
        assignment_id: Uuid::new_v4(),
        participant: "owned".into(),
        cancel: true,
    };
    f.authorize(assignment.clone()).unwrap();
    f.grant.rights.retain(|r| *r != ProcessRight::Cancel);
    save(&f.path, &f.grant);
    assert!(f.authorize(assignment).is_err());
    assert!(!f.workspace.join("late-output").exists());
}
#[test]
fn owner_goal_fencing_requires_current_parent_and_history_execute_cancel_without_scope_upgrade() {
    let mut f = Fixture::new();
    let command = RuntimeCommand::GoalReconcile {
        offset: 0,
        limit: 1,
        fence_children: true,
    };
    assert!(f.authorize(command.clone()).is_err());
    let (parent_path, parent) = f.connection(true);
    assert!(f.authorize(command.clone()).unwrap().owner_connection);
    for right in [
        ProcessRight::History,
        ProcessRight::Execute,
        ProcessRight::Cancel,
    ] {
        let mut changed = f.grant.clone();
        changed.rights.retain(|r| *r != right);
        save(&f.path, &changed);
        assert!(f.authorize(command.clone()).is_err());
    }
    save(&f.path, &f.grant);
    std::fs::remove_file(&parent_path).unwrap();
    assert!(f.authorize(command).is_err());
    save(&parent_path, &parent);
    assert!(!f.workspace.join("late-output").exists());
}
#[test]
fn ordinary_local_owner_and_wrong_workspace_paths_do_not_acquire_ambient_userfile_or_broker_scope()
{
    let f = Fixture::new();
    let mut request = f.request(RuntimeCommand::Snapshot);
    request.authorization = None;
    let local = authorize_parts(f.actor, &f.registration, &request, &f.directory).unwrap();
    assert!(local.authority.is_none() && local.scope_source.is_none() && local.grant.is_none());
    assert_eq!(local.actor, f.actor);
    let mut registration = f.registration.clone();
    registration.workspace = f.root.join("different");
    assert!(
        authorize_parts(
            f.actor,
            &registration,
            &f.request(RuntimeCommand::Snapshot),
            &f.directory
        )
        .is_err()
    );
    assert!(
        authorize_parts(
            f.actor,
            &f.registration,
            &f.request(RuntimeCommand::Snapshot),
            &f.root
                .join("not-sessions")
                .join(f.grant.session_id.to_string())
        )
        .is_err()
    );
    assert_eq!(
        format!(
            "{:?}",
            f.authorize(RuntimeCommand::Snapshot)
                .unwrap()
                .scope_source
                .unwrap()
        ),
        "AuthoritySource([private])"
    );
}

const ACCOUNT_ROOT: &str = "VOYAGE_USERFILE_ACCOUNT_ROOT";
#[test]
fn owned_account_child() {
    let Some(root) = std::env::var_os(ACCOUNT_ROOT) else {
        return;
    };
    let root = PathBuf::from(root);
    assert!(crate::config::default_data_dir().starts_with(root.join("data")));
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let mut f = Fixture::new();
        let registry = crate::accounts::Registry::default_host().unwrap();
        let endpoint = "http://127.0.0.1:9/v1";
        let connection = registry
            .add_connection(
                "owned no-network".into(),
                endpoint.into(),
                vec![voyage_protocol::accounts::Transport::OpenaiResponses],
            )
            .unwrap();
        let account = registry
            .add_api(
                connection.id,
                "owned-synthetic".into(),
                "owned fixture".into(),
                crate::accounts::ApiKeyInput::Stored(
                    "SYNTHETIC-NOT-A-REAL-PROVIDER-CREDENTIAL".into(),
                ),
            )
            .unwrap();
        let bound = voyage_protocol::accounts::AccountBinding {
            account_id: account.id,
            connection_id: connection.id,
            identity_generation: account.identity_generation,
            connection_revision: connection.revision,
            transport: voyage_protocol::accounts::Transport::OpenaiResponses,
        };
        f.grant.accounts.push(account.id);
        save(&f.path, &f.grant);
        let config = crate::Config {
            account: Some(bound.clone()),
            base_url: Some(endpoint.into()),
            access: Some(crate::config::AccessMode::Unrestricted),
            ..Default::default()
        };
        let mut authorization = f.authorize(RuntimeCommand::Snapshot).unwrap();
        bind_config(&f.registration, &mut authorization, &config).unwrap();
        let context = f.context(&authorization, false);
        assert!(
            Fixture::tools()
                .execute("read_file", json!({"path":"input"}), &context)
                .await
                .is_ok()
        );
        for case in 0..2 {
            let mut changed = f.grant.clone();
            if case == 0 {
                changed.accounts.clear();
            } else {
                changed.rights.retain(|r| *r != ProcessRight::AccountUse);
            }
            save(&f.path, &changed);
            f.no_effect(&context).await;
        }
        save(&f.path, &f.grant);
        assert!(
            registry.resolve_api_key(&bound).unwrap() == "SYNTHETIC-NOT-A-REAL-PROVIDER-CREDENTIAL"
        );
        registry.logout(account.id, true).unwrap();
        f.no_effect(&context).await;
        assert!(registry.validate_binding(&bound).is_err());
        std::fs::write(
            root.join("complete"),
            b"account-authority-qualified-offline",
        )
        .unwrap();
    });
}
#[test]
fn frozen_named_account_authority_refuses_scope_removal_and_real_registry_tombstone_before_tools() {
    let root = tempfile::tempdir().unwrap();
    let log = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(root.path().join("child.log"))
        .unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .env_clear()
        .args([
            "--exact",
            "server::authorization::owned_userfile_journeys::owned_account_child",
            "--nocapture",
        ])
        .env(ACCOUNT_ROOT, root.path())
        .env("HOME", root.path())
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_STATE_HOME", root.path().join("state"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log);
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let mut child = command.spawn().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "owned account child failed: {}",
                std::fs::read_to_string(root.path().join("child.log")).unwrap()
            );
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("owned account child exceeded deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        std::fs::read(root.path().join("complete")).unwrap(),
        b"account-authority-qualified-offline"
    );
}
