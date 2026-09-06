//! Explicit consent administration runs in the execution-host runtime executable.
use anyhow::Result;
pub(super) use voyage_runtime::server::remote_consent::Args;
pub(super) async fn run(mut args: Args) -> Result<()> {
    args.directory = super::remote_worker::supervised_directory(&args.directory)?;
    let result = super::managed::maintenance::invoke(args.arguments()).await?;
    super::remote_worker::write_notice(result.to_string()).await
}
pub(super) fn safe_error(error: anyhow::Error) -> anyhow::Error {
    voyage_runtime::server::remote_consent::safe_error(error)
}
