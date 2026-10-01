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

pub(super) fn projection(
    scope: &Scope,
    root: &Path,
    workspace: &Path,
) -> Result<IdentityAccountScope> {
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
pub(super) enum Selection<'a> {
    Default,
    InitializationDefault(&'a ConfiguredExecutionIdentity),
    FrozenDefault(&'a ConfiguredExecutionIdentity),
    Bound(&'a ProcessRegistration),
    ReviewTarget(&'a ConfiguredExecutionIdentity),
}
fn bound_current(registration: &ProcessRegistration, current: &ProcessRegistration) -> Result<()> {
    ensure!(
        current.incarnation == registration.incarnation
            && current.peer_uids == registration.peer_uids
            && current.command_id == registration.command_id
            && current.token == registration.token
            && current.workspace == registration.workspace
            && current.config_path == registration.config_path,
        "account runtime incarnation changed"
    );
    Ok(())
}
fn account_scope_current(
    grant: &ProcessGrant,
    binding: &GrantBinding,
    registration: &ProcessRegistration,
) -> Result<()> {
    ensure!(
        grant.grant_id == binding.grant_id
            && grant.principal_id == binding.principal_id
            && grant.revision == binding.revision
            && grant.session_id == registration.session_id
            && grant.workspace == registration.workspace,
        "bound account authority mismatch"
    );
    Ok(())
}
fn helper_value(response: IdentityHelperResponse) -> Result<serde_json::Value> {
    match response {
        IdentityHelperResponse::Value { value } => Ok(value),
        _ => anyhow::bail!("identity account operation unavailable"),
    }
}
fn request_bound(request: &IdentityHelperRequest) -> Result<()> {
    ensure!(
        serde_json::to_vec(request)?.len() <= IDENTITY_HELPER_BYTES,
        "identity account request exceeds bounds"
    );
    Ok(())
}
fn response_bound(response: &IdentityHelperResponse) -> Result<()> {
    ensure!(
        serde_json::to_vec(response)?.len() <= IDENTITY_HELPER_BYTES,
        "identity helper output exceeds bounds"
    );
    Ok(())
}
fn callback_actor_current(
    scope: &Scope,
    workspace: &Path,
    check: &IdentityAuthorityRequest,
) -> bool {
    check.actor == scope.actor(workspace)
}
// This metadata decision is reached only AFTER current scope/identity/OS checks.
// It is not a substitute for those checks and confers no authority by itself.
fn callback_permission(
    scope: &Scope,
    check: &IdentityAuthorityRequest,
    projection: Option<&IdentityAccountScope>,
) -> bool {
    match check.right {
        IdentityAuthorityRight::Recover => false,
        IdentityAuthorityRight::Enroll => {
            check.account_id.is_none() && scope.connection_allowed(check.connection_id)
        }
        IdentityAuthorityRight::Use => check.account_id.is_some_and(|id| {
            projection.is_some_and(|projection| {
                projection.full_access
                    || projection.account_ids.contains(&id)
                    || (projection.can_enroll && scope.connection_allowed(check.connection_id))
            })
        }),
    }
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
    if let Selection::InitializationDefault(identity) = selection {
        let current = super::default_execution::protected_default(root)?;
        ensure!(
            &current == identity
                && database::configured_identity(root, &identity.identity).await? == current,
            "initialization ordinary destination changed"
        );
        Ok(current)
    } else if let Selection::Bound(registration) = selection {
        let current = database::registration(root, registration.session_id).await?;
        bound_current(registration, &current)?;
        database::bound_observer_identity(root, &current).await
    } else if let Selection::FrozenDefault(identity) = selection {
        let current = selected(scope, root).await?;
        ensure!(
            &current == identity,
            "enrollment identity selection changed"
        );
        Ok(current)
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
        if let VesselCommand::EnrollAccount {
            command_id,
            enrollment_id,
            workspace,
            connection_id,
            alias,
            label,
        } = &command
        {
            let request = voyage_protocol::accounts::EnrollmentRequest {
                command_id: *command_id,
                enrollment_id: *enrollment_id,
                connection_id: *connection_id,
                alias: alias.clone(),
                label: label.clone(),
                actor: scope.actor(workspace),
            };
            return self
                .start_identity_enrollment(scope, workspace.clone(), request)
                .await;
        }
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
            VesselCommand::AccountUsage { workspace, .. }
            | VesselCommand::AccountSetDefault { workspace, .. } => {
                (workspace.clone(), ProcessRight::AccountUse)
            }
            VesselCommand::ResolveAccountEnrollment { workspace, .. }
            | VesselCommand::CancelAccountEnrollment { workspace, .. }
            | VesselCommand::PrivateAccountEnrollment { workspace, .. } => {
                (workspace.clone(), ProcessRight::AccountEnroll)
            }
            _ => anyhow::bail!("identity-scoped account operation unavailable"),
        };
        let projection = self.identity_account_scope(&scope, &workspace)?;
        static USAGE_REFRESH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let refresh_requested =
            matches!(&command, VesselCommand::AccountUsage { refresh: true, .. });
        let refresh_guard = if refresh_requested {
            USAGE_REFRESH.try_lock().ok()
        } else {
            None
        };
        let refresh_unavailable = refresh_requested && refresh_guard.is_none();
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
            VesselCommand::AccountUsage {
                account, refresh, ..
            } => {
                ensure!(
                    !matches!(scope, Scope::Session(_)),
                    "usage observations require a human connection"
                );
                IdentityHelperOperation::Usage {
                    scope: projection,
                    account,
                    refresh: refresh && !refresh_unavailable,
                }
            }
            VesselCommand::AccountSetDefault {
                command_id,
                account,
                expected_revision,
                ..
            } => {
                ensure!(
                    matches!(scope, Scope::Owner),
                    "only host owner may change default account"
                );
                IdentityHelperOperation::SetDefault {
                    scope: projection,
                    command_id,
                    account,
                    expected_revision,
                }
            }
            VesselCommand::ResolveAccountEnrollment {
                command_id,
                enrollment_id,
                connection_id,
                alias,
                label,
                ..
            } => {
                ensure!(
                    scope.connection_allowed(connection_id),
                    "enrollment connection denied"
                );
                IdentityHelperOperation::Enrollment {
                    scope: projection,
                    operation: IdentityEnrollmentOperation::Resolve {
                        request: voyage_protocol::accounts::EnrollmentRequest {
                            command_id,
                            enrollment_id,
                            connection_id,
                            alias,
                            label,
                            actor: scope.actor(&workspace),
                        },
                    },
                }
            }
            VesselCommand::CancelAccountEnrollment {
                command_id,
                enrollment_id,
                ..
            } => IdentityHelperOperation::Enrollment {
                scope: projection,
                operation: IdentityEnrollmentOperation::Cancel {
                    command_id,
                    enrollment_id,
                },
            },
            VesselCommand::PrivateAccountEnrollment { enrollment_id, .. } => {
                IdentityHelperOperation::Enrollment {
                    scope: projection,
                    operation: IdentityEnrollmentOperation::Status { enrollment_id },
                }
            }
            _ => anyhow::bail!("identity-scoped account operation unavailable"),
        };
        let mut value = self
            .identity_account_helper(&scope, &workspace, right, operation)
            .await?;
        if refresh_unavailable {
            value["refresh_status"] = serde_json::to_value(
                voyage_protocol::accounts::AccountUsageRefreshStatus::Unavailable,
            )?;
        }
        drop(refresh_guard);
        Ok(value)
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
        helper_value(
            self.run_identity_helper(scope, workspace, right, operation, selection)
                .await?,
        )
    }

    async fn run_identity_helper(
        &self,
        scope: &Scope,
        workspace: &Path,
        right: ProcessRight,
        operation: IdentityHelperOperation,
        selection: Selection<'_>,
    ) -> Result<IdentityHelperResponse> {
        run_owned_helper(
            &self.directory,
            &self.binary,
            scope,
            workspace,
            right,
            operation,
            selection,
        )
        .await
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
            account_scope_current(&grant, binding, registration)?;
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

/// The owning supervisor task may retain this runner after a socket waiter leaves.
pub(super) async fn run_owned_helper(
    root: &Path,
    binary: &Path,
    scope: &Scope,
    workspace: &Path,
    right: ProcessRight,
    operation: IdentityHelperOperation,
    selection: Selection<'_>,
) -> Result<IdentityHelperResponse> {
    ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
    scope.check(root, workspace, right)?;
    let identity = selected_for(scope, root, selection).await?;
    current(scope, root, workspace, &identity, right, selection).await?;
    super::launch::protected_binary(binary)?;
    let current_authority = matches!(
        &operation,
        IdentityHelperOperation::Enrollment { .. }
            | IdentityHelperOperation::Usage { .. }
            | IdentityHelperOperation::SetDefault { .. }
    );
    let request = IdentityHelperRequest {
        schema: IDENTITY_HELPER_SCHEMA,
        workspace: workspace.to_owned(),
        operation,
    };
    request_bound(&request)?;
    let mut command = tokio::process::Command::new(binary);
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
    let (mut authority, descriptor) = if current_authority {
        let (stream, descriptor) = super::identity_authority::attach(&mut command)?;
        (Some(stream), Some(descriptor))
    } else {
        (None, None)
    };
    unsafe {
        command.pre_exec(|| {
            for (resource, limit) in [(libc::RLIMIT_AS, 512 * 1024 * 1024), (libc::RLIMIT_CPU, 5)] {
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
    drop(descriptor);
    let result = tokio::time::timeout(Duration::from_secs(if current_authority {55} else {30}), async {
        voyage_protocol::process::write_frame(&mut child.stdin.take().context("identity helper input missing")?, &request).await?;
        let mut output = child.stdout.take().context("identity helper output missing")?;
        let read = voyage_protocol::process::read_frame::<IdentityHelperResponse>(&mut output);
        tokio::pin!(read);
        let response = loop {
            tokio::select! {
                biased;
                response = &mut read => break response?,
                check = async {if let Some(stream)=&mut authority {super::identity_authority::read(stream).await} else {std::future::pending().await}} => {
                    match check {
                        Ok(check)=>{
                            let right=match check.right {IdentityAuthorityRight::Use=>ProcessRight::AccountUse,IdentityAuthorityRight::Enroll=>ProcessRight::AccountEnroll,IdentityAuthorityRight::Recover=>{super::identity_authority::reply(authority.as_mut().context("authority channel unavailable")?,false).await?;continue;}};
                            let mut allowed=callback_actor_current(scope,workspace,&check)
                                &&current(scope,root,workspace,&identity,right,selection).await.is_ok();
                            if allowed {
                                let projection=if matches!(check.right,IdentityAuthorityRight::Use)&&check.account_id.is_some() {Some(projection(scope,root,workspace)?)} else {None};
                                allowed=callback_permission(scope,&check,projection.as_ref());
                            }
                            super::identity_authority::reply(authority.as_mut().context("authority channel unavailable")?,allowed).await?;
                        },
                        Err(error) if error.downcast_ref::<std::io::Error>().is_some_and(|e|e.kind()==std::io::ErrorKind::UnexpectedEof)=>authority=None,
                        Err(_)=>anyhow::bail!("identity authority channel refused"),
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(100)) => current(scope, root, workspace, &identity, right, selection).await?,
            }
        };
        response_bound(&response)?;
        ensure!(child.wait().await?.success(), "identity helper failed");
        current(scope, root, workspace, &identity, right, selection).await?;
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

#[cfg(test)]
#[path = "identity_accounts_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "identity_accounts_test_fixtures.rs"]
pub(in crate::process) mod test_fixtures;
