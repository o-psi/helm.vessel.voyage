use super::*;
use anyhow::Context;

pub async fn run(
    config: crate::Config,
    workspace: Option<PathBuf>,
    session: Option<String>,
    args: crate::github::operator::Args,
    configuration_explicit: bool,
) -> Result<()> {
    let words = super::github_args::words(args)?;
    let (client, process) =
        open(&config, workspace, session, false, configuration_explicit).await?;
    let snapshot = client
        .forward(
            process.session_id,
            process.incarnation,
            RuntimeCommand::Snapshot,
        )
        .await?;
    let command_id = Uuid::new_v4();
    eprintln!(
        "GitHub operator · voyage {} · command {command_id}",
        process.session_id
    );
    let receipt = client
        .forward(
            process.session_id,
            process.incarnation,
            RuntimeCommand::Github {
                command_id,
                expected_revision: snapshot["revision"]
                    .as_u64()
                    .context("snapshot revision missing")?,
                expires_at_ms: deadline()?,
                words,
            },
        )
        .await?;
    ensure!(
        receipt["status"] != "rejected",
        "GitHub command rejected: {}",
        super::super::safe(&receipt.to_string())
    );
    let run_id = serde_json::from_value(receipt["run_id"].clone())
        .context("operator receipt missing run identity")?;
    super::super::plain::follow(&client, process.session_id, run_id).await
}
