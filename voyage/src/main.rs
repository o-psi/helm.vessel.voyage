use clap::{CommandFactory, Parser, Subcommand};
#[derive(Parser)]
#[command(version, about = "Independent voyage session runtime")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Read a bounded model-discovery request on stdin, without creating a session.
    DiscoverModels,

    /// Run one supervisor-registered session owner.
    Serve(voyage::server::ServeArgs),
    /// Guard one runtime and observe descendant cleanup after it exits (Linux).
    Supervise(voyage::server::ServeArgs),
    /// Read one suspended-session observation without starting an execution runtime.
    ObserveSuspended(voyage::server::suspended::Args),
    /// Inspect or reconcile an unavailable incarnation under its exclusive fence.
    Recover(voyage::server::recovery::RecoverArgs),
    /// Reconcile abandoned work in a legacy installation without replay.
    LegacyRecover(voyage::server::legacy_recovery::LegacyRecoverArgs),
    /// Upgrade an existing quiescent journal under its ownership guards.
    UpgradeJournal(voyage::server::bootstrap::UpgradeArgs),
    Completions {
        shell: clap_complete::Shell,
    },
    Manpage,
    /// Inspect retained host quota charges or explicitly attest cleanup.
    HostResources(voyage::host_resources::cli::Args),
    /// Validate host configuration and workspace without dispatching a model.
    ValidateStart(voyage::server::bootstrap::ValidateStartArgs),
    /// Inspect a private ordinary session and its exact migration fingerprint.
    ImportPlan(voyage::server::bootstrap::ImportPlanArgs),
}
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let command = match cli.command {
        Command::Supervise(args) => return voyage::server::guardian::run(args),
        command => command,
    };
    tokio::runtime::Runtime::new()?.block_on(async move {
        match command {
            Command::DiscoverModels => voyage::server::models::run().await,
            Command::LegacyRecover(args) => {
                println!("{}", voyage::server::legacy_recovery::recover(args).await?);
                Ok(())
            }
            Command::UpgradeJournal(args) => voyage::server::bootstrap::upgrade(args),
            Command::Recover(args) => {
                println!("{}", voyage::server::recovery::recover(args).await?);
                Ok(())
            }
            Command::ObserveSuspended(args) => voyage::server::suspended::run(args).await,
            Command::Serve(args) => voyage::server::serve(args).await,
            Command::Supervise(_) => unreachable!("guardian runs before async runtime creation"),
            Command::HostResources(args) => voyage::host_resources::cli::run(args),
            Command::ValidateStart(args) => voyage::server::bootstrap::validate_start(args),
            Command::ImportPlan(args) => {
                println!("{}", voyage::server::bootstrap::import_plan(args)?);
                Ok(())
            }
            Command::Completions { shell } => {
                clap_complete::generate(
                    shell,
                    &mut Cli::command(),
                    "voyage",
                    &mut std::io::stdout(),
                );
                Ok(())
            }
            Command::Manpage => {
                clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
                Ok(())
            }
        }
    })
}
