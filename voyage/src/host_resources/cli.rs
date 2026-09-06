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
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}
