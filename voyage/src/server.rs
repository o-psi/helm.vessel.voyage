//! One lifetime owner per process. Transport tasks never instantiate executors.
use crate::attachment::{
    journal::Journal,
    local_actor::{LocalActor, LocalActorStore},
    runtime::{ManagedSessionOwner, ManagedSteeringHandle},
};
use crate::{Config, session::Session};
use anyhow::{Context, Result, ensure};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::process::ProcessRegistration;
mod commands;
mod decisions;
#[cfg(unix)]
mod transport;

#[derive(clap::Args)]
pub struct ServeArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub session: Uuid,
    #[arg(long)]
    pub incarnation: Uuid,
    #[arg(long)]
    pub workspace: PathBuf,
    #[arg(long)]
    pub config: Option<PathBuf>,
}
struct ActiveRun {
    id: Uuid,
    cancel: CancellationToken,
    steering: ManagedSteeringHandle,
}
struct State {
    owner: ManagedSessionOwner,
    actor: LocalActor,
    config: Config,
    registration: ProcessRegistration,
    active: Mutex<Option<ActiveRun>>,
    admission: Mutex<()>,
    shutdown: CancellationToken,
}
pub async fn serve(args: ServeArgs) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = args;
        anyhow::bail!("private voyage process transport unsupported on this platform");
    }
    #[cfg(unix)]
    {
        let directory = crate::attachment::journal::prepare_directory(args.directory)?;
        let registration = transport::registration(&directory)?;
        ensure!(
            registration.session_id == args.session
                && registration.incarnation == args.incarnation
                && registration.workspace == args.workspace,
            "runtime registration mismatch"
        );
        let config = Config::load(args.config.as_deref())?;
        let workspace = config.resolve_workspace(Some(args.workspace))?;
        let actor = LocalActorStore::open(&directory.join("identity"))?.identity()?;
        let journal_dir = directory.join("journal");
        let mut journal = Journal::open(journal_dir.clone())?;
        match journal.load_session(args.session) {
            Ok(saved) => ensure!(
                saved.session.workspace == workspace,
                "saved workspace mismatch"
            ),
            Err(error)
                if error
                    .downcast_ref::<rusqlite::Error>()
                    .is_some_and(|e| matches!(e, rusqlite::Error::QueryReturnedNoRows)) =>
            {
                let mut session = Session::new(workspace, config.model.clone());
                session.id = args.session;
                journal.create_session(&session)?;
            }
            Err(error) => return Err(error),
        }
        drop(journal);
        let owner = ManagedSessionOwner::open(journal_dir, args.session).await?;
        // A new lifetime must establish its own shutdown evidence, even when an
        // operator explicitly starts the same incarnation outside the supervisor.
        match std::fs::remove_file(directory.join("stopped.json")) {
            Ok(()) => std::fs::File::open(&directory)?.sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        owner.initialize_process_commands().await?;
        // Recovery commits interrupted evidence; it never claims descendants stopped.
        owner.recover_interrupted().await?;
        crate::build::set_resource_root(directory.join("resources"))?;
        let state = Arc::new(State {
            owner,
            actor,
            config,
            registration,
            active: Mutex::new(None),
            admission: Mutex::new(()),
            shutdown: CancellationToken::new(),
        });
        transport::listen(directory, state).await
    }
}
