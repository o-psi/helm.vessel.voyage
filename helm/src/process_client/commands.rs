use super::{cli::ConnectedCommand, transport::Client};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, VesselCommand};

pub(super) async fn execute(
    client: &Client,
    command: ConnectedCommand,
) -> Result<serde_json::Value> {
    let (session, command) = match command {
        ConnectedCommand::Export { session, path } => {
            return super::export::markdown(client, session, &path).await;
        }
        ConnectedCommand::Admin { command } => return super::admin::execute(client, command).await,
        ConnectedCommand::Run { .. }
        | ConnectedCommand::Chat { .. }
        | ConnectedCommand::Terminal { .. } => {
            unreachable!("streaming frontend handled separately")
        }
        ConnectedCommand::List => return client.request(VesselCommand::Catalogue).await,
        ConnectedCommand::New {
            id,
            command_id,
            workspace,
            config_path,
        } => {
            let workspace = if client.is_local() {
                workspace.canonicalize()?
            } else {
                workspace
            };
            let command_id = command_id.unwrap_or_else(Uuid::new_v4);
            let session_id = id.unwrap_or_else(Uuid::new_v4);
            eprintln!(
                "Starting voyage {session_id} · command {command_id}; inspect this identity if delivery is lost"
            );
            let command = match config_path {
                Some(config_path) => VesselCommand::StartConfigured {
                    command_id,
                    session_id,
                    workspace,
                    config_path,
                },
                None => VesselCommand::Start {
                    command_id,
                    session_id,
                    workspace,
                },
            };
            return client.request(command).await;
        }
        ConnectedCommand::Import {
            session,
            command_id,
            workspace,
            source_directory,
            expected_revision,
            source_sha256,
            config_path,
        } => {
            return client
                .request(VesselCommand::Import {
                    command_id,
                    session_id: session,
                    workspace,
                    source_directory,
                    expected_revision,
                    source_sha256,
                    config_path,
                })
                .await;
        }
        ConnectedCommand::Branch {
            session,
            branch_id,
            command_id,
            expected_revision,
            expires_at_ms,
            name,
        } => {
            let process: ProcessInfo = serde_json::from_value(
                client
                    .request(VesselCommand::Inspect {
                        session_id: session,
                    })
                    .await?,
            )?;
            return client
                .request(VesselCommand::Branch {
                    command_id,
                    session_id: session,
                    incarnation: process.incarnation,
                    expected_revision,
                    expires_at_ms,
                    branch_id,
                    name,
                })
                .await;
        }
        ConnectedCommand::Archive {
            session,
            restore,
            command_id,
            expected_revision,
            expires_at_ms,
        } => (
            session,
            RuntimeCommand::Archive {
                command_id,
                expected_revision,
                expires_at_ms,
                archived: !restore,
            },
        ),
        ConnectedCommand::Delete {
            session,
            confirm_session_id,
            command_id,
            expected_revision,
            expires_at_ms,
        } => (
            session,
            RuntimeCommand::Delete {
                command_id,
                expected_revision,
                expires_at_ms,
                confirm_session_id,
            },
        ),
        ConnectedCommand::Inspect { session } => (session, RuntimeCommand::Snapshot),
        ConnectedCommand::Submit {
            session,
            expected_revision,
            command_id,
            expires_at_ms,
            prompt,
        } => (
            session,
            RuntimeCommand::Submit {
                command_id,
                expected_revision,
                expires_at_ms,
                prompt: prompt.join(" "),
            },
        ),
        ConnectedCommand::Receipt {
            session,
            command_id,
        } => (session, RuntimeCommand::Receipt { command_id }),
        ConnectedCommand::Cancel {
            session,
            run,
            expected_revision,
            command_id,
            expires_at_ms,
        } => (
            session,
            RuntimeCommand::Cancel {
                command_id,
                expected_revision,
                expires_at_ms,
                run_id: run,
            },
        ),
        ConnectedCommand::Stop {
            session,
            incarnation,
        } => {
            return client
                .request(VesselCommand::Stop {
                    session_id: session,
                    incarnation,
                })
                .await;
        }
        ConnectedCommand::Restart {
            session,
            incarnation,
            command_id,
        } => {
            return client
                .request(VesselCommand::Restart {
                    command_id,
                    session_id: session,
                    incarnation,
                })
                .await;
        }
        ConnectedCommand::Request { session, request } => {
            ensure!(
                request.len() <= 256 * 1024,
                "command JSON exceeds client limit"
            );
            (
                session,
                serde_json::from_str(&request).context("invalid typed runtime command")?,
            )
        }
    };
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect {
                session_id: session,
            })
            .await?,
    )?;
    client.forward(session, process.incarnation, command).await
}
