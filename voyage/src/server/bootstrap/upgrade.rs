//! Explicit offline format upgrade acquires all legacy session guards internally.
use super::*;
#[derive(clap::Args)]
pub struct UpgradeArgs {
    #[arg(long)]
    pub directory: PathBuf,
}
pub fn upgrade(args: UpgradeArgs) -> Result<()> {
    ensure!(
        args.directory.is_absolute() && args.directory.join("journal.sqlite3").is_file(),
        "existing absolute journal directory required"
    );
    Journal::open(args.directory)?.upgrade_quiescent()
}
