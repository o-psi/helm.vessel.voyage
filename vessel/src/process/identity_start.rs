//! Exact ordinary starts capture account settings under the selected identity.
//! Root retains only request/binding authority, never the private configuration.
use super::{accounts::Scope, database, service::Supervisor};
use anyhow::{Result, ensure};
use sha2::{Digest, Sha256};
use voyage_protocol::{identity_helper::IdentityHelperOperation, process::*};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchPin {
    schema: u32,
    command_id: uuid::Uuid,
    session_id: uuid::Uuid,
    config_path: std::path::PathBuf,
    config_digest: String,
}

pub(super) fn pin_launch(
    root: &std::path::Path,
    command: uuid::Uuid,
    session: uuid::Uuid,
    path: &std::path::Path,
    digest: &str,
) -> Result<()> {
    ensure!(
        !command.is_nil()
            && !session.is_nil()
            && path.is_absolute()
            && digest.len() == 64
            && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid protected launch pin"
    );
    let control = voyage_storage::protected_linux::RootDirectory::open(root)?
        .create_child("launch-pins".as_ref())?;
    let pin = LaunchPin {
        schema: 1,
        command_id: command,
        session_id: session,
        config_path: path.to_owned(),
        config_digest: digest.to_owned(),
    };
    let bytes = serde_json::to_vec(&pin)?;
    let name = format!("{session}-{command}.json");
    match control.read(name.as_ref(), 4096) {
        Ok(prior) => ensure!(prior == bytes, "protected launch pin changed"),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            control.publish_new(name.as_ref(), &bytes, 4096)?
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

pub(super) fn launch_digest(
    root: &std::path::Path,
    registration: &ProcessRegistration,
) -> Result<Option<String>> {
    let result = (|| {
        let control = voyage_storage::protected_linux::RootDirectory::open(root)?
            .child("launch-pins".as_ref())?;
        control.read(
            format!(
                "{}-{}.json",
                registration.session_id, registration.command_id
            )
            .as_ref(),
            4096,
        )
    })();
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let pin: LaunchPin = serde_json::from_slice(&bytes)?;
    ensure!(
        pin.schema == 1
            && pin.command_id == registration.command_id
            && pin.session_id == registration.session_id
            && registration.config_path.as_ref() == Some(&pin.config_path)
            && pin.config_digest.len() == 64
            && pin.config_digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "protected launch pin mismatch"
    );
    Ok(Some(pin.config_digest))
}

impl Supervisor {
    pub(super) async fn start_identity_request(
        &self,
        command: VesselCommand,
        scope: Scope,
    ) -> Result<serde_json::Value> {
        let (command_id, session_id, workspace, base, account, settings, resolve) = match &command {
            VesselCommand::Start {
                command_id,
                session_id,
                workspace,
            } => (
                *command_id,
                *session_id,
                workspace.clone(),
                None,
                None,
                voyage_protocol::start_settings::StartSettings::default(),
                false,
            ),
            VesselCommand::StartSettings {
                command_id,
                session_id,
                workspace,
                config_path,
                settings,
                binding,
            } => (
                *command_id,
                *session_id,
                workspace.clone(),
                config_path.clone(),
                binding.clone(),
                settings.clone(),
                false,
            ),
            VesselCommand::StartAccount {
                command_id,
                session_id,
                workspace,
                config_path,
                account,
                model,
                reasoning_effort,
                service_tier,
            }
            | VesselCommand::ResolveStartAccount {
                command_id,
                session_id,
                workspace,
                config_path,
                account,
                model,
                reasoning_effort,
                service_tier,
            } => (
                *command_id,
                *session_id,
                workspace.clone(),
                config_path.clone(),
                Some(account.clone()),
                voyage_protocol::start_settings::StartSettings {
                    model: Some(model.clone()),
                    reasoning_effort: Some(reasoning_effort.clone()),
                    service_tier: Some(service_tier.clone()),
                    ..Default::default()
                },
                matches!(command, VesselCommand::ResolveStartAccount { .. }),
            ),
            _ => anyhow::bail!("unsupported ordinary identity start"),
        };
        ensure!(
            !command_id.is_nil() && !session_id.is_nil(),
            "nil creation identity"
        );
        let right = if matches!(scope, Scope::Session(_)) {
            ProcessRight::Lifecycle
        } else {
            ProcessRight::Create
        };
        scope.check(&self.directory, &workspace, right)?;
        ensure!(
            base.is_none() || matches!(scope, Scope::Owner),
            "configured account creation requires owner-local authority"
        );
        if let Scope::Session(grant) = &scope {
            ensure!(
                grant.session_id == session_id,
                "session creation scope denied"
            );
        }
        let original = if resolve {
            VesselCommand::StartAccount {
                command_id,
                session_id,
                workspace: workspace.clone(),
                config_path: base.clone(),
                account: account
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("account required"))?,
                model: settings
                    .model
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("model required"))?,
                reasoning_effort: settings.reasoning_effort.clone().flatten(),
                service_tier: settings.service_tier.clone().flatten(),
            }
        } else {
            command
        };
        let directory =
            super::runtime_storage::planned_bound_directory(&self.directory, session_id)?;
        let config_path = directory.join("launch.json");
        let lock = self.bound_creation_lock(session_id).await?;
        let _guard = lock.lock().await;
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
        ensure!(
            !super::start::resolution_record(
                &self.directory,
                "intent",
                command_id,
                &original,
                false
            )
            .await?,
            "start command was fenced as not admitted"
        );
        if super::registry::command_record(&self.directory, command_id, &original, false).await? {
            let binding = database::execution_binding(&self.directory, session_id)
                .await?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "bound creation outcome unconfirmed; inspect retained admission"
                    )
                    .context(super::routing::OutcomeUnknown)
                })?;
            return self
                .start_bound_request_locked(
                    command_id,
                    session_id,
                    workspace,
                    config_path,
                    binding,
                    original,
                )
                .await;
        }
        ensure!(
            !self.registrations.lock().await?.contains_key(&session_id),
            "voyage already reserved"
        );
        let identity = super::identity_accounts::selected(&scope, &self.directory).await?;
        ensure!(
            identity.uid != 0,
            "ordinary creation cannot select administrator identity"
        );
        let control = voyage_storage::protected_linux::RootDirectory::open(&self.directory)?
            .create_child("account-launch".as_ref())?;
        let request = serde_json::to_vec(
            &serde_json::json!({"schema":1,"actor":scope.actor(&workspace),"command":original,"identity":identity.identity,"account_context":identity.account_context}),
        )?;
        ensure!(
            request.len() <= 65_536,
            "identity creation intent exceeds bounds"
        );
        let entry = format!("{command_id}.json");
        match control.read(entry.as_ref(), 65_536) {
            Ok(prior) => ensure!(prior == request, "identity creation intent changed"),
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                ensure!(
                    std::fs::read_dir(self.directory.join("account-launch"))?
                        .take(4097)
                        .count()
                        < 4096,
                    "identity creation intent capacity reached"
                );
                control.publish_new(entry.as_ref(), &request, 65_536)?;
            }
            Err(error) => return Err(error),
        }
        let digest = format!("{:x}", Sha256::digest(&request));
        let directory = super::runtime_storage::provision_bound_directory(
            &self.directory,
            session_id,
            identity.uid,
            identity.gid,
        )?;
        let projection = self.identity_account_scope(&scope, &workspace)?;
        let captured = self
            .identity_account_helper(
                &scope,
                &workspace,
                right,
                IdentityHelperOperation::CaptureLaunch {
                    scope: projection,
                    directory,
                    request_digest: digest,
                    base_config_path: base,
                    account,
                    settings,
                },
            )
            .await?;
        let captured_path: std::path::PathBuf =
            serde_json::from_value(captured["config_path"].clone())?;
        ensure!(
            captured_path == config_path
                && super::identity_accounts::selected(&scope, &self.directory).await? == identity,
            "ordinary launch context changed"
        );
        pin_launch(
            &self.directory,
            command_id,
            session_id,
            &config_path,
            captured["config_digest"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("captured launch digest missing"))?,
        )?;
        scope.check(&self.directory, &workspace, right)?;
        if let Scope::Connection(grant) = &scope {
            self.connection_session(grant, session_id, &workspace)?;
        }
        let binding = super::default_execution::binding(&identity, session_id)?;
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
}
