//! Bound processes keep supervisor authority outside runtime-owned files.
use super::{database, guardian, routing, runtime_storage, service::Supervisor};
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};
use std::{process::Stdio, time::Duration};
use uuid::Uuid;
use voyage_protocol::execution_identity::{AuthorityClass, ExecutionBinding};
use voyage_protocol::process::*;

pub(super) async fn inspect(root: &Path, registration: &ProcessRegistration) -> ProcessInfo {
    let mut info = ProcessInfo::from(registration);
    if registration.state == ProcessState::Relinquished {
        return info;
    }
    if guardian::cleanup_observed(root, registration.session_id, registration.incarnation)
        .unwrap_or(false)
    {
        info.state = ProcessState::Stopped;
        return info;
    }
    if super::migration::dormant(root, registration.session_id, registration.incarnation)
        .unwrap_or(false)
        || super::execution_transition::dormant(
            root,
            registration.session_id,
            registration.incarnation,
        )
        .unwrap_or(false)
    {
        info.state = ProcessState::Suspended;
        return info;
    }
    info.state = ProcessState::Unavailable;
    if let Ok(directory) = runtime_storage::directory(root, registration).await
        && let Ok(response) =
            routing::forward(&directory, registration, RuntimeCommand::Health).await
        && response.error.is_none()
        && database::bound_observer_identity(root, registration)
            .await
            .is_ok()
    {
        info.state = ProcessState::Live;
    }
    info
}

impl Supervisor {
    pub(super) async fn bound_creation_lock(
        &self,
        session: Uuid,
    ) -> Result<std::sync::Arc<tokio::sync::Mutex<()>>> {
        ensure!(!session.is_nil(), "nil creation identity");
        let mut locks = self.lifecycle_locks.lock().await;
        ensure!(
            locks.contains_key(&session) || locks.len() < 4096,
            "bound creation retention limit reached"
        );
        Ok(locks
            .entry(session)
            .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
            .clone())
    }
    /// Admission with a supervisor-created ordinary execution binding. Public
    /// configured creation selects only the protected default identity.
    #[cfg(test)]
    pub(super) async fn start_bound_configured(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
        binding: ExecutionBinding,
    ) -> Result<serde_json::Value> {
        let lock = self.bound_creation_lock(session_id).await?;
        let _guard = lock.lock().await;
        self.start_bound_configured_locked(command_id, session_id, workspace, config_path, binding)
            .await
    }

    pub(super) async fn start_bound_configured_locked(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
        binding: ExecutionBinding,
    ) -> Result<serde_json::Value> {
        let original = VesselCommand::StartConfigured {
            command_id,
            session_id,
            workspace: workspace.clone(),
            config_path: config_path.clone(),
        };
        self.start_bound_request_locked(
            command_id,
            session_id,
            workspace,
            config_path,
            binding,
            original,
        )
        .await
    }

    pub(super) async fn start_bound_request_locked(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
        binding: ExecutionBinding,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        self.start_bound_initialized_request_locked(
            command_id,
            session_id,
            workspace,
            config_path,
            binding,
            None,
            command,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn start_bound_initialized_request_locked(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: PathBuf,
        binding: ExecutionBinding,
        initialize: Option<RuntimeInitialization>,
        command: VesselCommand,
    ) -> Result<serde_json::Value> {
        ensure!(unsafe { libc::geteuid() } == 0, "root supervisor required");
        ensure!(
            !command_id.is_nil()
                && !session_id.is_nil()
                && !binding.incarnation.is_nil()
                && binding.session_id == session_id
                && binding.peer_uids.supervisor == 0,
            "invalid bound creation identity"
        );
        ensure!(
            workspace.is_absolute() && config_path.is_absolute(),
            "bound workspace and configuration must be absolute host paths"
        );
        let mut registrations = self.registrations.lock().await?;
        ensure!(
            !super::start::resolution_record(
                &self.directory,
                "intent",
                command_id,
                &command,
                false,
            )
            .await?,
            "start command was fenced as not admitted"
        );
        if super::registry::command_record(&self.directory, command_id, &command, false).await? {
            let previous = registrations.get(&session_id).ok_or_else(|| {
                anyhow::anyhow!("bound creation outcome unconfirmed; inspect retained admission")
                    .context(routing::OutcomeUnknown)
            })?;
            ensure!(
                previous.command_id == command_id
                    && previous.initialize == initialize
                    && previous.config_path.as_ref() == Some(&config_path)
                    && previous.incarnation == binding.incarnation
                    && previous.peer_uids.as_ref() == Some(&binding.peer_uids)
                    && database::execution_binding(&self.directory, session_id)
                        .await?
                        .as_ref()
                        == Some(&binding),
                "bound creation receipt conflict"
            );
            let previous = previous.clone();
            drop(registrations);
            return Ok(serde_json::to_value(
                inspect(&self.directory, &previous).await,
            )?);
        }
        ensure!(
            !registrations.contains_key(&session_id),
            "voyage already reserved"
        );
        runtime_storage::validate_bound_layout(&self.directory)?;
        let identity = database::configured_identity(&self.directory, &binding.identity).await?;
        ensure!(
            matches!(
                (identity.authority, binding.administrator_grant_id),
                (AuthorityClass::Ordinary, None) | (AuthorityClass::Administrator, Some(_))
            ) && identity.account_context == binding.account_context
                && identity.uid == binding.peer_uids.runtime,
            "bound creation identity changed"
        );
        super::launch::validate_identity(&identity)?;
        super::launch::protected_binary(&self.binary)?;
        super::launch::protected_binary(&self.binary.with_file_name("vessel"))?;
        ensure!(
            config_path.is_file(),
            "bound launch configuration unavailable"
        );
        let workspace = std::fs::canonicalize(workspace)?;
        ensure!(workspace.is_dir(), "workspace must be a directory");
        let registration = ProcessRegistration {
            executable: Some(self.binary.clone()),
            protocol: PROCESS_PROTOCOL,
            session_id,
            incarnation: binding.incarnation,
            command_id,
            restart_from: None,
            initialize,
            config_path: Some(config_path),
            token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            peer_uids: Some(binding.peer_uids.clone()),
            workspace,
            state: ProcessState::Starting,
            name: None,
        };
        database::admit_with_binding(
            &self.directory,
            &registration,
            serde_json::to_vec(&command)?,
            Some(&binding),
        )
        .await
        .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registrations.insert(session_id, registration.clone());
        drop(registrations);
        self.spawn_bound_guardian(&registration)
            .await
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        let info = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let info = inspect(&self.directory, &registration).await;
                if matches!(info.state, ProcessState::Live | ProcessState::Stopped) {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("bound creation outcome unconfirmed; inspect retained incarnation")
                .context(routing::OutcomeUnknown)
        })?;
        database::settle_creation(&self.directory, command_id, &info)
            .await
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        Ok(serde_json::to_value(info)?)
    }

    pub(super) async fn spawn_bound_guardian(
        &self,
        registration: &ProcessRegistration,
    ) -> Result<()> {
        let vessel = self.binary.with_file_name("vessel");
        let mut process = tokio::process::Command::new(vessel);
        process
            .arg("guard-bound")
            .arg("--directory")
            .arg(&self.directory)
            .arg("--session")
            .arg(registration.session_id.to_string())
            .arg("--incarnation")
            .arg(registration.incarnation.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        // Independent process group/lifetime; service detach cannot cancel it.
        unsafe {
            process.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = process.spawn()?;
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        Ok(())
    }

    pub(super) async fn restart_bound(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        incarnation: Uuid,
    ) -> Result<serde_json::Value> {
        let lock = self.lifecycle_lock(session_id).await?;
        let _guard = lock.lock().await;
        let previous = self.registration(session_id).await?;
        let command = VesselCommand::Restart {
            command_id,
            session_id,
            incarnation,
        };
        if super::registry::command_record(&self.directory, command_id, &command, false).await? {
            // A committed admission is never re-launched on a retry, even if
            // the service died between recording it and spawning its guardian.
            return Ok(serde_json::to_value(
                inspect(&self.directory, &previous).await,
            )?);
        }
        ensure!(
            previous.incarnation == incarnation,
            "stale runtime incarnation"
        );
        ensure!(
            previous.state != ProcessState::Relinquished,
            "ownership relinquished"
        );
        ensure!(
            guardian::cleanup_observed(&self.directory, session_id, incarnation).unwrap_or(false)
                || super::migration::dormant(&self.directory, session_id, incarnation)
                    .unwrap_or(false)
                || super::execution_transition::dormant(&self.directory, session_id, incarnation)
                    .unwrap_or(false),
            "restart requires protected observed local cleanup"
        );
        let identity = database::bound_observer_identity(&self.directory, &previous).await?;
        super::launch::validate_identity(&identity)?;
        super::launch::protected_binary(&self.binary)?;
        // A release pins both executables in the same protected bin directory.
        let vessel = self.binary.with_file_name("vessel");
        super::launch::protected_binary(&vessel)?;
        let mut next = previous.clone();
        next.executable = Some(self.binary.clone());
        next.incarnation = Uuid::new_v4();
        next.command_id = command_id;
        next.restart_from = Some(incarnation);
        next.token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        next.state = ProcessState::Starting;
        if let Some(digest) = super::identity_start::launch_digest(&self.directory, &previous)? {
            let path = next
                .config_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("retained launch path missing"))?;
            super::identity_start::pin_launch(
                &self.directory,
                command_id,
                session_id,
                path,
                &digest,
            )?;
        }
        super::execution_transition::carry_retained_digest(&self.directory, &previous, &next)?;
        super::admin_execution::carry_namespace(&self.directory, &previous, &next, &identity)?;
        database::restart_bound(
            &self.directory,
            &previous,
            &next,
            serde_json::to_vec(&command)?,
        )
        .await
        .map_err(|error| error.context(routing::OutcomeUnknown))?;
        self.spawn_bound_guardian(&next)
            .await
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        let info = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let info = inspect(&self.directory, &next).await;
                if matches!(info.state, ProcessState::Live | ProcessState::Stopped) {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| {
            anyhow::anyhow!("restart outcome unconfirmed; inspect retained incarnation")
                .context(routing::OutcomeUnknown)
        })?;
        Ok(serde_json::to_value(info)?)
    }

    pub(super) async fn bound_stop(
        &self,
        registration: &ProcessRegistration,
        authorization: Option<GrantBinding>,
    ) -> Result<RuntimeResponse> {
        ensure!(
            registration.state != ProcessState::Relinquished,
            "relinquished owner cannot stop"
        );
        if let Some(binding) = &authorization {
            let grant: ProcessGrant = super::access::store::load(
                &super::access::store::grant_path(&self.directory, binding.grant_id),
            )?;
            ensure!(
                grant.session_id == registration.session_id
                    && grant.grant_id == binding.grant_id
                    && grant.revision == binding.revision
                    && grant.principal_id == binding.principal_id,
                "stop authority mismatch"
            );
            super::accounts::Scope::Session(grant).check(
                &self.directory,
                &registration.workspace,
                ProcessRight::Lifecycle,
            )?;
        }
        let cleaned = guardian::cleanup_observed(
            &self.directory,
            registration.session_id,
            registration.incarnation,
        )
        .unwrap_or(false);
        let mut saved = registration.clone();
        saved.state = if cleaned {
            ProcessState::Stopped
        } else {
            ProcessState::CleanupUnconfirmed
        };
        database::save(&self.directory, &saved).await?;
        if !cleaned {
            // Stop may retire an already-revoked execution identity. It grants no
            // new execution and never relies on child-owned registration files.
            guardian::request_stop(
                &self.directory,
                registration.session_id,
                registration.incarnation,
            )?;
            if let Ok(directory) = runtime_storage::directory(&self.directory, registration).await {
                let _ = routing::forward_bound(
                    &self.directory,
                    &directory,
                    registration,
                    RuntimeCommand::Stop,
                    authorization,
                )
                .await;
            }
        }
        Ok(RuntimeResponse {
            protocol: PROCESS_PROTOCOL,
            session_id: registration.session_id,
            incarnation: registration.incarnation,
            resumed_from: None,
            error: None,
            outcome_unknown: false,
            result: if cleaned {
                serde_json::json!({"status":"stopped","cleanup":"observed"})
            } else {
                serde_json::json!({"status":"stopping","cleanup":"pending"})
            },
        })
    }

    pub(super) async fn dispatch_bound(
        &self,
        registration: ProcessRegistration,
        command: RuntimeCommand,
        authorization: Option<GrantBinding>,
    ) -> Result<RuntimeResponse> {
        ensure!(
            registration.state != ProcessState::Relinquished,
            "source ownership has been permanently relinquished"
        );
        let lock = if matches!(
            &command,
            RuntimeCommand::Events { .. }
                | RuntimeCommand::HostBrowser { .. }
                | RuntimeCommand::HostBrowserDisconnected { .. }
        ) {
            None
        } else {
            Some(self.lifecycle_lock(registration.session_id).await?)
        };
        let _guard = if let Some(lock) = &lock {
            Some(lock.lock().await)
        } else {
            None
        };
        ensure!(
            self.registration(registration.session_id)
                .await?
                .incarnation
                == registration.incarnation,
            "runtime changed while awaiting lifecycle admission"
        );

        // Bound configuration/account changes need their executing-identity
        // helpers; never inspect the supervisor's account namespace as fallback.
        if matches!(
            command,
            RuntimeCommand::SetAccountInference { .. }
                | RuntimeCommand::SetInference { .. }
                | RuntimeCommand::SetModel { .. }
                | RuntimeCommand::SetAccess { .. }
                | RuntimeCommand::Configure { .. }
        ) {
            let binding = database::execution_binding(&self.directory, registration.session_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("bound identity unavailable"))?;
            let identity =
                database::configured_identity(&self.directory, &binding.identity).await?;
            ensure!(
                identity.uid != 0,
                "administrator account settings require a fresh execution review"
            );
        }
        if matches!(command, RuntimeCommand::Stop) {
            return self.bound_stop(&registration, authorization).await;
        }
        let directory = runtime_storage::directory(&self.directory, &registration).await?;
        let result = routing::forward_bound(
            &self.directory,
            &directory,
            &registration,
            command.clone(),
            authorization.clone(),
        )
        .await;
        if result
            .as_ref()
            .is_err_and(|e| e.downcast_ref::<routing::NotConnected>().is_some())
            && command.observes_saved()
        {
            return self
                .observe_current(&directory, &registration, command, authorization)
                .await;
        }
        let response = result?;
        database::bound_observer_identity(&self.directory, &registration).await?;
        if response.error.is_none() {
            let name = match &command {
                RuntimeCommand::Snapshot => response.result["name"].as_str(),
                RuntimeCommand::Rename { name, .. } => Some(name.as_str()),
                _ => None,
            };
            if let Some(name) = name {
                ensure!(
                    !name.is_empty() && name.len() <= 512 && !name.chars().any(char::is_control),
                    "invalid public voyage name"
                );
                let mut current = self.registration(registration.session_id).await?;
                ensure!(
                    current.incarnation == registration.incarnation,
                    "runtime changed while observing name"
                );
                if current.name.as_deref() != Some(name) {
                    current.name = Some(name.to_owned());
                    database::save(&self.directory, &current).await?;
                }
            }
        }
        Ok(response)
    }
}

#[cfg(test)]
mod start_tests {
    use super::*;
    use crate::process::test_support::Fixture;
    use voyage_protocol::execution_identity::{AccountContextRef, IdentityRef};

    #[tokio::test]
    async fn ordinary_process_cannot_reserve_a_bound_start() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let fixture = Fixture::new();
        let supervisor = fixture.supervisor().await;
        let session = Uuid::new_v4();
        let config = fixture.0.join("config.json");
        let binding = ExecutionBinding {
            session_id: session,
            incarnation: Uuid::new_v4(),
            identity: IdentityRef {
                id: Uuid::new_v4(),
                revision: 1.try_into().unwrap(),
            },
            account_context: AccountContextRef {
                id: Uuid::new_v4(),
                revision: 1.try_into().unwrap(),
            },
            peer_uids: ProcessPeerUids {
                supervisor: 0,
                runtime: unsafe { libc::geteuid() },
            },
            administrator_grant_id: None,
            host_identity_digest: "a".repeat(64),
            policy_digest: "b".repeat(64),
        };
        let error = supervisor
            .start_bound_configured(Uuid::new_v4(), session, fixture.0.clone(), config, binding)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("root supervisor required"));
        assert!(database::registration(&fixture.0, session).await.is_err());
    }
}
