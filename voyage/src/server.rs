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
pub(crate) mod decisions;
mod dispatch;
mod github;
mod goals;
pub mod guardian;
mod images;
pub mod models;
mod observations;
mod submission;
pub mod suspended;
mod suspension;
mod transfer;
#[cfg(unix)]
mod transport;
mod workflows;
mod workspace_changes;
mod workspace_file_catalog;

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
    browser: Arc<crate::browser::BrowserBroker>,
    host_browser: Arc<crate::host_browser::HostBrowser>,
    directory: PathBuf,
    workflows: workflows::Workflows,
    controls: Arc<controls::LiveControls>,
    cleanup: Arc<crate::execution::cleanup::CleanupSlot>,
    owner: ManagedSessionOwner,
    actor: LocalActor,
    config: tokio::sync::RwLock<Config>,
    registration: ProcessRegistration,
    active: Mutex<Option<ActiveRun>>,
    goal_wake: tokio::sync::Notify,
    admission: Mutex<()>,
    requests: tokio::sync::RwLock<()>,
    suspend_requested: std::sync::atomic::AtomicBool,
    shutdown: CancellationToken,
    archive_receipt: Mutex<Option<serde_json::Value>>,
}
#[cfg(target_os = "linux")]
mod bound;

pub async fn serve(args: ServeArgs) -> Result<()> {
    serve_registered(args, None).await
}

/// Root-owned launch pipe, distinct from the child-writable projection. No
/// configuration or journal is opened until this registration is authenticated.
#[cfg(target_os = "linux")]
pub async fn serve_bound(args: ServeArgs) -> Result<()> {
    let registration = bound::registration().await?;
    serve_registered(args, Some(registration)).await
}

async fn serve_registered(args: ServeArgs, admitted: Option<ProcessRegistration>) -> Result<()> {
    #[cfg(not(unix))]
    {
        let _ = (args, admitted);
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
        let registration = match &admitted {
            Some(registration) => registration.clone(),
            None => {
                let registration = transport::registration(&directory)?;
                ensure!(
                    registration.peer_uids.is_none(),
                    "bound startup requires protected launch admission"
                );
                registration
            }
        };
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
        // A child-writable projection cannot replace authority received on the
        // launch pipe. Publish only as this runtime identity for later observers.
        let current = if let Some(registration) = &admitted {
            bound_projection(&directory, registration)?;
            registration.clone()
        } else {
            transport::registration(&directory)?
        };
        ensure!(
            current.session_id == registration.session_id
                && current.incarnation == registration.incarnation
                && current.token == registration.token
                && current.workspace == registration.workspace
                && current.config_path == registration.config_path,
            "runtime registration changed during startup"
        );
        let prepared: Result<_> = async {
            bootstrap::prepare_workspace(&directory, &registration)?;
            let workspace = args.workspace.canonicalize()?;
            let initial =
                Journal::open(directory.join("journal"))?.initial_configuration(args.session)?;
            let initial = match initial {
                Some(initial) => Some(initial),
                None => Journal::frozen_branch_configuration(registration.initialize.as_ref())?,
            };
            let config = match initial {
                Some(settings) => {
                    if admitted.is_some(){
                        if let Ok(expected)=std::env::var("VOYAGE_BOUND_RETAINED_CONFIG_DIGEST"){
                            ensure!(settings.len()<=65536&&expected.len()==64&&crate::identity_helper::config_digest(settings.as_bytes())==expected,"reviewed retained configuration changed before startup");
                        }
                    }
                    serde_json::from_str::<crate::launch_config::LaunchConfig>(&settings)?
                        .resolve(&workspace)?
                }
                None => {
                    let expected_digest = if admitted.is_some() {
                        std::env::var("VOYAGE_BOUND_CONFIG_DIGEST").ok()
                    } else {
                        None
                    };
                    let mut config = bootstrap::load_config_with_digest(
                        args.config.as_deref(),
                        &workspace,
                        expected_digest.as_deref(),
                    )?;
                    if registration.initialize.is_none() {
                        match Journal::open(directory.join("journal"))?.load_session(args.session) {
                            Ok(_) => {}
                            Err(error)
                                if error.downcast_ref::<rusqlite::Error>().is_some_and(|e| {
                                    matches!(e, rusqlite::Error::QueryReturnedNoRows)
                                }) =>
                            {
                                config.require_default_account()?
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    config
                }
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
        owner
            .bind_notification_incarnation(args.incarnation)
            .await?;
        crate::host_resources::set_process_scope(args.session, args.incarnation)?;
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
        if let Some(previous) = admitted.as_ref().and_then(|r| r.restart_from) {
            // Only the authenticated root pipe carries this local cleanup proof.
            // Remote effects and unresolved receipts are retained, never replayed.
            crate::host_resources::recover_process_scope(args.session, previous)?;
            owner.recover_process_cleanup().await?;
            owner.retain_interrupted_cleanup().await?;
            owner.recover_tool_outcomes().await?;
        }
        owner.recover_goal_turn().await?;
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
        config.artifact_scope = Some(crate::artifacts::Scope {
            directory: directory.join("journal"),
            session: registration.session_id,
        });
        config.vessel_context = if registration.peer_uids.is_some() {
            None
        } else {
            Some(crate::tools::VesselContext {
                session_id: registration.session_id,
                directory: directory
                    .parent()
                    .and_then(std::path::Path::parent)
                    .context("runtime directory missing supervising Vessel")?
                    .to_path_buf(),
            })
        };
        config.live_access = Some(Arc::new(crate::policy::LiveAccess::new(
            crate::runtime_policy::RuntimePolicy::resolve(&config, &registration.workspace)?
                .policy()
                .access_mode(),
        )));
        crate::build::set_resource_root(directory.join("resources"))?;
        let browser_directory = directory.join("journal");
        let browser_session = registration.session_id;
        let browser_incarnation = registration.incarnation;
        let browser = Journal::blocking_checkpoint(move || {
            crate::browser::BrowserBroker::open(
                browser_directory,
                browser_session,
                browser_incarnation,
            )
        })
        .await?;
        let host_browser_launch = if config.sandbox.mode == crate::sandbox::Mode::Required {
            None // Never silently bypass a host-required OS execution sandbox.
        } else {
            config
                .host_browser_launch
                .clone()
                .or_else(crate::host_browser::Launch::discover)
        };
        let host_browser = crate::host_browser::HostBrowser::new(
            directory.join("journal"),
            registration.session_id,
            registration.incarnation,
            host_browser_launch,
        );
        config.host_browser = Some(host_browser.clone());
        config.browser = Some(browser.clone());
        let state = Arc::new(State {
            host_browser,
            browser,
            directory: directory.clone(),
            workflows: workflows::Workflows::default(),
            controls: Arc::new(controls::LiveControls::default()),
            cleanup: Arc::new(crate::execution::cleanup::CleanupSlot::default()),
            owner,
            actor,
            config: tokio::sync::RwLock::new(config),
            registration,
            active: Mutex::new(None),
            goal_wake: tokio::sync::Notify::new(),
            admission: Mutex::new(()),
            requests: tokio::sync::RwLock::new(()),
            suspend_requested: std::sync::atomic::AtomicBool::new(false),
            shutdown: CancellationToken::new(),
            archive_receipt: Mutex::new(None),
        });
        transport::listen(directory, state).await
    }
}

pub mod recovery;

pub mod legacy_recovery;

#[cfg(all(test, unix))]
mod tests;

#[cfg(all(test, unix))]
mod recovery_tests;

#[cfg(all(test, unix))]
mod image_upload_tests;

#[cfg(unix)]
fn bound_projection(directory: &std::path::Path, registration: &ProcessRegistration) -> Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    let bytes = serde_json::to_vec(registration)?;
    ensure!(bytes.len() <= 16384, "bound projection exceeds limit");
    let temporary = directory.join(format!(".registration-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, directory.join("registration.json"))?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}
