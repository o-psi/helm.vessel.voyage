use clap::{CommandFactory, Parser, Subcommand};
#[derive(Parser)]
#[command(version, about = "Independent voyage session runtime")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Serve(voyage::server::ServeArgs),
    Completions { shell: clap_complete::Shell },
    Manpage,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve(args) => voyage::server::serve(args).await,
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "voyage", &mut std::io::stdout());
            Ok(())
        }
        Command::Manpage => {
            clap_mangen::Man::new(Cli::command()).render(&mut std::io::stdout())?;
            Ok(())
        }
    }
}
