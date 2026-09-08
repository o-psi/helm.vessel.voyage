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
mod authorization;
pub mod bootstrap;
mod commands;
mod configuration;
pub mod controls;
mod decisions;
mod dispatch;
mod github;
mod observations;
#[cfg(unix)]
mod outbound;
#[cfg(unix)]
pub use outbound::{outbound_observe, outbound_relay};
mod images;
mod submission;
pub mod suspended;
mod suspension;
mod transfer;
#[cfg(unix)]
mod transport;
mod workflows;

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
    inference: serde_json::Value,
    cancel: CancellationToken,
    steering: Option<ManagedSteeringHandle>,
}
struct State {
    directory: PathBuf,
    workflows: workflows::Workflows,
    controls: Arc<controls::LiveControls>,
    cleanup: Arc<crate::execution::cleanup::CleanupSlot>,
    owner: ManagedSessionOwner,
    actor: LocalActor,
    config: tokio::sync::RwLock<Config>,
    registration: ProcessRegistration,
    active: Mutex<Option<ActiveRun>>,
    admission: Mutex<()>,
    requests: tokio::sync::RwLock<()>,
    suspend_requested: std::sync::atomic::AtomicBool,
    shutdown: CancellationToken,
    archive_receipt: Mutex<Option<serde_json::Value>>,
    outbound_task: Mutex<Option<tokio::task::JoinHandle<Result<()>>>>,
    outbound_status: Mutex<serde_json::Value>,
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
        ensure!(
            directory
                .join("runtime.sock")
                .as_os_str()
                .as_encoded_bytes()
                .len()
                < 108,
            "runtime socket path exceeds Unix socket limit"
        );
        let registration = transport::registration(&directory)?;
        ensure!(
            registration.session_id == args.session
                && registration.incarnation == args.incarnation
                && registration.workspace == args.workspace,
            "runtime registration mismatch"
        );
        ensure!(
            args.config == registration.config_path,
            "runtime configuration registration mismatch"
        );
        let startup =
            crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
        let startup_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match startup.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if tokio::time::Instant::now() < startup_deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => {
                    return Err(anyhow::anyhow!("runtime startup already owned: {error}"));
                }
            }
        }
        let current = transport::registration(&directory)?;
        ensure!(
            current.session_id == registration.session_id
                && current.incarnation == registration.incarnation
                && current.token == registration.token
                && current.workspace == registration.workspace
                && current.config_path == registration.config_path,
            "runtime registration changed during startup"
        );
        let prepared: Result<_> = async {
            let workspace = args.workspace.canonicalize()?;
            let initial =
                Journal::open(directory.join("journal"))?.initial_configuration(args.session)?;
            let initial = match initial {
                Some(initial) => Some(initial),
                None => Journal::frozen_branch_configuration(registration.initialize.as_ref())?,
            };
            let config = match initial {
                Some(settings) => {
                    serde_json::from_str::<crate::launch_config::LaunchConfig>(&settings)?
                        .resolve(&workspace)?
                }
                None => bootstrap::load_config(args.config.as_deref(), &workspace)?,
            };
            let workspace = config.resolve_workspace(Some(workspace))?;
            bootstrap::prepare_identity(&directory, &registration)?;
            let actor = LocalActorStore::open(&directory.join("identity"))?.identity()?;
            bootstrap::initialize(&directory, &registration, &workspace).await?;
            Ok((workspace, config, actor))
        }
        .await;
        let (workspace, config, actor) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                if let Err(evidence) = bootstrap::record_failure(&directory, &registration) {
                    tracing::warn!("startup failed without clean evidence: {evidence}");
                }
                return Err(error);
            }
        };
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
                if let Some(voyage_protocol::process::RuntimeInitialization::Participant {
                    parent_session_id,
                    ..
                }) = &registration.initialize
                {
                    session.parent_id = Some(*parent_session_id);
                }
                journal.create_session(&session)?;
            }
            Err(error) => return Err(error),
        }
        drop(journal);
        let owner = suspended::open_owner(journal_dir, args.session).await?;
        drop(startup);
        // A new lifetime must establish its own shutdown evidence, even when an
        // operator explicitly starts the same incarnation outside the supervisor.
        match std::fs::remove_file(directory.join("stopped.json")) {
            Ok(()) => std::fs::File::open(&directory)?.sync_all()?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        owner.initialize_process_commands().await?;
        owner.initialize_session_resources().await?;
        owner
            .initialize_command_bindings(actor.principal_id)
            .await?;
        // Recovery commits interrupted evidence; it never claims descendants stopped.
        owner.recover_interrupted().await?;
        let mut config = match owner.saved_configuration().await? {
            Some(settings) => {
                serde_json::from_str::<crate::launch_config::LaunchConfig>(&settings)?
                    .resolve(&registration.workspace)?
            }
            None => config,
        };
        bootstrap::limit_participant(&mut config, &registration)?;
        owner
            .retain_initial_configuration(&config, &registration.workspace)
            .await?;
        config.vessel_context = Some(crate::tools::VesselContext {
            session_id: registration.session_id,
            directory: directory
                .parent()
                .and_then(std::path::Path::parent)
                .context("runtime directory missing supervising Vessel")?
                .to_path_buf(),
        });
        config.live_access = Some(Arc::new(crate::policy::LiveAccess::new(
            crate::runtime_policy::RuntimePolicy::resolve(&config, &registration.workspace)?
                .policy()
                .access_mode(),
        )));
        crate::build::set_resource_root(directory.join("resources"))?;
        let state = Arc::new(State {
            directory: directory.clone(),
            workflows: workflows::Workflows::default(),
            controls: Arc::new(controls::LiveControls::default()),
            cleanup: Arc::new(crate::execution::cleanup::CleanupSlot::default()),
            owner,
            actor,
            config: tokio::sync::RwLock::new(config),
            registration,
            active: Mutex::new(None),
            admission: Mutex::new(()),
            requests: tokio::sync::RwLock::new(()),
            suspend_requested: std::sync::atomic::AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            archive_receipt: Mutex::new(None),
            outbound_task: Mutex::new(None),
            outbound_status: Mutex::new(serde_json::Value::Null),
        });
        if matches!(
            state.registration.initialize,
            Some(voyage_protocol::process::RuntimeInitialization::Outbound { .. })
        ) {
            *state.outbound_status.lock().await = serde_json::json!({"state":"connecting"});
            let relay = state.clone();
            *state.outbound_task.lock().await = Some(tokio::spawn(async move {
                let result = outbound::run(relay.clone()).await;
                if result.is_err() {
                    relay.shutdown.cancel();
                }
                *relay.outbound_status.lock().await = if result.is_ok() {
                    serde_json::json!({"state":"stopped","grant":"inactive"})
                } else {
                    serde_json::json!({"state":"unavailable","grant":"inactive","detail":"relay ended; inspect local consent and explicitly restart to reconnect"})
                };
                result
            }));
        }
        transport::listen(directory, state).await
    }
}

pub mod recovery;

pub mod legacy_recovery;

pub mod remote_consent;
