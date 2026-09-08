//! Connected frontends share the same Vessel requests as the multiplexer.
use super::{local, transport::Client};
use anyhow::{Result, ensure};
use clap::{Args, Subcommand};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Args)]
pub struct ConnectArgs {
    /// Private local Vessel state directory containing HTTP discovery credentials.
    #[arg(long)]
    pub directory: Option<PathBuf>,
    /// Include the local Vessel alongside scoped HTTPS routes.
    #[arg(long)]
    pub include_local: bool,
    /// Connect without starting an absent local Vessel.
    #[arg(long)]
    pub no_start: bool,
    /// Private HTTPS credential for one explicitly granted remote voyage.
    #[arg(long)]
    pub access_file: Vec<PathBuf>,
    #[command(subcommand)]
    pub command: Option<ConnectedCommand>,
}

#[derive(Subcommand)]
pub enum ConnectedCommand {
    /// Export complete public history at one revision to a new local Markdown file.
    Export { session: Uuid, path: PathBuf },
    /// Explicit account administration, participant bindings and signed owner moves.
    Admin {
        #[command(subcommand)]
        command: super::admin::AdminCommand,
    },
    /// Attach a separate private human-input channel to a runtime terminal.
    Terminal {
        session: Uuid,
        #[arg(long)]
        run: Uuid,
        #[arg(long)]
        terminal: Uuid,
    },
    /// Run a turn in an existing independent voyage and stream its durable output.
    Run {
        session: Uuid,
        #[arg(long)]
        command_id: Option<Uuid>,
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    /// Use a line-oriented client; EOF or Ctrl+C detaches without cancelling work.
    Chat { session: Uuid },
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
        /// Absolute execution-host configuration path.
        #[arg(long)]
        config_path: Option<PathBuf>,
    },
    /// Migrate a fenced ordinary session with its exact UUID and source fingerprint.
    Import {
        session: Uuid,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        workspace: PathBuf,
        #[arg(long)]
        source_directory: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        source_sha256: String,
        #[arg(long)]
        config_path: Option<PathBuf>,
    },
    /// Fork an idle voyage's canonical history into a new independent owner.
    Branch {
        session: Uuid,
        #[arg(long)]
        branch_id: Uuid,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expires_at_ms: u64,
        #[arg(long)]
        name: Option<String>,
    },
    /// Archive or restore an idle voyage.
    Archive {
        session: Uuid,
        #[arg(long)]
        restore: bool,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expires_at_ms: u64,
    },
    /// Purge canonical history after exact session confirmation; retain a tombstone.
    Delete {
        session: Uuid,
        #[arg(long)]
        confirm_session_id: Uuid,
        #[arg(long)]
        command_id: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        expires_at_ms: u64,
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
    /// Send one typed public voyage operation as JSON, including history, steering or decisions.
    Request { session: Uuid, request: String },
}

pub async fn run(args: ConnectArgs) -> Result<()> {
    let mut clients = Vec::new();
    if args.access_file.is_empty() || args.include_local {
        let directory = args.directory.unwrap_or_else(default_directory);
        if args.command.is_some() {
            clients.push(local::connect(directory, !args.no_start).await?);
        } else {
            let client = Client {
                directory: directory.clone(),
                access_file: None,
            };
            clients.push(client);
            if !args.no_start {
                tokio::spawn(async move {
                    let _ = local::connect(directory, true).await;
                });
            }
        }
    }
    ensure!(args.access_file.len() <= 16, "at most 16 grant routes");
    for path in args.access_file {
        clients.push(Client {
            directory: PathBuf::new(),
            access_file: Some(path),
        });
    }
    if let Some(command) = args.command {
        ensure!(
            clients.len() == 1,
            "CLI operations require a single selected Vessel"
        );
        match command {
            ConnectedCommand::Terminal {
                session,
                run,
                terminal,
            } => return super::terminal::attach(&clients[0], session, run, terminal).await,
            ConnectedCommand::Run {
                session,
                command_id,
                prompt,
            } => {
                return super::plain::run(&clients[0], session, command_id, prompt.join(" ")).await;
            }
            ConnectedCommand::Chat { session } => {
                return super::plain::chat(&clients[0], session).await;
            }
            command => {
                let value = super::commands::execute(&clients[0], command).await?;
                println!("{}", serde_json::to_string_pretty(&value)?);
                Ok(())
            }
        }
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

impl ConnectedCommand {
    /// Clap propagates the shared workspace option ID to the global parser.
    pub fn uses_host_workspace(&self) -> bool {
        matches!(
            self,
            Self::New { .. }
                | Self::Import { .. }
                | Self::Admin {
                    command: super::admin::AdminCommand::Move(_)
                }
        )
    }
}
