//! Host account operations. Only safe intents enter generic command records.
use super::{access::store, registry, service::Supervisor};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;
use voyage_protocol::{accounts::*, process::*};
use voyage_runtime::accounts::{Registry, device::DeviceService};

#[derive(Clone)]
pub(super) enum Scope {
    Owner,
    Connection(ConnectionGrant),
    Session(ProcessGrant),
}
impl Scope {
    pub(super) fn actor(&self, workspace: &Path) -> EnrollmentActor {
        let principal = match self {
            Self::Owner => "owner".into(),
            Self::Connection(g) => {
                format!("grant:{}:{}:{}", g.grant_id, g.principal_id, g.revision)
            }
            Self::Session(g) => format!(
                "grant:{}:{}:{}",
                g.connection_binding
                    .as_ref()
                    .map_or(g.grant_id, |b| b.grant_id),
                g.principal_id,
                g.connection_binding
                    .as_ref()
                    .map_or(g.revision, |b| b.revision)
            ),
        };
        EnrollmentActor {
            principal,
            workspace: workspace.to_string_lossy().into_owned(),
        }
    }
    fn check(&self, root: &Path, workspace: &Path, right: ProcessRight) -> Result<()> {
        ensure!(
            workspace.is_absolute()
                && workspace.is_dir()
                && std::fs::canonicalize(workspace)? == workspace,
            "workspace must be canonical"
        );
        match self {
            Self::Owner => Ok(()),
            Self::Connection(g) => {
                store::current_connection(root, g)?;
                ensure!(
                    g.rights.contains(&right) && g.workspaces.iter().any(|w| w.path == workspace),
                    "account workspace permission denied"
                );
                Ok(())
            }
            Self::Session(g) => {
                let latest: ProcessGrant = store::load(&store::grant_path(root, g.grant_id))?;
                current_session_scope(root, &latest)?;
                ensure!(
                    latest.grant_id == g.grant_id
                        && latest.accounts == g.accounts
                        && latest.enrollment_connections == g.enrollment_connections
                        && latest.rights == g.rights
                        && latest.revision == g.revision
                        && latest.principal_id == g.principal_id
                        && latest.workspace == workspace
                        && latest.rights.contains(&right),
                    "account session permission denied"
                );
                Ok(())
            }
        }
    }
    fn connection_allowed(&self, id: Uuid) -> bool {
        match self {
            Self::Owner => true,
            Self::Connection(g) => g.enrollment_connections.contains(&id),
            Self::Session(g) => g.enrollment_connections.contains(&id),
        }
    }
    fn account_allowed(
        &self,
        registry: &Registry,
        id: Uuid,
        connection: Uuid,
        workspace: &Path,
    ) -> bool {
        let (ids, rights) = match self {
            Self::Owner => return true,
            Self::Connection(g) => (&g.accounts, &g.rights),
            Self::Session(g) => (&g.accounts, &g.rights),
        };
        ids.contains(&id)
            || (rights.contains(&ProcessRight::AccountEnroll)
                && self.connection_allowed(connection)
                && registry.enrollment_actor(id).ok().flatten().as_ref()
                    == Some(&self.actor(workspace)))
    }
    pub(super) fn observe_account_intent(
        &self,
        root: &Path,
        workspace: &Path,
        account: &AccountBinding,
    ) -> Result<()> {
        self.check(root, workspace, ProcessRight::AccountUse)?;
        let registry = Registry::default_host()?;
        ensure!(
            self.account_allowed(
                &registry,
                account.account_id,
                account.connection_id,
                workspace
            ),
            "account intent observation denied"
        );
        Ok(())
    }
    pub(super) fn use_account(
        &self,
        root: &Path,
        workspace: &Path,
        account: &AccountBinding,
    ) -> Result<()> {
        self.check(root, workspace, ProcessRight::AccountUse)?;
        let registry = Registry::default_host()?;
        registry.validate_binding(account)?;
        ensure!(
            self.account_allowed(
                &registry,
                account.account_id,
                account.connection_id,
                workspace
            ),
            "account use denied"
        );
        Ok(())
    }
}

/// Host checks mirror Voyage's inherited-grant checks; a revoked parent cannot
/// continue enrollment merely because a derived session record is still present.
fn current_session_scope(root: &Path, grant: &ProcessGrant) -> Result<()> {
    store::current(grant)?;
    if let Some(binding) = &grant.connection_binding {
        let parent: ConnectionGrant = store::load(&store::connection_path(root, binding.grant_id))?;
        store::current_connection(root, &parent)?;
        ensure!(
            parent.principal_id == grant.principal_id
                && binding.principal_id == grant.principal_id
                && binding.revision == parent.revision
                && grant.expires_at_ms <= parent.expires_at_ms
                && parent.workspaces.iter().any(|w| w.path == grant.workspace)
                && grant.rights.iter().all(|r| parent.rights.contains(r))
                && grant.accounts.iter().all(|id| parent.accounts.contains(id))
                && grant
                    .enrollment_connections
                    .iter()
                    .all(|id| parent.enrollment_connections.contains(id)),
            "account connection authority changed"
        );
    }
    if let Some(binding) = &grant.parent_grant {
        let parent: ProcessGrant = store::load(&store::grant_path(root, binding.grant_id))?;
        ensure!(
            parent.parent_grant.is_none()
                && parent.grant_id == binding.grant_id
                && parent.principal_id == binding.principal_id
                && parent.principal_id == grant.principal_id
                && parent.revision == binding.revision,
            "account parent authority changed"
        );
        current_session_scope(root, &parent)?;
        // Enrollment is a human-only operation, never delegated participant authority.
        ensure!(
            !grant.rights.contains(&ProcessRight::AccountEnroll)
                && grant.rights.iter().all(|r| parent.rights.contains(r))
                && grant.accounts.iter().all(|id| parent.accounts.contains(id)),
            "account participant scope denied"
        );
        let accepted = grant
            .participant_binding
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("missing participant binding"))?;
        let participant: ParticipantBinding = store::load(
            &root
                .join("participants/bindings")
                .join(format!("{}.json", accepted.binding_id)),
        )?;
        ensure!(
            participant.binding_id == accepted.binding_id
                && participant.revision >= accepted.revision
                && !participant.cancel_existing
                && participant.expires_at_ms > store::now()?
                && participant.parent_session_id == parent.session_id
                && participant.principal_id == grant.principal_id,
            "account participant authority withdrawn"
        );
    }
    Ok(())
}

/// Publication callback re-reads current host-private grant metadata. It never
/// reenters the account registry while the device publication transaction is held.
fn enrollment_authorized(root: &Path, actor: &EnrollmentActor, connection: Uuid) -> bool {
    let workspace = Path::new(&actor.workspace);
    // The private host CLI shares this durable service; supervise its retained
    // attempts after restart without inventing a remote principal or workspace.
    if actor.principal == "execution-host-owner" && actor.workspace == "execution-host" {
        return true;
    }
    if actor.principal == "owner" {
        return workspace.is_absolute() && workspace.is_dir();
    }
    let parts: Vec<_> = actor.principal.split(':').collect();
    if parts.len() != 4 || parts[0] != "grant" {
        return false;
    }
    let (Ok(id), Ok(principal), Ok(revision)) = (
        Uuid::parse_str(parts[1]),
        Uuid::parse_str(parts[2]),
        parts[3].parse::<u64>(),
    ) else {
        return false;
    };
    let scope = if store::connection_path(root, id).exists() {
        let Ok(g) = store::load::<ConnectionGrant>(&store::connection_path(root, id)) else {
            return false;
        };
        if g.principal_id != principal || g.revision != revision {
            return false;
        }
        Scope::Connection(g)
    } else {
        let Ok(g) = store::load::<ProcessGrant>(&store::grant_path(root, id)) else {
            return false;
        };
        if g.principal_id != principal || g.revision != revision {
            return false;
        }
        Scope::Session(g)
    };
    scope
        .check(root, workspace, ProcessRight::AccountEnroll)
        .is_ok()
        && scope.connection_allowed(connection)
}

pub(super) fn device_service(root: PathBuf) -> Result<DeviceService> {
    let registry = Registry::default_host()?;
    registry.ensure_chatgpt_connection()?;
    Ok(DeviceService::new(
        registry,
        Arc::new(move |actor, connection| enrollment_authorized(&root, actor, connection)),
    ))
}

impl Supervisor {
    pub(super) async fn resume_enrollments(&self) -> Result<()> {
        for (id, actor) in self.devices.resume_candidates()? {
            self.enrollment_worker(id, actor, None).await;
        }
        Ok(())
    }
    async fn enrollment_worker(
        &self,
        id: Uuid,
        actor: EnrollmentActor,
        request: Option<EnrollmentRequest>,
    ) {
        let mut workers = self.enrollment_workers.lock().await;
        workers.retain(|_, handle| !handle.is_finished());
        if workers.contains_key(&id) {
            return;
        }
        // Backend admits at most eight active attempts. Recovery can include
        // bounded uncertain attempts, so retain up to its total record limit.
        if workers.len() >= 64 {
            return;
        }
        let service = self.devices.clone();
        workers.insert(
            id,
            tokio::spawn(async move {
                if let Some(request) = request {
                    if service.start(request).await.is_err() {
                        return;
                    }
                }
                let _ = service.run(id, &actor).await;
            }),
        );
    }
    pub(super) async fn host_accounts(
        &self,
        command: VesselCommand,
        scope: Scope,
    ) -> Result<Value> {
        match command {
            VesselCommand::Accounts {
                workspace,
                transport,
            } => {
                scope.check(&self.directory, &workspace, ProcessRight::AccountUse)?;
                let registry = Registry::default_host()?;
                // Avoid reentering Registry from its locked list callback.
                let (revision, all) = registry.list(|_| true)?;
                let accounts: Vec<_> = all
                    .into_iter()
                    .filter(|a| scope.account_allowed(&registry, a.id, a.connection_id, &workspace))
                    .filter(|a| {
                        transport.is_none_or(|t| {
                            registry
                                .connection(a.connection_id)
                                .is_ok_and(|c| c.transports.contains(&t))
                        })
                    })
                    .collect();
                let connections: Vec<_> = registry
                    .connections()?
                    .into_iter()
                    .filter(|c| {
                        accounts.iter().any(|a| a.connection_id == c.id)
                            || scope.connection_allowed(c.id)
                    })
                    .filter(|c| transport.is_none_or(|t| c.transports.contains(&t)))
                    .collect();
                scope.check(&self.directory, &workspace, ProcessRight::AccountUse)?;
                Ok(json!({"revision": revision, "accounts": accounts, "connections": connections}))
            }
            VesselCommand::AccountDefaults { workspace } => {
                scope.check(&self.directory, &workspace, ProcessRight::AccountUse)?;
                let mut config = voyage_runtime::Config::load(None)?;
                config.materialize_legacy_account()?;
                if let Some(binding) = &config.account {
                    scope.use_account(&self.directory, &workspace, binding)?;
                }
                config.validate_account()?;
                let secrets = voyage_runtime::build::redactor(&config);
                voyage_runtime::provider::validate_models_for_display(
                    &[voyage_runtime::provider::ModelInfo::minimal(
                        config.model.clone(),
                    )],
                    |value| secrets.contains_secret(value),
                )?;
                Ok(
                    json!({"account":config.account,"provider":config.provider,"model":config.model,"reasoning_effort":config.reasoning_effort,"service_tier":config.service_tier}),
                )
            }
            VesselCommand::AccountModels { workspace, account } => {
                scope.use_account(&self.directory, &workspace, &account)?;
                let mut config = voyage_runtime::Config::load(None)?;
                config.select_account(account.clone())?;
                config.reasoning_effort = None;
                config.service_tier = None;
                let context = voyage_runtime::provider::inference_context(&config).await;
                let launch =
                    voyage_runtime::launch_config::LaunchConfig::capture(&config, &workspace)?;
                // Provider execution stays in the bounded Voyage helper. It resolves
                // executing-host policy and validates model text before returning it.
                let models = self
                    .discover_models(workspace.clone(), serde_json::to_value(launch)?)
                    .await?;
                scope.use_account(&self.directory, &workspace, &account)?;
                ensure!(
                    context.is_some()
                        && voyage_runtime::provider::inference_context(&config).await == context,
                    "account catalog context changed"
                );
                Ok(
                    json!({"account":account,"capability_revision":Registry::default_host()?.validate_binding(&account)?.capability_revision,"models":models}),
                )
            }
            VesselCommand::EnrollAccount {
                command_id,
                enrollment_id,
                workspace,
                connection_id,
                alias,
                label,
            } => {
                scope.check(&self.directory, &workspace, ProcessRight::AccountEnroll)?;
                ensure!(
                    scope.connection_allowed(connection_id),
                    "enrollment connection denied"
                );
                let actor = scope.actor(&workspace);
                let request = EnrollmentRequest {
                    command_id,
                    enrollment_id,
                    connection_id,
                    alias,
                    label,
                    actor: actor.clone(),
                };
                // Track ownership BEFORE start/network effects. The same task owns
                // both start and polling, even if the socket waiter disappears.
                // Duplicate drivers are harmless: the durable Exchanging transition
                // serializes polling, while start checks the exact original envelope.
                let mut workers = self.enrollment_workers.lock().await;
                workers.retain(|_, handle| !handle.is_finished());
                ensure!(workers.len() < 64, "enrollment worker capacity reached");
                let service = self.devices.clone();
                let (send, receive) = tokio::sync::oneshot::channel();
                workers.insert(
                    Uuid::new_v4(),
                    tokio::spawn(async move {
                        match service.start(request).await {
                            Ok(status) => {
                                let _ = send.send(Ok(status));
                                let _ = service.run(enrollment_id, &actor).await;
                            }
                            Err(error) => {
                                let _ = send.send(Err(error));
                            }
                        }
                    }),
                );
                drop(workers);
                Ok(serde_json::to_value(receive.await??)?)
            }
            VesselCommand::CancelAccountEnrollment {
                command_id,
                enrollment_id,
                workspace,
            } => {
                scope.check(&self.directory, &workspace, ProcessRight::AccountEnroll)?;
                Ok(serde_json::to_value(self.devices.cancel(
                    command_id,
                    enrollment_id,
                    &scope.actor(&workspace),
                )?)?)
            }
            VesselCommand::PrivateAccountEnrollment {
                enrollment_id,
                workspace,
            } => {
                scope.check(&self.directory, &workspace, ProcessRight::AccountEnroll)?;
                // Deliberately no command_record, receipt, event or diagnostics.
                Ok(serde_json::to_value(
                    self.devices
                        .status(enrollment_id, &scope.actor(&workspace))?,
                )?)
            }
            other => self.start_account(other, scope).await,
        }
    }
}

impl Supervisor {
    async fn start_account(&self, command: VesselCommand, scope: Scope) -> Result<Value> {
        let (
            resolve,
            command_id,
            session_id,
            workspace,
            account,
            model,
            reasoning_effort,
            service_tier,
        ) = match &command {
            VesselCommand::StartAccount {
                command_id,
                session_id,
                workspace,
                account,
                model,
                reasoning_effort,
                service_tier,
            } => (
                false,
                *command_id,
                *session_id,
                workspace.clone(),
                account.clone(),
                model.clone(),
                reasoning_effort.clone(),
                service_tier.clone(),
            ),
            VesselCommand::ResolveStartAccount {
                command_id,
                session_id,
                workspace,
                account,
                model,
                reasoning_effort,
                service_tier,
            } => (
                true,
                *command_id,
                *session_id,
                workspace.clone(),
                account.clone(),
                model.clone(),
                reasoning_effort.clone(),
                service_tier.clone(),
            ),
            _ => anyhow::bail!("not an account operation"),
        };
        ensure!(
            !command_id.is_nil() && !session_id.is_nil(),
            "nil creation identity"
        );
        let right = match &scope {
            Scope::Session(_) => ProcessRight::Lifecycle,
            _ => ProcessRight::Create,
        };
        scope.check(&self.directory, &workspace, right)?;
        if let Scope::Session(g) = &scope {
            ensure!(g.session_id == session_id, "session creation scope denied");
        }
        // Resolve must compare the exact immutable envelope even if the account
        // is now logged out; it does not infer permission to dispatch it again.
        let original = VesselCommand::StartAccount {
            command_id,
            session_id,
            workspace: workspace.clone(),
            account: account.clone(),
            model: model.clone(),
            reasoning_effort: reasoning_effort.clone(),
            service_tier: service_tier.clone(),
        };
        let directory = registry::directory(&self.directory, session_id);
        let config_path = directory.join(format!("account-start-{command_id}.json"));
        if resolve {
            return self
                .resolve_start_original(
                    command_id,
                    session_id,
                    workspace,
                    Some(config_path),
                    original,
                )
                .await;
        }
        scope.use_account(&self.directory, &workspace, &account)?;
        if let Scope::Connection(grant) = &scope {
            self.connection_session(grant, session_id, &workspace)?;
        }
        let lock = {
            let mut locks = self.lifecycle_locks.lock().await;
            locks
                .entry(session_id)
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let _lock = lock.lock().await;
        let mut config = voyage_runtime::Config::load(None)?;
        config.select_account(account)?;
        config.model = model;
        config.reasoning_effort = reasoning_effort;
        config.service_tier = service_tier;
        voyage_runtime::provider::validate_inference_settings(&config)?;
        let launch = voyage_runtime::launch_config::LaunchConfig::capture(&config, &workspace)?;
        registry::private_directory(&directory)?;
        // A retry must retain the first trusted configuration (including policy
        // defaults), not recapture changed host defaults under the same identity.
        if config_path.exists() {
            let prior: voyage_runtime::launch_config::LaunchConfig =
                store::load_bounded(&config_path, 1024 * 1024)?;
            let previous = prior.resolve(&workspace)?;
            ensure!(
                previous.account == config.account
                    && previous.model == config.model
                    && previous.reasoning_effort == config.reasoning_effort
                    && previous.service_tier == config.service_tier,
                "account start configuration conflict"
            );
        } else {
            store::save_bounded(&config_path, &launch, 1024 * 1024)?;
        }
        self.start_initialized(
            command_id,
            session_id,
            workspace,
            Some(config_path),
            None,
            original,
        )
        .await
    }
}
