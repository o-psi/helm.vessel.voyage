//! Connected frontends share the same Vessel requests as the multiplexer.
use super::{local, transport::Client};
use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, VesselCommand};

#[derive(Args)]
pub struct ConnectArgs {
    /// Private local Vessel state directory.
    #[arg(long)]
    pub directory: Option<PathBuf>,
    /// Explicit SSH account; remote authority is this account's local authority.
    #[arg(long, requires = "remote_directory")]
    pub ssh: Vec<String>,
    /// Absolute private Vessel directory on the SSH host.
    #[arg(long, requires = "ssh")]
    pub remote_directory: Vec<PathBuf>,
    /// Include the local Vessel alongside the SSH host.
    #[arg(long, requires = "ssh")]
    pub include_local: bool,
    /// Connect without starting an absent local Vessel.
    #[arg(long)]
    pub no_start: bool,
    #[command(subcommand)]
    pub command: Option<ConnectedCommand>,
}

#[derive(Subcommand)]
pub enum ConnectedCommand {
    /// List independently supervised voyages.
    List,
    /// Start a voyage on the selected host. UUIDs make create outcomes recoverable.
    New {
        #[arg(long)]
        id: Option<Uuid>,
        #[arg(long)]
        command_id: Option<Uuid>,
        #[arg(long)]
        workspace: PathBuf,
    },
    /// Read a current canonical projection without resubmitting work.
    Inspect { session: Uuid },
    /// Submit once against an observed revision; observe the exact receipt after loss.
    Submit {
        session: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        expires_at_ms: u64,
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    /// Retrieve a command's original durable receipt.
    Receipt { session: Uuid, command_id: Uuid },
    /// Cancel one exact run; intent is distinct from observed cleanup.
    Cancel {
        session: Uuid,
        #[arg(long)]
        run: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        expires_at_ms: u64,
    },
    /// Stop the selected runtime after explicit inspection of its incarnation.
    Stop {
        session: Uuid,
        #[arg(long)]
        incarnation: Uuid,
    },
    /// Restart only after the previous incarnation recorded observed cleanup.
    Restart {
        session: Uuid,
        #[arg(long)]
        incarnation: Uuid,
        #[arg(long)]
        command_id: Uuid,
    },
    /// Send one typed runtime command as JSON, including history, steering or decisions.
    Request { session: Uuid, request: String },
}

pub async fn run(args: ConnectArgs) -> Result<()> {
    let mut clients = Vec::new();
    ensure!(
        args.ssh.len() == args.remote_directory.len() && args.ssh.len() <= 16,
        "provide one --remote-directory per --ssh destination, with at most 16 remote Vessels"
    );
    if args.ssh.is_empty() || args.include_local {
        let directory = args.directory.unwrap_or_else(default_directory);
        clients.push(local::connect(directory, !args.no_start).await?);
    }
    for (ssh, directory) in args.ssh.into_iter().zip(args.remote_directory) {
        let client = Client {
            directory,
            ssh: Some(ssh),
        };
        client.request(VesselCommand::Capabilities).await?;
        clients.push(client);
    }
    if let Some(command) = args.command {
        ensure!(
            clients.len() == 1,
            "CLI operations require a single selected Vessel"
        );
        let value = execute(&clients[0], command).await?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        Ok(())
    } else {
        super::ui::run(clients).await
    }
}

pub fn default_directory() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state")
        })
        .join("voyage/vessel")
}

async fn execute(client: &Client, command: ConnectedCommand) -> Result<serde_json::Value> {
    let (session, command) = match command {
        ConnectedCommand::List => return client.request(VesselCommand::Catalogue).await,
        ConnectedCommand::New {
            id,
            command_id,
            workspace,
        } => {
            let workspace = if client.ssh.is_none() {
                workspace.canonicalize()?
            } else {
                workspace
            };
            return client
                .request(VesselCommand::Start {
                    command_id: command_id.unwrap_or_else(Uuid::new_v4),
                    session_id: id.unwrap_or_else(Uuid::new_v4),
                    workspace,
                })
                .await;
        }
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
