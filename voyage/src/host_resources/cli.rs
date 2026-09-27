use anyhow::{Result, ensure};
#[derive(clap::Args)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(clap::Subcommand)]
pub enum Command {
    Inspect,
    /// Attest cleanup only after independently inspecting descendants of a dead owner.
    Attest {
        reservation: uuid::Uuid,
        #[arg(long)]
        confirm: uuid::Uuid,
        #[arg(long)]
        reason: String,
    },
    /// Reclaim a suspended session's stale browser slots after observing cleanup.
    BrowserCapacity {
        session: uuid::Uuid,
        #[arg(long)]
        session_dir: std::path::PathBuf,
        #[arg(long)]
        confirm: uuid::Uuid,
        #[arg(long)]
        observed_no_descendants: bool,
        #[arg(long)]
        reason: String,
    },
}
pub fn run(args: Args) -> Result<()> {
    let value = match args.command {
        Command::Inspect => super::inspect()?,
        Command::Attest {
            reservation,
            confirm,
            reason,
        } => {
            ensure!(
                reservation == confirm,
                "confirmation must match the exact reservation"
            );
            super::attest(reservation, &reason)?
        }
        Command::BrowserCapacity {
            session,
            session_dir,
            confirm,
            observed_no_descendants,
            reason,
        } => {
            ensure!(
                session == confirm,
                "confirmation must match the exact session"
            );
            crate::host_browser_capacity::recover(
                &session_dir,
                session,
                observed_no_descendants,
                &reason,
            )?
        }
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
