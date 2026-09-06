//! Explicit account-authorized administration with reviewable, resumable identities.
mod args;
mod transfer;
use super::transport::Client;
use anyhow::{Result, ensure};
pub use args::AdminCommand;
use serde_json::Value;
use voyage_protocol::process::*;

pub(super) async fn execute(client: &Client, command: AdminCommand) -> Result<Value> {
    let command = match command {
        AdminCommand::Identity => VesselCommand::Identity,
        AdminCommand::Trust { identity_file } => VesselCommand::TrustVessel {
            identity: read_json(&identity_file)?,
        },
        AdminCommand::Move(args) => return transfer::run(client, args).await,
        AdminCommand::AcceptParticipant {
            binding_file,
            command_id,
        } => VesselCommand::AcceptParticipant {
            command_id,
            binding: read_json(&binding_file)?,
        },
        AdminCommand::RemoveParticipant {
            binding_id,
            command_id,
            expected_revision,
            cancel,
        } => VesselCommand::RemoveParticipant {
            binding_id,
            command_id,
            expected_revision,
            cancel,
        },
        AdminCommand::Recover {
            session,
            incarnation,
            command_id,
            acknowledge_cleanup,
            reconcile_tools,
            expected_revision,
            acknowledge_resources,
        } => VesselCommand::Recover {
            session_id: session,
            incarnation,
            command_id,
            acknowledge_cleanup,
            reconcile_tools,
            expected_revision,
            acknowledge_resources,
        },
        AdminCommand::Assignment {
            session,
            run,
            assignment,
            participant,
            cancel,
        } => {
            let info: ProcessInfo = serde_json::from_value(
                client
                    .request(VesselCommand::Inspect {
                        session_id: session,
                    })
                    .await?,
            )?;
            return client
                .forward(
                    session,
                    info.incarnation,
                    RuntimeCommand::AssignmentObserve {
                        run_id: run,
                        assignment_id: assignment,
                        participant,
                        cancel,
                    },
                )
                .await;
        }
    };
    client.request(command).await
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= 65536,
        "administration input exceeds limit"
    );
    let mut bytes = Vec::new();
    file.by_ref().take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "administration input exceeds limit");
    Ok(serde_json::from_slice(&bytes)?)
}
