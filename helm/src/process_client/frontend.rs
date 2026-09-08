//! Ordinary local frontends use the same Vessel surface as explicit connections.
pub mod github;
mod github_args;
pub(crate) mod launch;
pub mod models;
mod resume;
pub mod workflow;
use super::{local, transport::Client};
use anyhow::{Result, ensure};
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, VesselCommand, VoyageCommand};

pub async fn open(
    config: &crate::Config,
    workspace: Option<PathBuf>,
    reference: Option<String>,
    model_overridden: bool,
    configuration_explicit: bool,
) -> Result<(Client, ProcessInfo)> {
    open_inner(
        config,
        workspace,
        reference,
        model_overridden,
        configuration_explicit,
        true,
    )
    .await
}
async fn open_inner(
    config: &crate::Config,
    workspace: Option<PathBuf>,
    reference: Option<String>,
    model_overridden: bool,
    configuration_explicit: bool,
    announce: bool,
) -> Result<(Client, ProcessInfo)> {
    let client = local::connect(super::cli::default_directory(), true).await?;
    let resuming = reference.is_some();
    let process = match reference {
        Some(reference) => resume::open(&client, config, workspace, &reference).await?,
        None => {
            let workspace = config.resolve_workspace(workspace)?;
            let config_path = launch::persist(config, &workspace, &client.directory)?;
            let session_id = Uuid::new_v4();
            let command_id = Uuid::new_v4();
            if announce {
                eprintln!("Creating voyage {session_id} · command {command_id}");
            }
            serde_json::from_value(
                client
                    .request(VesselCommand::StartConfigured {
                        command_id,
                        session_id,
                        workspace,
                        config_path,
                    })
                    .await?,
            )?
        }
    };
    if resuming && (configuration_explicit || model_overridden) {
        let snapshot = client
            .voyage(
                process.session_id,
                process.incarnation,
                VoyageCommand::Snapshot,
            )
            .await?;
        let active = matches!(
            snapshot["run"]["state"].as_str(),
            Some("accepted" | "running" | "awaiting_decision" | "cancel_requested")
        );
        if active {
            ensure!(
                !configuration_explicit && !model_overridden,
                "configuration change requires idle voyage; reconnect without overrides or cancel the exact active run first"
            );
        } else {
            let mut config = config.clone();
            if !model_overridden {
                config.model = snapshot["model"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("snapshot model missing"))?
                    .to_owned();
            }
            let config_path = launch::persist(&config, &process.workspace, &client.directory)?;
            let command_id = Uuid::new_v4();
            let expected_revision = snapshot["revision"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("snapshot revision missing"))?;
            if announce {
                eprintln!("Configuration command {command_id}");
            }
            client
                .voyage(
                    process.session_id,
                    process.incarnation,
                    VoyageCommand::Configure {
                        command_id,
                        expected_revision,
                        expires_at_ms: deadline()?,
                        config_path,
                    },
                )
                .await?;
        }
    }
    Ok((client, process))
}

pub async fn run(
    config: crate::Config,
    workspace: Option<PathBuf>,
    reference: Option<String>,
    prompt: String,
    no_save: bool,
    model_overridden: bool,
    configuration_explicit: bool,
) -> Result<()> {
    let was_resume = reference.is_some();
    let (client, mut process) = open(
        &config,
        workspace,
        reference,
        model_overridden && !no_save,
        configuration_explicit && !no_save,
    )
    .await?;
    if no_save && was_resume {
        let snapshot = client
            .voyage(
                process.session_id,
                process.incarnation,
                VoyageCommand::Snapshot,
            )
            .await?;
        process = serde_json::from_value(
            client
                .request(VesselCommand::Branch {
                    command_id: Uuid::new_v4(),
                    session_id: process.session_id,
                    incarnation: process.incarnation,
                    expected_revision: snapshot["revision"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("snapshot revision missing"))?,
                    expires_at_ms: deadline()?,
                    branch_id: Uuid::new_v4(),
                    name: None,
                })
                .await?,
        )?;
    }
    let result = async {
        if no_save && was_resume && (model_overridden || configuration_explicit) {
            let mut selected = config.clone();
            let snapshot = client
                .voyage(
                    process.session_id,
                    process.incarnation,
                    VoyageCommand::Snapshot,
                )
                .await?;
            if !model_overridden {
                selected.model = snapshot["model"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("snapshot model missing"))?
                    .into();
            }
            let config_path = launch::persist(&selected, &process.workspace, &client.directory)?;
            client
                .voyage(
                    process.session_id,
                    process.incarnation,
                    VoyageCommand::Configure {
                        command_id: Uuid::new_v4(),
                        expected_revision: snapshot["revision"]
                            .as_u64()
                            .ok_or_else(|| anyhow::anyhow!("snapshot revision missing"))?,
                        expires_at_ms: deadline()?,
                        config_path,
                    },
                )
                .await?;
        }
        super::plain::run(&client, process.session_id, None, prompt).await
    }
    .await;
    if no_save {
        discard(&client, &process).await?;
    }
    result
}

pub async fn chat(
    mut config: crate::Config,
    workspace: Option<PathBuf>,
    reference: Option<String>,
    plain: bool,
    model_overridden: bool,
    configuration_explicit: bool,
) -> Result<()> {
    if reference.is_none() {
        config.workspace = Some(config.resolve_workspace(workspace)?);
        let client = local::connect(super::cli::default_directory(), true).await?;
        if plain {
            return super::plain::chat_new(&client, config).await;
        }
        return super::ui::run_with_config(vec![client], None, Some(config)).await;
    }
    let creating = reference.is_none();
    let opened = open(
        &config,
        workspace,
        reference,
        model_overridden,
        configuration_explicit,
    )
    .await;
    let (client, process) = match opened {
        Ok(value) => value,
        Err(error)
            if !plain
                && creating
                && error.downcast_ref::<super::transport::Refusal>().is_some() =>
        {
            let client = local::connect(super::cli::default_directory(), true).await?;
            return super::ui::run_with_notice(vec![client], None, Some(config), Some(format!("New voyage was refused: {error}. Select an existing voyage; /archive frees a slot after cleanup."))).await;
        }
        Err(error) => return Err(error),
    };
    if plain {
        super::plain::chat(&client, process.session_id).await
    } else {
        super::ui::run_with_config(vec![client], Some(process.session_id), Some(config)).await
    }
}

pub(super) fn deadline() -> Result<u64> {
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?;
    ensure!(now < u64::MAX - 60000, "clock out of range");
    Ok(now + 60000)
}

async fn discard(client: &Client, process: &ProcessInfo) -> Result<()> {
    let snapshot = client
        .voyage(
            process.session_id,
            process.incarnation,
            VoyageCommand::Snapshot,
        )
        .await?;
    let active = matches!(
        snapshot["run"]["state"].as_str(),
        Some("accepted" | "running" | "awaiting_decision" | "cancel_requested")
    );
    if active || !snapshot["pending_cleanup_run"].is_null() {
        anyhow::bail!(
            "Temporary voyage {} retained until runtime work and cleanup finish; no-save cleanup remains pending",
            process.session_id
        );
    } else {
        let expected_revision = snapshot["revision"]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("snapshot revision missing"))?;
        let command_id = Uuid::new_v4();
        let result = client
            .voyage(
                process.session_id,
                process.incarnation,
                VoyageCommand::Delete {
                    command_id,
                    expected_revision,
                    expires_at_ms: deadline()?,
                    confirm_session_id: process.session_id,
                },
            )
            .await;
        if let Err(error) = &result
            && error.downcast_ref::<super::transport::Refusal>().is_some()
        {
            return result.map(|_| ());
        }
        // Delete already shuts the runtime down. Sending Stop races that shutdown
        // and can turn a successful discovery into an uncertain cleanup error.
        // Observe the exact durable deletion and clean stop instead; never replay
        // either an uncertain deletion or a second lifecycle mutation.
        let observed = tokio::time::timeout(std::time::Duration::from_secs(20), async {
            loop {
                if let Ok(value) = client
                    .request(VesselCommand::Inspect {
                        session_id: process.session_id,
                    })
                    .await
                    && let Ok(info) = serde_json::from_value::<ProcessInfo>(value)
                    && info.session_id == process.session_id
                    && info.incarnation == process.incarnation
                    && info.state == voyage_protocol::vessel::ProcessState::Stopped
                    && info.deletion.as_ref().is_some_and(|receipt| {
                        receipt["command_id"].as_str() == Some(command_id.to_string().as_str())
                            && receipt["status"] == "applied"
                            && receipt["deleted"] == true
                            && receipt["cleanup"] == "observed"
                    })
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await;
        ensure!(
            observed.is_ok(),
            "Temporary voyage {} deletion {} remains unconfirmed; inspect its durable outcome before retrying",
            process.session_id,
            command_id
        );
    }
    Ok(())
}

pub fn persist_launch(
    config: &crate::Config,
    workspace: &std::path::Path,
    directory: &std::path::Path,
) -> Result<PathBuf> {
    launch::persist(config, workspace, directory)
}
