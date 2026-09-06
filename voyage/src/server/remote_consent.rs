//! Local administration only; does not initialize a provider or contact Vessel.
use crate::attachment::{
    journal::{Journal, WithdrawalRequest},
    local_actor::LocalActorStore,
};
use anyhow::Result;
use std::path::PathBuf;

#[derive(clap::Args)]
pub struct Args {
    /// Existing dedicated worker installation; never creates or imports a session.
    #[arg(long)]
    pub directory: PathBuf,
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
pub async fn run(args: Args) -> Result<()> {
    anyhow::ensure!(
        args.directory.is_absolute()
            && (args.directory.join("actor.json").is_file()
                || args.directory.join("identity/actor.json").is_file())
            && args.directory.join("journal/journal.sqlite3").is_file(),
        "existing absolute dedicated installation required"
    );
    let actor = LocalActorStore::open(&if args.directory.join("registration.json").is_file() {
        args.directory.join("identity")
    } else {
        args.directory.clone()
    })?
    .identity()?;
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
    println!("{}", serde_json::to_string(&value)?);
    Ok(())
}

pub fn safe_error(error: anyhow::Error) -> anyhow::Error {
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

impl Args {
    pub fn arguments(self) -> Vec<std::ffi::OsString> {
        let mut args = vec![
            "remote-consent".into(),
            "--directory".into(),
            self.directory.into_os_string(),
        ];
        let (action, selection, confirmation) = match self.command {
            Action::Inspect => ("inspect", None, None),
            Action::Preview(selection) => ("preview", Some(selection), None),
            Action::Withdraw { selection, confirm } => ("withdraw", Some(selection), Some(confirm)),
        };
        args.push(action.into());
        if let Some(selection) = selection {
            for (flag, value) in [
                ("--session-id", selection.session_id.to_string()),
                ("--operation-id", selection.operation_id.to_string()),
                (
                    "--expected-revision",
                    selection.expected_revision.to_string(),
                ),
            ] {
                args.push(flag.into());
                args.push(value.into());
            }
        }
        if let Some(confirm) = confirmation {
            args.push("--confirm".into());
            args.push(confirm.into());
        }
        args
    }
}
