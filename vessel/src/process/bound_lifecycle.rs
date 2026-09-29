//! Bound processes keep supervisor authority outside runtime-owned files.
use super::{database, guardian, routing, runtime_storage, service::Supervisor};
use anyhow::{Result, ensure};
use std::path::Path;
use std::{process::Stdio, time::Duration};
use uuid::Uuid;
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
            guardian::cleanup_observed(&self.directory, session_id, incarnation)?,
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
        database::restart_bound(
            &self.directory,
            &previous,
            &next,
            serde_json::to_vec(&command)?,
        )
        .await
        .map_err(|error| error.context(routing::OutcomeUnknown))?;
        let mut process = tokio::process::Command::new(vessel);
        process
            .arg("guard-bound")
            .arg("--directory")
            .arg(&self.directory)
            .arg("--session")
            .arg(session_id.to_string())
            .arg("--incarnation")
            .arg(next.incarnation.to_string())
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
        let mut child = process
            .spawn()
            .map_err(|error| anyhow::Error::new(error).context(routing::OutcomeUnknown))?;
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
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
                let _ = routing::forward_authorized(
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
        ensure!(
            !matches!(command, RuntimeCommand::SetAccountInference { .. }),
            "bound account selection requires identity-scoped account helpers"
        );
        if matches!(command, RuntimeCommand::Stop) {
            return self.bound_stop(&registration, authorization).await;
        }
        let directory = runtime_storage::directory(&self.directory, &registration).await?;
        let result = routing::forward_authorized(
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
