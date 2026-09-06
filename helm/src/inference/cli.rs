//! Local operator control. No provider credentials or model dispatch are needed.
use super::*;
use clap::{Args, Subcommand};
#[derive(Args)]
pub struct InferenceArgs {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Inspect exact local permit counts and independently available token reports.
    Inspect {
        /// A saved session UUID; omit for the selected project's aggregate.
        #[arg(long)]
        session: Option<Uuid>,
        /// Exclusive sequence cursor for the bounded attempt page.
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// Read the bounded immutable configuration/warning/denial audit page.
    Audit {
        #[arg(long)]
        session: Option<Uuid>,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    /// Preview a revision-checked allowance edit; add --confirm to commit it.
    Configure {
        #[arg(long)]
        session: Option<Uuid>,
        /// Unique immutable edit identity; reuse these exact arguments for retry.
        #[arg(long)]
        operation: Uuid,
        #[arg(long)]
        expected_revision: u64,
        /// Cumulative permit ceiling since binding, not additional attempts.
        #[arg(
            long,
            conflicts_with = "unlimited",
            required_unless_present = "unlimited"
        )]
        limit: Option<u64>,
        #[arg(long)]
        unlimited: bool,
        #[arg(long)]
        warning: Option<u64>,
        /// Required operator explanation, including for an increase or removal.
        #[arg(long)]
        reason: String,
        #[arg(long)]
        confirm: bool,
    },
}
pub async fn run(
    args: InferenceArgs,
    workspace: &Path,
    redactor: &crate::tools::Redactor,
) -> Result<()> {
    run_inner(args, workspace, redactor)
        .await
        .map_err(|error| Failure::from_error(&error).into())
}
async fn run_inner(
    args: InferenceArgs,
    workspace: &Path,
    redactor: &crate::tools::Redactor,
) -> Result<()> {
    let path = Store::default_path();
    std::fs::create_dir_all(path.parent().context("inference store has no parent")?)?;
    let mut store = Store::open(path)?;
    let project = store.project(workspace)?;
    let session = match &args.command {
        Command::Inspect { session, .. }
        | Command::Audit { session, .. }
        | Command::Configure { session, .. } => *session,
    };
    let scope = if let Some(id) = session {
        // Initial configuration may target a saved, not-yet-run session. Verify
        // its actual workspace rather than inventing a session from CLI input.
        if store.session_project(id).is_err() {
            let session = crate::session::SessionStore::default().load(id).await?;
            ensure!(
                session.workspace.canonicalize()? == workspace.canonicalize()?,
                "saved session belongs to another project workspace"
            );
            store.bind_session(project, id)?;
        }
        ensure!(
            store.session_project(id)? == project,
            "session belongs to another inference project"
        );
        Scope::Session(id)
    } else {
        Scope::Project(project)
    };
    let output = match args.command {
        Command::Inspect { after, limit, .. } => {
            serde_json::json!({"schema_version":1,"unit":"local_inference_dispatch_permits","status":store.inspect(scope)?,"attempts":store.attempts(scope,after,limit)?,"usage_semantics":"token fields are provider-reported when present; null means unavailable; historical saved token sums are not complete billing totals"})
        }
        Command::Audit { after, limit, .. } => {
            serde_json::json!({"schema_version":1,"audit":store.audit(scope,after,limit)?})
        }
        Command::Configure {
            operation,
            expected_revision,
            limit,
            warning,
            reason,
            confirm,
            ..
        } => {
            ensure!(!redactor.contains_secret(&reason), Failure::Invalid);
            let change = Change {
                operation,
                scope,
                expected_revision,
                limit,
                warning,
                reason,
            };
            if confirm {
                serde_json::json!({"schema_version":1,"committed":true,"receipt":store.configure(&change)?})
            } else {
                serde_json::json!({"schema_version":1,"committed":false,"current":store.preview(&change)?,"proposed":change,"confirmation":"repeat these exact arguments with --confirm"})
            }
        }
    };
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

pub fn inspect_session(session: Uuid, workspace: &Path) -> Result<()> {
    let result = (|| {
        let path = Store::default_path();
        std::fs::create_dir_all(path.parent().context("inference store has no parent")?)?;
        let mut store = Store::open(path)?;
        let project = store.project(workspace)?;
        store.bind_session(project, session)?;
        for scope in [Scope::Project(project), Scope::Session(session)] {
            println!("{}", store.inspect(scope)?.summary());
        }
        Ok(())
    })();
    result.map_err(|error| Failure::from_error(&error).into())
}
