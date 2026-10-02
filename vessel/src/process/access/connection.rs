//! Workspace routing derives session authority without changing local policy.
use super::{Supervisor, store};
use crate::process::registry;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use voyage_protocol::process::*;
use voyage_protocol::vessel::{VESSEL_API_VERSION, VoyageCommand};

impl Supervisor {
    async fn administrative_authority(&self, grant: &ConnectionGrant) -> Result<u64> {
        #[cfg(target_os = "linux")]
        {
            crate::process::database::execution_reviews::authority(&self.directory, grant).await
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = grant;
            anyhow::bail!("administrator execution is unavailable on this platform")
        }
    }
    #[cfg(not(target_os = "linux"))]
    pub(in crate::process) async fn execution_connection(
        &self,
        _grant: &ConnectionGrant,
        registration: &ProcessRegistration,
        _observe_revoked: bool,
    ) -> Result<()> {
        ordinary(registration)
    }
    #[cfg(target_os = "linux")]
    pub(in crate::process) async fn execution_connection(
        &self,
        grant: &ConnectionGrant,
        registration: &ProcessRegistration,
        observe_revoked: bool,
    ) -> Result<()> {
        if registration
            .peer_uids
            .as_ref()
            .is_none_or(|uids| uids.runtime != 0)
        {
            return ordinary(registration);
        }
        ensure!(
            grant.full_access,
            "administrator Voyage requires its explicit enrolled owner"
        );
        crate::process::database::execution_reviews::authority(&self.directory, grant).await?;
        let binding =
            crate::process::database::execution_binding(&self.directory, registration.session_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("administrator binding unavailable"))?;
        let id = binding
            .administrator_grant_id
            .ok_or_else(|| anyhow::anyhow!("administrator authorization missing"))?;
        let authorization =
            crate::process::database::administrator_grant(&self.directory, id).await?;
        ensure!(
            authorization.administrative_owner_id == grant.principal_id
                && authorization.vessel_id == grant.vessel_id
                && authorization.session_id == registration.session_id
                && (observe_revoked || authorization.revoked_at_ms.is_none()),
            "administrator Voyage authority unavailable"
        );
        Ok(())
    }
    #[cfg(test)]
    pub(super) async fn connected(
        &self,
        id: Uuid,
        token: &str,
        command: VesselCommand,
    ) -> Result<Value> {
        self.connected_with_authority(id, token, None, command)
            .await
    }
    pub(super) async fn connected_with_authority(
        &self,
        id: Uuid,
        token: &str,
        expected_authority: Option<&str>,
        command: VesselCommand,
    ) -> Result<Value> {
        let grant = store::authenticate_connection(&self.directory, id, token)?;
        if let Some(expected) = expected_authority {
            ensure!(
                expected.len() == 64
                    && expected.bytes().all(|b| b.is_ascii_hexdigit())
                    && store::connection_authority_fingerprint(&grant)? == expected,
                "saved connection authority changed"
            );
        }
        let has = |right| -> Result<()> {
            ensure!(grant.rights.contains(&right), "workspace permission denied");
            Ok(())
        };
        let operation = command.clone();
        let response = match command {
            VesselCommand::Notifications { operation } => {
                self.notifications(
                    operation,
                    Some(GrantBinding {
                        grant_id: grant.grant_id,
                        principal_id: grant.principal_id,
                        revision: grant.revision,
                    }),
                )
                .await
            }
            VesselCommand::Capabilities => Ok(json!({
                "protocol": VESSEL_API_VERSION, "version": env!("CARGO_PKG_VERSION"),
                "vessel_id": grant.vessel_id, "principal_id": grant.principal_id,
                "scope": if grant.full_access { "owner" } else { "workspaces" }, "grant_revision": grant.revision,
                "rights": grant.rights, "expires_at_ms": grant.expires_at_ms,
                "authorization_fingerprint": store::connection_authority_fingerprint(&grant)?,
                "workspaces": self.connection_workspaces(&grant).await?,
                "running_release": crate::process::updates::running_release(),
                "remote_updates": grant.full_access && crate::process::updates::supported(&self.directory)
                    && (!crate::process::runtime_storage::has_bound_layout(&self.directory)
                        || self.administrative_authority(&grant).await.is_ok()),
                "features": ({let mut features=vec!["saved_authority_pin","sqlite_catalogue","catalogue_changes","workspace_pairing", "sse_events","duplex_socket", "notifications","scoped_catalogue", "voyage_operations", "grant_revocation","start_resolution","provider_accounts","execution_profiles","account_start","private_account_enrollment","execution_budget","workspace_changes","workspace_file","skills_catalog","workspace_file_catalog","goals","start_settings"];
                    if crate::process::runtime_storage::has_bound_layout(&self.directory){features.push("execution_identity");}
                    if grant.full_access && crate::process::updates::verified_user_updates(&self.directory){features.push("verified_user_updates");}features})
            })),
            VesselCommand::Execution { operation } => {
                use voyage_protocol::execution_review_control::ExecutionOperation;
                match &operation {
                    ExecutionOperation::Inventory
                    | ExecutionOperation::Review { .. }
                    | ExecutionOperation::Status { .. } => has(ProcessRight::Observe)?,
                    ExecutionOperation::PrepareTransition { .. }
                    | ExecutionOperation::ReconcileTransition { .. } => {
                        has(ProcessRight::Observe)?;
                        has(ProcessRight::Lifecycle)?;
                        has(ProcessRight::Cancel)?;
                        has(ProcessRight::Execute)?;
                        has(ProcessRight::Decide)?;
                    }
                    ExecutionOperation::Prepare { .. } | ExecutionOperation::Approve { .. } => {
                        has(ProcessRight::Create)?;
                        has(ProcessRight::Execute)?;
                        has(ProcessRight::Decide)?;
                    }
                    ExecutionOperation::Control { .. } => has(ProcessRight::Lifecycle)?,
                }
                if !matches!(
                    &operation,
                    ExecutionOperation::Inventory | ExecutionOperation::Status { .. }
                ) {
                    ensure!(
                        grant.full_access,
                        "execution identity review requires explicit account-owner connection"
                    );
                }
                store::current_connection(&self.directory, &grant)?;
                #[cfg(target_os = "linux")]
                {
                    self.execution_operation(&grant, operation).await
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = operation;
                    anyhow::bail!("execution identity is unavailable on this platform")
                }
            }
            command @ (VesselCommand::UpdatePrepare { .. }
            | VesselCommand::UpdateStatus { .. }
            | VesselCommand::UpdateApply { .. }
            | VesselCommand::UpdateDiscard { .. }) => {
                ensure!(
                    grant.full_access,
                    "Updating Vessel requires account-owner authority"
                );
                store::current_connection(&self.directory, &grant)?;
                if crate::process::runtime_storage::has_bound_layout(&self.directory) {
                    self.administrative_authority(&grant).await?;
                }
                self.update(command).await
            }
            command @ (VesselCommand::Accounts { .. }
            | VesselCommand::AccountDefaults { .. }
            | VesselCommand::Profiles { .. }
            | VesselCommand::SaveProfile { .. }
            | VesselCommand::DeleteProfile { .. }
            | VesselCommand::SetDefaultProfile { .. }
            | VesselCommand::AccountUsage { .. }
            | VesselCommand::AccountSetDefault { .. }
            | VesselCommand::AccountModels { .. }
            | VesselCommand::StartAccount { .. }
            | VesselCommand::StartSettings { .. }
            | VesselCommand::ResolveStartAccount { .. }
            | VesselCommand::EnrollAccount { .. }
            | VesselCommand::CancelAccountEnrollment { .. }
            | VesselCommand::ResolveAccountEnrollment { .. }
            | VesselCommand::PrivateAccountEnrollment { .. }) => {
                self.host_accounts(
                    command,
                    crate::process::accounts::Scope::Connection(grant.clone()),
                )
                .await
            }
            command @ (VesselCommand::Catalogue | VesselCommand::CatalogueChanges { .. }) => {
                has(ProcessRight::Catalogue)?;
                let mut page = if let VesselCommand::CatalogueChanges {
                    after,
                    limit,
                    wait_ms,
                } = command
                {
                    Some(self.catalogue_changes(after, limit, wait_ms).await?)
                } else {
                    None
                };
                let source = match page.as_mut() {
                    Some(page) => std::mem::take(&mut page.entries),
                    None => self.catalogue().await?,
                };
                let mut entries = Vec::new();
                for entry in source {
                    if (grant.full_access
                        || grant.workspaces.iter().any(|w| w.path == entry.workspace))
                        && let Ok(registration) = self.registration(entry.session_id).await
                        && self
                            .execution_connection(&grant, &registration, true)
                            .await
                            .is_ok()
                    {
                        entries.push(entry);
                    }
                }
                if !grant.rights.contains(&ProcessRight::History) {
                    for entry in &mut entries {
                        if let Some(metadata) = &mut entry.catalogue {
                            metadata.summary = None;
                        }
                    }
                }
                store::current_connection(&self.directory, &grant)?;
                if let Some(mut page) = page {
                    page.entries = entries;
                    Ok(serde_json::to_value(page)?)
                } else {
                    Ok(serde_json::to_value(entries)?)
                }
            }
            VesselCommand::ResolveStart {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                has(ProcessRight::Create)?;
                ensure!(
                    config_path.is_none()
                        || (grant.full_access
                            && crate::process::runtime_storage::has_bound_layout(&self.directory)),
                    "host configuration denied"
                );
                approved(&grant, &workspace)?;
                if let Ok(existing) = self.registration(session_id).await {
                    ordinary(&existing)?;
                    ensure!(
                        existing.workspace == workspace,
                        "session workspace conflict"
                    );
                }
                // Resolve binds the original Start, never a second lifecycle payload.
                let original = match &config_path {
                    Some(path) => VesselCommand::StartConfigured {
                        command_id,
                        session_id,
                        workspace: workspace.clone(),
                        config_path: path.clone(),
                    },
                    None => VesselCommand::Start {
                        command_id,
                        session_id,
                        workspace: workspace.clone(),
                    },
                };
                self.bind_connection_operation(&grant, command_id, &original)
                    .await?;
                self.resolve_start(command_id, session_id, workspace, config_path)
                    .await
            }
            VesselCommand::StartConfigured {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                ensure!(
                    grant.full_access
                        && crate::process::runtime_storage::has_bound_layout(&self.directory),
                    "configured system creation requires owner authority"
                );
                has(ProcessRight::Create)?;
                approved(&grant, &workspace)?;
                if let Ok(existing) = self.registration(session_id).await {
                    ordinary(&existing)?;
                    ensure!(
                        existing.workspace == workspace,
                        "session workspace conflict"
                    );
                }
                self.bind_connection_operation(&grant, command_id, &operation)
                    .await?;
                self.connection_session(&grant, session_id, &workspace)?;
                store::current_connection(&self.directory, &grant)?;
                self.start(command_id, session_id, workspace, Some(config_path))
                    .await
            }
            VesselCommand::Start {
                command_id,
                session_id,
                workspace,
            } => {
                has(ProcessRight::Create)?;
                ensure!(
                    !session_id.is_nil() && !command_id.is_nil(),
                    "nil creation identity"
                );
                let canonical = std::fs::canonicalize(&workspace)?;
                ensure!(
                    workspace == canonical,
                    "use the exact approved workspace path"
                );
                approved(&grant, &canonical)?;
                if let Ok(existing) = self.registration(session_id).await {
                    ordinary(&existing)?;
                    ensure!(
                        existing.workspace == canonical,
                        "session workspace conflict"
                    );
                }
                self.bind_connection_operation(&grant, command_id, &operation)
                    .await?;
                self.connection_session(&grant, session_id, &canonical)?;
                // Dispatch the path that was authorized, not a caller-supplied
                // symlink that could resolve differently after the scope check.
                // Retain the exact original request for command deduplication.
                let original = VesselCommand::Start {
                    command_id,
                    session_id,
                    workspace,
                };
                if crate::process::runtime_storage::has_bound_layout(&self.directory) {
                    self.start_identity_request(
                        original,
                        crate::process::accounts::Scope::Connection(grant.clone()),
                    )
                    .await
                } else {
                    self.start_initialized(command_id, session_id, canonical, None, None, original)
                        .await
                }
            }
            VesselCommand::Inspect { session_id } => {
                has(ProcessRight::Observe)?;
                let registration = self.registration(session_id).await?;
                self.execution_connection(&grant, &registration, true)
                    .await?;
                approved(&grant, &registration.workspace)?;
                let info = self.inspect_registration(&registration).await;
                store::current_connection(&self.directory, &grant)?;
                Ok(serde_json::to_value(info)?)
            }
            VesselCommand::Voyage(request) => {
                let right = crate::process::api::required_right(&request.command)
                    .or_else(|| {
                        grant
                            .full_access
                            .then(|| crate::process::api::owner_connection_right(&request.command))
                            .flatten()
                    })
                    .ok_or_else(|| anyhow::anyhow!("operation unavailable to workspace clients"))?;
                has(right)?;
                if request.command.requires_browser_history() {
                    has(ProcessRight::History)?;
                }
                if matches!(request.command, VoyageCommand::Terminal { .. }) {
                    has(ProcessRight::Execute)?;
                }
                if matches!(
                    request.command,
                    VoyageCommand::AssignmentObserve { cancel: true, .. }
                ) {
                    has(ProcessRight::Cancel)?;
                }
                let registration = self.registration(request.session_id).await?;
                self.execution_connection(&grant, &registration, false)
                    .await?;
                let binding =
                    self.connection_session(&grant, request.session_id, &registration.workspace)?;
                self.voyage(request, Some(binding)).await
            }
            VesselCommand::Stop {
                session_id,
                incarnation,
            } => {
                has(ProcessRight::Lifecycle)?;
                let registration = self.registration(session_id).await?;
                self.execution_connection(&grant, &registration, true)
                    .await?;
                let binding =
                    self.connection_session(&grant, session_id, &registration.workspace)?;
                crate::process::api::reply(self.stop(session_id, incarnation, Some(binding)).await?)
            }
            command @ VesselCommand::Branch { .. } => {
                // Branch creates a new owner from private saved provenance. Do not
                // expose it to workspace/session grants or broaden their rights.
                ensure!(
                    grant.full_access,
                    "branch requires local account-owner authority"
                );
                has(ProcessRight::Create)?;
                has(ProcessRight::History)?;
                has(ProcessRight::Lifecycle)?;
                let VesselCommand::Branch {
                    command_id,
                    session_id,
                    branch_id,
                    ..
                } = &command
                else {
                    unreachable!()
                };
                ensure!(
                    !branch_id.is_nil() && branch_id != session_id,
                    "invalid branch identity"
                );
                let registration = self.registration(*session_id).await?;
                ordinary(&registration)?;
                self.connection_session(&grant, *session_id, &registration.workspace)?;
                self.connection_session(&grant, *branch_id, &registration.workspace)?;
                self.bind_connection_operation(&grant, *command_id, &operation)
                    .await?;
                store::current_connection(&self.directory, &grant)?;
                self.branch_scoped(
                    command,
                    crate::process::accounts::Scope::Connection(grant.clone()),
                )
                .await
            }
            command @ VesselCommand::Recover { .. } => {
                ensure!(
                    grant.full_access,
                    "offline recovery requires explicit account-owner connection"
                );
                has(ProcessRight::History)?;
                has(ProcessRight::Lifecycle)?;
                let VesselCommand::Recover {
                    command_id,
                    session_id,
                    ..
                } = &command
                else {
                    unreachable!()
                };
                let registration = self.registration(*session_id).await?;
                self.bind_connection_operation(&grant, *command_id, &operation)
                    .await?;
                store::current_connection(&self.directory, &grant)?;
                if registration.peer_uids.is_some() {
                    self.recover_bound(
                        command,
                        crate::process::accounts::Scope::Connection(grant.clone()),
                    )
                    .await
                } else {
                    self.recover(command).await
                }
            }
            VesselCommand::Restart {
                command_id,
                session_id,
                incarnation,
            } => {
                has(ProcessRight::Lifecycle)?;
                let registration = self.registration(session_id).await?;
                ordinary(&registration)?;
                self.connection_session(&grant, session_id, &registration.workspace)?;
                self.bind_connection_operation(&grant, command_id, &operation)
                    .await?;
                self.restart(command_id, session_id, incarnation).await
            }
            _ => anyhow::bail!("operation requires local account-owner authority"),
        }?;
        self.finish_connected_reply(&grant, response)
    }

    fn finish_connected_reply(
        &self,
        grant: &ConnectionGrant,
        mut response: Value,
    ) -> Result<Value> {
        // Dispatch already succeeded. A later revocation/rotation can withhold its
        // response, but cannot establish that no effect was admitted. Preserve
        // the exact original operation for receipt observation, never replay it.
        // Vessel commands have no exhaustive effect classifier; conservatively
        // mark every failed postdispatch authority observation as unknown.
        store::current_connection(&self.directory, grant)
            .map_err(|error| error.context(super::super::routing::OutcomeUnknown))?;
        if !grant.rights.contains(&ProcessRight::History) {
            super::redact_catalogue_reply(&mut response);
        }
        Ok(response)
    }

    async fn connection_workspaces(
        &self,
        grant: &ConnectionGrant,
    ) -> Result<Vec<ApprovedWorkspace>> {
        if !grant.full_access {
            return Ok(grant.workspaces.clone());
        }
        let mut paths = std::collections::BTreeSet::new();
        for entry in self.catalogue().await? {
            if self
                .registration(entry.session_id)
                .await
                .is_ok_and(|registration| ordinary(&registration).is_ok())
                && entry.workspace.is_dir()
                && std::fs::canonicalize(&entry.workspace).is_ok_and(|p| p == entry.workspace)
            {
                paths.insert(entry.workspace);
            }
        }
        if paths.is_empty() {
            let default = if crate::process::runtime_storage::has_bound_layout(&self.directory) {
                crate::process::default_execution::protected_default(&self.directory)?.home
            } else {
                std::env::current_dir()?
            };
            paths.insert(std::fs::canonicalize(default)?);
        }
        Ok(paths
            .into_iter()
            .map(|path| {
                let mut hash = Sha256::new();
                hash.update(b"voyage/owner-workspace/v1\0");
                hash.update(grant.vessel_id.as_bytes());
                hash.update(path.as_os_str().as_encoded_bytes());
                let bytes: [u8; 32] = hash.finalize().into();
                ApprovedWorkspace {
                    id: Uuid::from_bytes(bytes[..16].try_into().expect("UUID bytes")),
                    name: path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "Workspace".into()),
                    path,
                    provider_ready: None,
                }
            })
            .collect())
    }

    // The public start payload has no bearer. Retain its exact principal/grant
    // binding separately, before effects, so replacement credentials cannot replay it.
    async fn bind_connection_operation(
        &self,
        grant: &ConnectionGrant,
        id: Uuid,
        command: &VesselCommand,
    ) -> Result<()> {
        let _serial = self.registrations.lock().await?;
        store::current_connection(&self.directory, grant)?;
        let directory = self.directory.join("access/connection-commands");
        registry::private_directory(&directory)?;
        let path = directory.join(format!("{id}.json"));
        let intent = json!({"schema_version": 1, "command_id": id,
            "vessel_id": grant.vessel_id, "grant_id": grant.grant_id,
            "principal_id": grant.principal_id, "grant_revision": grant.revision,
            "command": command});
        if path.try_exists()? {
            let prior: Value = store::load(&path)?;
            ensure!(
                prior == intent,
                "connection command identity or payload conflict"
            );
        } else {
            ensure!(
                std::fs::read_dir(&directory)?.take(65536).count() < 65536,
                "connection command capacity exhausted"
            );
            store::save(&path, &intent)?;
        }
        Ok(())
    }

    pub(crate) fn connection_session(
        &self,
        grant: &ConnectionGrant,
        session_id: Uuid,
        workspace: &std::path::Path,
    ) -> Result<GrantBinding> {
        approved(grant, workspace)?;
        store::current_connection(&self.directory, grant)?;
        let mut digest = Sha256::new();
        digest.update(b"voyage/connection-session/v1\0");
        digest.update(grant.grant_id.as_bytes());
        digest.update(session_id.as_bytes());
        if let Some(epoch) = super::execution_epoch::current(&self.directory, session_id)? {
            digest.update(b"system-execution-epoch/v1\0");
            digest.update(epoch.as_bytes());
            digest.update(grant.revision.to_be_bytes());
        }
        let bytes: [u8; 32] = digest.finalize().into();
        let id = Uuid::from_bytes(bytes[..16].try_into()?);
        let binding = GrantBinding {
            grant_id: id,
            revision: grant.revision,
            principal_id: grant.principal_id,
        };
        let path = store::grant_path(&self.directory, id);
        store::initialize(&self.directory)?;
        let rights: Vec<_> = grant
            .rights
            .iter()
            .copied()
            .filter(|r| !matches!(r, ProcessRight::Catalogue | ProcessRight::Create))
            .collect();
        if path.exists() {
            let previous: ProcessGrant = store::load(&path)?;
            ensure!(
                previous.full_access == grant.full_access
                    && previous.grant_id == id
                    && previous.principal_id == grant.principal_id
                    && previous.session_id == session_id
                    && previous.workspace == workspace
                    && previous.revision == grant.revision
                    && !previous.revoked
                    && previous.expires_at_ms == grant.expires_at_ms
                    && previous.rights == rights
                    && previous.accounts == grant.accounts
                    && previous.enrollment_connections == grant.enrollment_connections
                    && previous.parent_grant.is_none()
                    && previous.participant_binding.is_none()
                    && previous
                        .connection_binding
                        .as_ref()
                        .is_some_and(|b| b.grant_id == grant.grant_id
                            && b.revision == grant.revision
                            && b.principal_id == grant.principal_id),
                "derived session authority conflict"
            );
        } else {
            ensure!(
                std::fs::read_dir(path.parent().expect("grant directory"))?
                    .take(16384)
                    .count()
                    < 16384,
                "session authority capacity exhausted"
            );
            let derived = ProcessGrant {
                full_access: grant.full_access,
                grant_id: id,
                principal_id: grant.principal_id,
                session_id,
                workspace: workspace.to_owned(),
                revision: grant.revision,
                rights,
                accounts: grant.accounts.clone(),
                enrollment_connections: grant.enrollment_connections.clone(),
                expires_at_ms: grant.expires_at_ms,
                revoked: false,
                token_hash: String::new(),
                parent_grant: None,
                participant_binding: None,
                connection_binding: Some(GrantBinding {
                    grant_id: grant.grant_id,
                    revision: grant.revision,
                    principal_id: grant.principal_id,
                }),
            };
            super::execution_epoch::pin(&self.directory, &derived)?;
            store::save(&path, &derived)?;
        }
        let current: ProcessGrant = store::load(&path)?;
        super::execution_epoch::check(&self.directory, &current)?;
        Ok(binding)
    }
}

fn approved(grant: &ConnectionGrant, workspace: &std::path::Path) -> Result<()> {
    ensure!(
        workspace.is_absolute()
            && workspace.is_dir()
            && std::fs::canonicalize(workspace)? == workspace
            && (grant.full_access || grant.workspaces.iter().any(|w| w.path == workspace)),
        "workspace scope denied"
    );
    Ok(())
}

fn ordinary(registration: &ProcessRegistration) -> Result<()> {
    ensure!(
        !matches!(
            registration.initialize,
            Some(RuntimeInitialization::Participant { .. })
        ),
        "participant sessions require their accepted participant authority"
    );
    Ok(())
}

#[cfg(test)]
#[path = "scoped_frontdoor_tests.rs"]
mod scoped_frontdoor_tests;
