//! Account/configuration work runs after an explicit OS identity drop. The root
//! supervisor retains grant/identity checks and never opens ordinary credentials.
use super::{accounts::Scope, database, service::Supervisor};
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Stdio, time::Duration};
use voyage_protocol::{
    execution_identity::ConfiguredExecutionIdentity,
    identity_helper::*,
    process::{ProcessRight, VesselCommand},
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

async fn current(
    scope: &Scope,
    root: &Path,
    workspace: &Path,
    identity: &ConfiguredExecutionIdentity,
    right: ProcessRight,
) -> Result<()> {
    scope.check(root, workspace, right)?;
    ensure!(
        selected(scope, root).await? == *identity,
        "account execution identity changed"
    );
    super::launch::validate_identity(identity)
}

impl Supervisor {
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
        ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
        scope.check(&self.directory, workspace, right)?;
        let identity = selected(scope, &self.directory).await?;
        current(scope, &self.directory, workspace, &identity, right).await?;
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
                    _ = tokio::time::sleep(Duration::from_millis(100)) => current(scope, &self.directory, workspace, &identity, right).await?,
                }
            };
            ensure!(serde_json::to_vec(&response)?.len() <= IDENTITY_HELPER_BYTES, "identity helper output exceeds bounds");
            ensure!(child.wait().await?.success(), "identity helper failed");
            current(scope, &self.directory, workspace, &identity, right).await?;
            match response {
                IdentityHelperResponse::Value {value} => Ok(value),
                _ => anyhow::bail!("identity account operation unavailable"),
            }
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
}
