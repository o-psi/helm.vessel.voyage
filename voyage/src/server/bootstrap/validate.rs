use super::*;
#[derive(clap::Args)]
pub struct ValidateStartArgs {
    #[arg(long)]
    pub workspace: PathBuf,
    #[arg(long)]
    pub config: Option<PathBuf>,
}
pub fn validate_start(args: ValidateStartArgs) -> Result<()> {
    let workspace = args.workspace.canonicalize()?;
    let config = config::load(args.config.as_deref(), &workspace)?;
    crate::runtime_policy::RuntimePolicy::resolve(&config, &workspace)?;
    Ok(())
}
