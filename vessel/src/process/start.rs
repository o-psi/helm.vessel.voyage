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
        let mut registrations = self.registrations.lock().await?;
        ensure!(
            !resolution_record(&self.directory, "intent", command_id, &command, false).await?,
            "start command was fenced as not admitted"
        );
        if registry::command_record(&self.directory, command_id, &command, false).await? {
            if let Some(info) =
                super::database::creation_receipt(&self.directory, command_id).await?
            {
                return Ok(serde_json::to_value(info)?);
            }
            let previous = registrations.get(&session_id).ok_or_else(|| {
                anyhow::anyhow!("start outcome unconfirmed; retained admission prevents replay")
                    .context(routing::OutcomeUnknown)
            })?;
            let previous = previous.clone();
            drop(registrations);
            return Ok(serde_json::to_value(
                routing::inspect(&registry::directory(&self.directory, session_id), &previous)
                    .await,
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
                .await
                .map_err(|error| error.context(routing::OutcomeUnknown))?;
            let previous = previous.clone();
            drop(registrations);
            return Ok(serde_json::to_value(
                routing::inspect(&registry::directory(&self.directory, session_id), &previous)
                    .await,
            )?);
        }
        if initialize.is_none() {
            let (_, default) =
                voyage_runtime::accounts::Registry::default_host()?.default_account()?;
            ensure!(
                default.is_some(),
                "default_account_required: choose a default account before creating a voyage"
            );
        }
        ensure!(
            registrations.len() < 4096,
            "supervisor registration retention limit reached"
        );
        for registration in registrations.values_mut() {
            let directory = registry::directory(&self.directory, registration.session_id);
            if registration.state != ProcessState::Relinquished
                && !super::recovery::suspended(&directory, registration)
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
            executable: Some(self.binary.clone()),
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
            name: None,
        };
        super::database::admit(
            &self.directory,
            &registration,
            serde_json::to_vec(&command)?,
        )
        .await
        .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registry::publish(&directory, &registration)
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
        if let Ok(mut info) = observed {
            let _ = super::database::refresh_now(&self.directory, &registration).await;
            if let Some(cached) = super::database::catalogue(&self.directory)
                .await
                .map_err(|error| error.context(routing::OutcomeUnknown))?
                .into_iter()
                .find(|p| p.session_id == session_id)
            {
                info.catalogue = cached.catalogue;
            }
            super::database::settle_creation(&self.directory, command_id, &info)
                .await
                .map_err(|error| error.context(routing::OutcomeUnknown))?;
            return Ok(serde_json::to_value(info)?);
        }
        Err(anyhow::anyhow!(
            "runtime startup unconfirmed; retained registration prevents duplicate launch"
        )
        .context(routing::OutcomeUnknown))
    }
}

// Separate private intent namespace: publication precedes the generic reservation.
// A crash in that gap must never permit the delayed Start to launch. Completed
// fences are retained indefinitely with bounded count/size, like lifecycle IDs.
async fn resolution_record(
    root: &std::path::Path,
    phase: &str,
    id: Uuid,
    original: &VesselCommand,
    reserve: bool,
) -> Result<bool> {
    super::database::command(
        root,
        &format!("start-resolution/{phase}"),
        id,
        serde_json::to_vec(original)?,
        reserve,
    )
    .await
}

impl Supervisor {
    pub(super) async fn resolve_start(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
    ) -> Result<Value> {
        ensure!(
            !command_id.is_nil() && !session_id.is_nil(),
            "nil creation identity"
        );
        // Preserve the lexical payload; do not require an old config file to
        // still exist just to resolve its admission.
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
        self.resolve_start_original(command_id, session_id, workspace, config_path, original)
            .await
    }
    pub(super) async fn resolve_start_original(
        &self,
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
        original: VesselCommand,
    ) -> Result<Value> {
        let registrations = self.registrations.lock().await?;
        let fenced =
            resolution_record(&self.directory, "intent", command_id, &original, false).await?;
        let admitted =
            registry::command_record(&self.directory, command_id, &original, false).await?;
        if fenced {
            registry::command_record(&self.directory, command_id, &original, true).await?;
            resolution_record(&self.directory, "not-admitted", command_id, &original, true).await?;
            return Ok(
                serde_json::json!({"status":"not_admitted", "command_id":command_id, "session_id":session_id}),
            );
        }
        if admitted {
            if let Some(process) =
                super::database::creation_receipt(&self.directory, command_id).await?
            {
                ensure!(
                    process.session_id == session_id,
                    "creation receipt identity conflict"
                );
                return Ok(
                    serde_json::json!({"status":"created", "command_id":command_id, "session_id":session_id, "process":process}),
                );
            }
            if let Some(registration) = registrations.get(&session_id).filter(|registration| {
                registration.command_id == command_id
                    && registration.config_path == config_path
                    && registration.initialize.is_none()
            }) {
                // The exact durable command establishes the lexical workspace;
                // a registration is usable only if its canonical workspace agrees.
                if std::fs::canonicalize(&workspace).ok().as_ref() == Some(&registration.workspace)
                {
                    let registration = registration.clone();
                    drop(registrations);
                    let process = routing::inspect(
                        &registry::directory(&self.directory, session_id),
                        &registration,
                    )
                    .await;
                    return Ok(
                        serde_json::json!({"status":"created", "command_id":command_id, "session_id":session_id, "process":process}),
                    );
                }
            }
            return Ok(
                serde_json::json!({"status":"unknown", "command_id":command_id, "session_id":session_id}),
            );
        }
        // Legacy registration-only admission cannot be turned into a fence.
        if let Some(registration) = registrations
            .values()
            .find(|registration| registration.command_id == command_id)
        {
            ensure!(
                registration.session_id == session_id
                    && registration.config_path == config_path
                    && registration.initialize.is_none(),
                "start command id payload conflict"
            );
            if let Ok(canonical) = std::fs::canonicalize(&workspace) {
                ensure!(
                    registration.workspace == canonical,
                    "start command id payload conflict"
                );
            }
            return Ok(
                serde_json::json!({"status":"unknown", "command_id":command_id, "session_id":session_id}),
            );
        }
        resolution_record(&self.directory, "intent", command_id, &original, true).await?;
        registry::command_record(&self.directory, command_id, &original, true).await?;
        resolution_record(&self.directory, "not-admitted", command_id, &original, true).await?;
        Ok(
            serde_json::json!({"status":"not_admitted", "command_id":command_id, "session_id":session_id}),
        )
    }
}
