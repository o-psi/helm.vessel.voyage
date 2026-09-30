//! Separate root-operator enrollment; pairing and execution grants stay independent.
use anyhow::Result;
use clap::Args;
use std::path::PathBuf;
use uuid::Uuid;
use voyage_protocol::execution_review_control::AdministratorOwnerChange;
#[derive(Args)]
pub struct AdministrativeOwnerArgs {
    /// Existing root-private supervisor control directory.
    #[arg(long)]
    pub directory: PathBuf,
    /// Exact Helm principal to enroll or revoke; never inferred from login/pairing.
    #[arg(long)]
    pub principal: Uuid,
    /// Explicitly enroll the principal for future administrator reviews.
    #[arg(long, required_unless_present = "revoke", conflicts_with = "revoke")]
    pub enroll: bool,
    /// Fence this principal's retained administrator grants. Cleanup is observed separately.
    #[arg(long, required_unless_present = "enroll", conflicts_with = "enroll")]
    pub revoke: bool,
    /// Pinned authority revision, or 0 for the first explicit enrollment.
    #[arg(long)]
    pub expected_revision: u64,
    /// Retain this UUID for retries of this exact root-operator operation.
    #[arg(long)]
    pub command_id: Uuid,
}
pub async fn run(args: AdministrativeOwnerArgs) -> Result<()> {
    let change = AdministratorOwnerChange {
        command_id: args.command_id,
        principal_id: args.principal,
        expected_authority_revision: args.expected_revision,
        enabled: args.enroll,
    };
    let receipt =
        super::database::execution_reviews::change_owner(&args.directory, &change).await?;
    println!("{}", serde_json::to_string(&receipt)?);
    Ok(())
}
