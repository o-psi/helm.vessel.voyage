//! Local administration only; does not initialize a provider or contact Vessel.
use super::*;
use helm::attachment::{
    journal::{Journal, WithdrawalRequest},
    local_actor::LocalActorStore,
};

#[derive(clap::Args)]
pub(super) struct Args {
    /// Existing dedicated worker installation; never creates or imports a session.
    #[arg(long)]
    directory: PathBuf,
    #[command(subcommand)]
    command: Action,
}
#[derive(clap::Subcommand)]
enum Action {
    /// Inspect the current grant, immutable retirement receipt and unresolved cleanup.
    Inspect,
    /// Preview permanent withdrawal. Save the exact request and confirmation digest.
    Preview(Selection),
    /// Permanently retire this grant; request active cancellation, never assert stopped.
    Withdraw {
        #[command(flatten)]
        selection: Selection,
        #[arg(long)]
        confirm: String,
    },
}
#[derive(clap::Args)]
struct Selection {
    #[arg(long)]
    session_id: uuid::Uuid,
    #[arg(long)]
    operation_id: uuid::Uuid,
    /// Grant revision, independent of changing canonical session revisions.
    #[arg(long)]
    expected_revision: u64,
}
impl Selection {
    fn request(self) -> WithdrawalRequest {
        WithdrawalRequest {
            session_id: self.session_id,
            operation_id: self.operation_id,
            expected_revision: self.expected_revision,
        }
    }
}
pub(super) async fn run(args: Args) -> Result<()> {
    anyhow::ensure!(
        args.directory.is_absolute()
            && args.directory.join("actor.json").is_file()
            && args.directory.join("journal/journal.sqlite3").is_file(),
        "existing absolute dedicated installation required"
    );
    let actor = LocalActorStore::open(&args.directory)?.identity()?;
    let mut journal = Journal::open(args.directory.join("journal"))?;
    let value =
        match args.command {
            Action::Inspect => serde_json::to_value(journal.remote_consent_status(&actor)?)?,
            Action::Preview(selection) => serde_json::to_value(
                journal.preview_remote_withdrawal(&actor, &selection.request())?,
            )?,
            Action::Withdraw { selection, confirm } => serde_json::to_value(
                journal.withdraw_remote(&actor, &selection.request(), &confirm)?,
            )?,
        };
    // A failed output does not undo a committed withdrawal. The original request recovers it.
    remote_worker::write_notice(serde_json::to_string(&value)?).await
}

pub(super) fn safe_error(error: anyhow::Error) -> anyhow::Error {
    for cause in error.chain() {
        if let Some(rusqlite::Error::SqliteFailure(code, _)) =
            cause.downcast_ref::<rusqlite::Error>()
        {
            match code.code {
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => {
                    return anyhow::anyhow!(
                        "remote consent store busy; preserve the request and explicitly retry the identical operation"
                    );
                }
                rusqlite::ErrorCode::DiskFull => {
                    return anyhow::anyhow!(
                        "remote consent storage full; preserve the request and inspect current consent after resolving storage capacity"
                    );
                }
                _ => {}
            }
        }
    }
    anyhow::anyhow!(
        "remote consent operation failed; preserve the exact request, inspect the dedicated journal and explicit schema upgrade requirements"
    )
}
