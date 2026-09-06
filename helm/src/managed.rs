//! Explicit local managed-session frontend. No legacy SessionStore is reachable.
use super::*;
use anyhow::ensure;
use clap::Subcommand;
use helm::attachment::journal::Journal;
use serde_json::{Value, json};

use uuid::Uuid;

#[derive(clap::Args)]
pub(super) struct Args {
    /// Dedicated absolute installation directory (alternate roots use distinct local identities).
    #[arg(long)]
    directory: PathBuf,
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    command: ManagedCommand,
}
#[derive(Subcommand)]
enum ManagedCommand {
    /// Create once. Use list to recover a lost response; creation never retries implicitly.
    Create {
        #[arg(long)]
        id: Option<Uuid>,
        #[arg(long)]
        name: Option<String>,
    },
    /// List bounded session metadata, excluding conversation and provider state.
    List {
        #[arg(long)]
        after: Option<Uuid>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Submit one turn against an explicit saved revision and observe its output.
    Submit {
        session: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, requires = "expires_at_ms")]
        command_id: Option<Uuid>,
        #[arg(long, requires = "command_id")]
        expires_at_ms: Option<i64>,
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    /// Request cancellation of one exact run; requested does not mean stopped.
    Cancel {
        session: Uuid,
        #[arg(long)]
        run: Uuid,
    },
    /// Mark abandoned work interrupted without replaying tools.
    Recover {
        session: Uuid,
        /// Attest that you independently stopped this run's previous effects.
        #[arg(long)]
        acknowledge_cleanup: Option<Uuid>,
        /// Attest independent cleanup of an exact retained session resource.
        #[arg(long = "acknowledge-resource")]
        acknowledge_resources: Vec<Uuid>,
        /// Append unknown/interrupted results after terminal cleanup; never replay tools.
        #[arg(long, requires = "expected_revision")]
        reconcile_tools: Option<Uuid>,
        #[arg(long, requires = "reconcile_tools")]
        expected_revision: Option<u64>,
    },
    /// Explicitly upgrade a quiescent journal. Stop older Helm processes first.
    Upgrade,
}
impl Args {
    pub(super) fn administrative(&self) -> bool {
        !matches!(
            self.command,
            ManagedCommand::Create { .. } | ManagedCommand::Submit { .. }
        )
    }
}

mod catalogue;
mod commands;
mod connection;
pub(crate) mod maintenance;

pub(super) fn safe_error(error: anyhow::Error) -> anyhow::Error {
    anyhow::anyhow!(crate::safe_diagnostic(&error.to_string()))
}
pub(super) async fn run(
    args: Args,
    config: Option<Config>,
    workspace: Option<PathBuf>,
    model_overridden: bool,
) -> Result<()> {
    commands::run(args, config, workspace, model_overridden).await
}
