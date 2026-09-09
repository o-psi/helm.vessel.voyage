//! Bounded administrative recovery of pre-supervisor journals, without execution.
use super::*;
#[derive(clap::Args)]
pub struct LegacyRecoverArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub session: Uuid,
    #[arg(long)]
    pub acknowledge_cleanup: Option<Uuid>,
    #[arg(long)]
    pub reconcile_tools: Option<Uuid>,
    #[arg(long)]
    pub expected_revision: Option<u64>,
}
pub async fn recover(args: LegacyRecoverArgs) -> Result<serde_json::Value> {
    ensure!(
        args.directory.is_absolute()
            && args.directory.join("journal/journal.sqlite3").is_file()
            && args.directory.join("actor.json").is_file(),
        "existing legacy installation required"
    );
    let actor = LocalActorStore::open(&args.directory)?.identity()?;
    let journal_dir = args.directory.join("journal");
    let owner = ManagedSessionOwner::open(journal_dir, args.session).await?;
    let recovered = owner.recover_interrupted().await?;
    if let Some(run) = args.acknowledge_cleanup {
        owner.attest_local_cleanup(run, actor).await?;
    }
    let reconciliation = if let Some(run) = args.reconcile_tools {
        let request = crate::attachment::journal::LocalReconcileRequest {
            session_id: args.session,
            run_id: run,
            installation_id: actor.installation_id,
            principal_id: actor.principal_id,
            expected_revision: args
                .expected_revision
                .context("reconciliation revision required")?,
        };
        Some(owner.reconcile_local_tools(request).await?)
    } else {
        None
    };
    Ok(
        serde_json::json!({"session_id":args.session,"run":recovered.map(|run|serde_json::json!({"run_id":run.id,"state":run.state})),"cleanup":if args.acknowledge_cleanup.is_some(){"operator_attested"}else{"unchanged"},"reconciliation":reconciliation}),
    )
}
