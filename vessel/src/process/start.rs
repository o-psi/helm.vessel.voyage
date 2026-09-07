use super::{registry, routing, service::Supervisor};
use anyhow::{Result, ensure};
use serde_json::Value;
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;
use voyage_protocol::process::*;

impl Supervisor {
    pub(super) async fn start(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
    ) -> Result<Value> {
        ensure!(
            !command_id.is_nil() && !session_id.is_nil(),
            "session and command IDs must be nonnil"
        );
        let command = match &config_path {
            Some(path) => {
                ensure!(
                    path.is_absolute() && path.is_file(),
                    "configuration must be an absolute host file"
                );
                VesselCommand::StartConfigured {
                    command_id,
                    session_id,
                    workspace: workspace.clone(),
                    config_path: path.clone(),
                }
            }
            None => VesselCommand::Start {
                command_id,
                session_id,
                workspace: workspace.clone(),
            },
        };
        self.start_initialized(
            command_id,
            session_id,
            workspace,
            config_path,
            None,
            command,
        )
        .await
    }
    pub(super) async fn start_initialized(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
        initialize: Option<RuntimeInitialization>,
        command: VesselCommand,
    ) -> Result<Value> {
        let endpoint = registry::directory(&self.directory, session_id).join("runtime.sock");
        ensure!(
            endpoint.as_os_str().as_encoded_bytes().len() < 108,
            "runtime Unix socket path exceeds Linux limit; choose a shorter Vessel directory"
        );
        let workspace = std::fs::canonicalize(workspace)?;
        ensure!(workspace.is_dir(), "workspace must be a directory");
        let mut registrations = self.registrations.lock().await;
        if registry::command_record(&self.directory, command_id, &command, false)? {
            let previous = registrations.get(&session_id).ok_or_else(|| {
                anyhow::anyhow!("start outcome unconfirmed; retained admission prevents replay")
                    .context(routing::OutcomeUnknown)
            })?;
            return Ok(serde_json::to_value(
                routing::inspect(&registry::directory(&self.directory, session_id), previous).await,
            )?);
        }
        if let Some(previous) = registrations
            .values()
            .find(|item| item.command_id == command_id)
        {
            ensure!(
                previous.session_id == session_id
                    && previous.workspace == workspace
                    && previous.config_path == config_path
                    && previous.initialize == initialize,
                "start command id payload conflict"
            );
        }
        if let Some(previous) = registrations.get(&session_id) {
            ensure!(
                previous.workspace == workspace
                    && previous.config_path == config_path
                    && previous.initialize == initialize,
                "session workspace conflict"
            );
            registry::command_record(&self.directory, command_id, &command, true)
                .map_err(|error| error.context(routing::OutcomeUnknown))?;
            return Ok(serde_json::to_value(
                routing::inspect(&registry::directory(&self.directory, session_id), previous).await,
            )?);
        }
        ensure!(
            registrations.len() < 4096,
            "supervisor registration retention limit reached"
        );
        for registration in registrations.values_mut() {
            let directory = registry::directory(&self.directory, registration.session_id);
            if registration.state != ProcessState::Relinquished
                && !directory.join("runtime.sock").exists()
                && (super::recovery::clean_stop(&directory, registration)
                    || super::recover_command::restart_permitted(&directory, registration))
            {
                registration.state = ProcessState::Stopped;
            }
        }
        self.check_transfer_retention()?;
        let directory = registry::directory(&self.directory, session_id);
        registry::private_directory(&directory)?;
        let registration = ProcessRegistration {
            protocol: PROCESS_PROTOCOL,
            session_id,
            incarnation: Uuid::new_v4(),
            command_id,
            restart_from: None,
            initialize,
            config_path,
            token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            workspace,
            state: ProcessState::Starting,
        };
        registry::command_record(&self.directory, command_id, &command, true)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registry::save(&directory, &registration)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registrations.insert(session_id, registration.clone());
        drop(registrations);
        super::launch::launch(&self.binary, &directory, &registration)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        let observed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let info = routing::inspect(&directory, &registration).await;
                if info.state == ProcessState::Live {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if let Ok(info) = observed {
            return Ok(serde_json::to_value(info)?);
        }
        Err(anyhow::anyhow!(
            "runtime startup unconfirmed; retained registration prevents duplicate launch"
        )
        .context(routing::OutcomeUnknown))
    }
}
