//! Account/configuration work runs after an explicit OS identity drop. The root
//! supervisor retains grant/identity checks and never opens ordinary credentials.
use super::{accounts::Scope, database, service::Supervisor};
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Stdio, time::Duration};
use voyage_protocol::{
    execution_identity::ConfiguredExecutionIdentity,
    identity_helper::*,
    process::{GrantBinding, ProcessGrant, ProcessRegistration, ProcessRight, VesselCommand},
};

fn projection(scope: &Scope, root: &Path, workspace: &Path) -> Result<IdentityAccountScope> {
    let can_use = scope
        .check(root, workspace, ProcessRight::AccountUse)
        .is_ok();
    let can_enroll = scope
        .check(root, workspace, ProcessRight::AccountEnroll)
        .is_ok();
    ensure!(can_use || can_enroll, "identity account permission denied");
    let (full_access, account_ids, enrollment_connections) = match scope {
        Scope::Owner => (true, vec![], vec![]),
        Scope::Connection(grant) => (
            grant.full_access,
            grant.accounts.clone(),
            grant.enrollment_connections.clone(),
        ),
        Scope::Session(grant) => (
            grant.full_access,
            grant.accounts.clone(),
            grant.enrollment_connections.clone(),
        ),
    };
    Ok(IdentityAccountScope {
        full_access,
        can_use,
        can_enroll,
        account_ids,
        enrollment_connections,
        actor: scope.actor(workspace),
    })
}

pub(super) async fn selected(scope: &Scope, root: &Path) -> Result<ConfiguredExecutionIdentity> {
    if let Scope::Session(grant) = scope {
        let registration = database::registration(root, grant.session_id).await?;
        return database::bound_observer_identity(root, &registration).await;
    }
    let identity = super::default_execution::protected_default(root)?;
    database::store_identity(root, &identity).await?;
    ensure!(
        database::configured_identity(root, &identity.identity).await? == identity,
        "ordinary account identity changed"
    );
    Ok(identity)
}

#[derive(Clone, Copy)]
enum Selection<'a> {
    Default,
    Bound(&'a ProcessRegistration),
    ReviewTarget(&'a ConfiguredExecutionIdentity),
}

async fn current(
    scope: &Scope,
    root: &Path,
    workspace: &Path,
    identity: &ConfiguredExecutionIdentity,
    right: ProcessRight,
    selection: Selection<'_>,
) -> Result<()> {
    scope.check(root, workspace, right)?;
    ensure!(
        selected_for(scope, root, selection).await? == *identity,
        "account execution identity changed"
    );
    super::launch::validate_identity(identity)
}

async fn selected_for(
    scope: &Scope,
    root: &Path,
    selection: Selection<'_>,
) -> Result<ConfiguredExecutionIdentity> {
    if let Selection::Bound(registration) = selection {
        let current = database::registration(root, registration.session_id).await?;
        ensure!(
            current.incarnation == registration.incarnation
                && current.peer_uids == registration.peer_uids
                && current.command_id == registration.command_id
                && current.token == registration.token
                && current.workspace == registration.workspace
                && current.config_path == registration.config_path,
            "account runtime incarnation changed"
        );
        database::bound_observer_identity(root, &current).await
    } else if let Selection::ReviewTarget(identity) = selection {
        let Scope::Connection(grant) = scope else {
            anyhow::bail!("review target requires an enrolled human owner");
        };
        database::execution_reviews::authority(root, grant).await?;
        let latest = database::configured_identity(root, &identity.identity).await?;
        ensure!(
            &latest == identity
                && latest.enabled
                && latest.uid != 0
                && latest.authority
                    == voyage_protocol::execution_identity::AuthorityClass::Ordinary,
            "ordinary review target changed"
        );
        super::launch::validate_identity(&latest)?;
        Ok(latest)
    } else {
        selected(scope, root).await
    }
}

impl Supervisor {
    /// Freeze a target ordinary configuration for an explicit enrolled owner.
    /// The supervisor receives only a path and typed facts, never its contents.
    pub(super) async fn prepare_identity_config_for_review(
        &self,
        grant: &voyage_protocol::process::ConnectionGrant,
        session: uuid::Uuid,
        command: uuid::Uuid,
        workspace: &Path,
        target: &ConfiguredExecutionIdentity,
    ) -> Result<(std::path::PathBuf, IdentityConfigFacts)> {
        use sha2::{Digest, Sha256};
        ensure!(
            !session.is_nil() && !command.is_nil(),
            "invalid identity review operation"
        );
        let authority = database::execution_reviews::authority(&self.directory, grant).await?;
        let scope = Scope::Connection(grant.clone());
        scope.check(&self.directory, workspace, ProcessRight::AccountUse)?;
        let selected =
            selected_for(&scope, &self.directory, Selection::ReviewTarget(target)).await?;
        let request = serde_json::to_vec(
            &serde_json::json!({"schema":1,"purpose":"ordinary_execution_review","command_id":command,"session_id":session,"workspace":workspace,"actor":scope.actor(workspace),"authority_revision":authority,"target":selected}),
        )?;
        ensure!(
            request.len() <= 65_536,
            "identity review capture intent exceeds bounds"
        );
        let control = voyage_storage::protected_linux::RootDirectory::open(&self.directory)?
            .create_child("identity-review-configs".as_ref())?;
        let name = format!("{command}.json");
        match control.read(name.as_ref(), 65_536) {
            Ok(prior) => ensure!(prior == request, "identity review capture intent changed"),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                ensure!(
                    std::fs::read_dir(self.directory.join("identity-review-configs"))?
                        .take(4097)
                        .count()
                        < 4096,
                    "identity review capture capacity reached"
                );
                control.publish_new(name.as_ref(), &request, 65_536)?;
            }
            Err(error) => return Err(error),
        }
        let mut digest = Sha256::new();
        digest.update(b"voyage/review-config-staging/v1\0");
        digest.update(&request);
        let digest: [u8; 32] = digest.finalize().into();
        let stage = uuid::Uuid::from_bytes(digest[..16].try_into()?);
        // The stage is not a session and has no admitted owner. A distinct path
        // lets a different target UID prepare its own file without touching the
        // old voyage's runtime directory before its cleanup/ownership review.
        let directory = super::runtime_storage::provision_bound_directory(
            &self.directory,
            stage,
            selected.uid,
            selected.gid,
        )?;
        let path = directory.join("launch.json");
        let operation = IdentityHelperOperation::CaptureLaunch {
            scope: self.identity_account_scope(&scope, workspace)?,
            directory,
            request_digest: format!("{:x}", Sha256::digest(&request)),
            base_config_path: None,
            account: None,
            settings: Default::default(),
        };
        let captured = self
            .run_identity_helper(
                &scope,
                workspace,
                ProcessRight::AccountUse,
                operation,
                Selection::ReviewTarget(target),
            )
            .await?;
        let IdentityHelperResponse::Value { value: captured } = captured else {
            anyhow::bail!("target configuration capture unavailable");
        };
        let captured_path: std::path::PathBuf =
            serde_json::from_value(captured["config_path"].clone())?;
        ensure!(captured_path == path, "target configuration path changed");
        let observed = self
            .run_identity_helper(
                &scope,
                workspace,
                ProcessRight::AccountUse,
                IdentityHelperOperation::ReviewConfig {
                    config_path: path.clone(),
                },
                Selection::ReviewTarget(target),
            )
            .await?;
        let IdentityHelperResponse::Facts { facts } = observed else {
            anyhow::bail!("target configuration review unavailable");
        };
        ensure!(
            Some(facts.config_digest.as_str()) == captured["config_digest"].as_str()
                && facts.uid == selected.uid
                && facts.gid == selected.gid
                && database::execution_reviews::authority(&self.directory, grant).await?
                    == authority,
            "target review authority or configuration changed"
        );
        Ok((path, facts))
    }

    pub(super) async fn host_identity_accounts(
        &self,
        command: VesselCommand,
        scope: Scope,
    ) -> Result<serde_json::Value> {
        if matches!(
            command,
            VesselCommand::StartAccount { .. }
                | VesselCommand::StartSettings { .. }
                | VesselCommand::ResolveStartAccount { .. }
        ) {
            return self.start_identity_request(command, scope).await;
        }
        if matches!(
            command,
            VesselCommand::Profiles { .. }
                | VesselCommand::SaveProfile { .. }
                | VesselCommand::DeleteProfile { .. }
                | VesselCommand::SetDefaultProfile { .. }
        ) {
            return self.identity_execution_profiles(command, scope).await;
        }
        let (workspace, right) = match &command {
            VesselCommand::Accounts { workspace, .. } => {
                let right = if scope
                    .check(&self.directory, workspace, ProcessRight::AccountUse)
                    .is_ok()
                {
                    ProcessRight::AccountUse
                } else {
                    ProcessRight::AccountEnroll
                };
                (workspace.clone(), right)
            }
            VesselCommand::AccountDefaults { workspace }
            | VesselCommand::AccountModels { workspace, .. } => {
                (workspace.clone(), ProcessRight::AccountUse)
            }
            _ => anyhow::bail!("identity-scoped account operation unavailable"),
        };
        let projection = self.identity_account_scope(&scope, &workspace)?;
        let operation = match command {
            VesselCommand::Accounts { transport, .. } => IdentityHelperOperation::Accounts {
                scope: projection,
                transport,
            },
            VesselCommand::AccountDefaults { .. } => {
                let catalogue = self.identity_profile_catalogue(&workspace, &scope).await?;
                let profile = catalogue
                    .default_profile_id
                    .and_then(|id| catalogue.profiles.into_iter().find(|p| p.id == id));
                IdentityHelperOperation::Defaults {
                    scope: projection,
                    profile,
                    profiles_revision: catalogue.revision,
                }
            }
            VesselCommand::AccountModels { account, .. } => IdentityHelperOperation::Models {
                scope: projection,
                account,
            },
            _ => anyhow::bail!("identity-scoped account operation unavailable"),
        };
        self.identity_account_helper(&scope, &workspace, right, operation)
            .await
    }

    pub(super) async fn identity_account_helper(
        &self,
        scope: &Scope,
        workspace: &Path,
        right: ProcessRight,
        operation: IdentityHelperOperation,
    ) -> Result<serde_json::Value> {
        self.identity_account_helper_for(scope, workspace, right, operation, None)
            .await
    }

    async fn identity_account_helper_for(
        &self,
        scope: &Scope,
        workspace: &Path,
        right: ProcessRight,
        operation: IdentityHelperOperation,
        registration: Option<&ProcessRegistration>,
    ) -> Result<serde_json::Value> {
        let selection = registration.map_or(Selection::Default, Selection::Bound);
        match self
            .run_identity_helper(scope, workspace, right, operation, selection)
            .await?
        {
            IdentityHelperResponse::Value { value } => Ok(value),
            _ => anyhow::bail!("identity account operation unavailable"),
        }
    }

    async fn run_identity_helper(
        &self,
        scope: &Scope,
        workspace: &Path,
        right: ProcessRight,
        operation: IdentityHelperOperation,
        selection: Selection<'_>,
    ) -> Result<IdentityHelperResponse> {
        ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
        scope.check(&self.directory, workspace, right)?;
        let identity = selected_for(scope, &self.directory, selection).await?;
        current(
            scope,
            &self.directory,
            workspace,
            &identity,
            right,
            selection,
        )
        .await?;
        super::launch::protected_binary(&self.binary)?;
        let request = IdentityHelperRequest {
            schema: IDENTITY_HELPER_SCHEMA,
            workspace: workspace.to_owned(),
            operation,
        };
        ensure!(
            serde_json::to_vec(&request)?.len() <= IDENTITY_HELPER_BYTES,
            "identity account request exceeds bounds"
        );
        let mut command = tokio::process::Command::new(&self.binary);
        command
            .arg("identity-helper")
            .current_dir(&identity.home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        super::launch::configure_identity(command.as_std_mut(), &identity)?;
        // The ordinary helper uses the exact selected user's local namespace.
        // Administrator helpers are handled by the explicit reviewed path.
        ensure!(
            identity.uid != 0,
            "administrator accounts require owner-reviewed identity selection"
        );
        unsafe {
            command.pre_exec(|| {
                for (resource, limit) in
                    [(libc::RLIMIT_AS, 512 * 1024 * 1024), (libc::RLIMIT_CPU, 5)]
                {
                    let limits = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(resource, &limits) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            voyage_protocol::process::write_frame(&mut child.stdin.take().context("identity helper input missing")?, &request).await?;
            let mut output = child.stdout.take().context("identity helper output missing")?;
            let read = voyage_protocol::process::read_frame::<IdentityHelperResponse>(&mut output);
            tokio::pin!(read);
            let response = loop {
                tokio::select! {
                    response = &mut read => break response?,
                    _ = tokio::time::sleep(Duration::from_millis(100)) => current(scope, &self.directory, workspace, &identity, right, selection).await?,
                }
            };
            ensure!(serde_json::to_vec(&response)?.len() <= IDENTITY_HELPER_BYTES, "identity helper output exceeds bounds");
            ensure!(child.wait().await?.success(), "identity helper failed");
            current(scope, &self.directory, workspace, &identity, right, selection).await?;
            Ok(response)
        }).await;
        match result {
            Ok(Ok(value)) => Ok(value),
            failure => {
                let _ = child.kill().await;
                match failure {
                    Ok(Err(error)) => Err(error),
                    _ => anyhow::bail!("identity account helper deadline elapsed"),
                }
            }
        }
    }

    pub(super) fn identity_account_scope(
        &self,
        scope: &Scope,
        workspace: &Path,
    ) -> Result<IdentityAccountScope> {
        projection(scope, &self.directory, workspace)
    }

    pub(super) async fn validate_identity_account_binding(
        &self,
        registration: &ProcessRegistration,
        authorization: Option<&GrantBinding>,
        account: &voyage_protocol::accounts::AccountBinding,
        observe_intent: bool,
    ) -> Result<()> {
        let scope = if let Some(binding) = authorization {
            let grant: ProcessGrant = super::access::store::load(
                &super::access::store::grant_path(&self.directory, binding.grant_id),
            )?;
            ensure!(
                grant.grant_id == binding.grant_id
                    && grant.principal_id == binding.principal_id
                    && grant.revision == binding.revision
                    && grant.session_id == registration.session_id
                    && grant.workspace == registration.workspace,
                "bound account authority mismatch"
            );
            Scope::Session(grant)
        } else {
            Scope::Owner
        };
        let projection = self.identity_account_scope(&scope, &registration.workspace)?;
        let operation = if observe_intent {
            IdentityHelperOperation::ObserveAccountIntent {
                scope: projection,
                account: account.clone(),
            }
        } else {
            IdentityHelperOperation::ValidateAccount {
                scope: projection,
                account: account.clone(),
            }
        };
        self.identity_account_helper_for(
            &scope,
            &registration.workspace,
            ProcessRight::AccountUse,
            operation,
            Some(registration),
        )
        .await?;
        Ok(())
    }
}
