//! Explicit outbound worker activation is a thin supervised-process client.
use crate::Config;
use anyhow::{Context, Result, ensure};
use helm::process_client;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use uuid::Uuid;
use voyage_protocol::process::{ProcessInfo, VesselCommand};
#[derive(clap::Args)]
pub(super) struct Args {
    #[arg(long)]
    directory: PathBuf,
    #[arg(long, required_unless_present = "recover", conflicts_with = "recover")]
    enrollment_directory: Option<PathBuf>,
    #[arg(long, required_unless_present = "recover", conflicts_with = "recover")]
    origin: Option<String>,
    #[arg(long, conflicts_with = "recover")]
    allow_insecure_loopback: bool,
    #[arg(long)]
    pub recover: bool,
    #[arg(long, requires = "recover")]
    acknowledge_cleanup: Option<Uuid>,
    #[arg(long,requires_all=["recover","expected_revision"])]
    reconcile_tools: Option<Uuid>,
    #[arg(long, requires = "reconcile_tools")]
    expected_revision: Option<u64>,
}
pub(super) async fn run(args: Args, config: Config, workspace: Option<PathBuf>) -> Result<()> {
    ensure!(
        args.directory.is_absolute(),
        "absolute outbound installation required"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&args.directory)?;
    }
    process_client::local::check_private_directory(&args.directory)?;
    let client =
        process_client::local::connect(process_client::cli::default_directory(), true).await?;
    let workspace = config.resolve_workspace(workspace)?;
    let manifest = args.directory.join("outbound-launch.json");
    let command = if manifest.exists() {
        let command: VesselCommand = serde_json::from_slice(&read(&manifest)?)?;
        let VesselCommand::StartOutbound {
            workspace: old_workspace,
            enrollment_directory,
            origin,
            allow_insecure_loopback,
            config_path,
            ..
        } = &command
        else {
            anyhow::bail!("invalid outbound launch receipt")
        };
        ensure!(
            *old_workspace == workspace
                && Some(enrollment_directory) == args.enrollment_directory.as_ref()
                && Some(origin) == args.origin.as_ref()
                && *allow_insecure_loopback == args.allow_insecure_loopback,
            "outbound launch parameters differ from retained request"
        );
        let launch: voyage_runtime::launch_config::LaunchConfig =
            serde_json::from_slice(&read(config_path)?)?;
        ensure!(
            launch.matches_config(&config)?,
            "outbound configuration differs from retained launch; use explicit owner configuration workflow"
        );
        command
    } else {
        let (session, revision, source) =
            if args.directory.join("journal/journal.sqlite3").is_file() {
                let actor = voyage_runtime::attachment::local_actor::LocalActorStore::open(
                    &args.directory,
                )?
                .identity()?;
                let journal = voyage_runtime::attachment::journal::Journal::open(
                    args.directory.join("journal"),
                )?;
                let (id, _) = journal.remote_local_binding(&actor)?;
                (
                    id,
                    Some(journal.load_session(id)?.revision),
                    Some(args.directory.clone()),
                )
            } else {
                (Uuid::new_v4(), None, None)
            };
        let config_path =
            process_client::frontend::persist_launch(&config, &workspace, &client.directory)?;
        let command = VesselCommand::StartOutbound {
            command_id: Uuid::new_v4(),
            session_id: session,
            workspace,
            config_path,
            enrollment_directory: args
                .enrollment_directory
                .context("enrollment directory required")?,
            origin: args.origin.context("origin required")?,
            allow_insecure_loopback: args.allow_insecure_loopback,
            source_directory: source,
            expected_revision: revision,
        };
        persist(&manifest, &serde_json::to_value(&command)?)?;
        command
    };
    let mut process: ProcessInfo = serde_json::from_value(client.request(command).await?)?;
    if process.state == voyage_protocol::process::ProcessState::Stopped {
        process = serde_json::from_value(
            client
                .request(VesselCommand::Restart {
                    command_id: Uuid::new_v4(),
                    session_id: process.session_id,
                    incarnation: process.incarnation,
                })
                .await?,
        )?;
    }
    write_notice(serde_json::json!({"event":"outbound_voyage_started","session_id":process.session_id,"incarnation":process.incarnation,"state":process.state,"lifetime":"supervised"}).to_string()).await
}
pub(super) async fn recover(args: Args) -> Result<()> {
    ensure!(
        args.directory.is_absolute(),
        "absolute outbound installation required"
    );
    let manifest = args.directory.join("outbound-launch.json");
    let result = if manifest.exists() {
        let command: VesselCommand = serde_json::from_slice(&read(&manifest)?)?;
        let VesselCommand::StartOutbound { session_id, .. } = command else {
            anyhow::bail!("invalid outbound launch receipt")
        };
        let client =
            process_client::local::connect(process_client::cli::default_directory(), true).await?;
        let processes: Vec<ProcessInfo> =
            serde_json::from_value(client.request(VesselCommand::Catalogue).await?)?;
        let process = processes
            .into_iter()
            .find(|process| process.session_id == session_id)
            .context("outbound registration unavailable")?;
        client
            .request(VesselCommand::Recover {
                command_id: Uuid::new_v4(),
                session_id,
                incarnation: process.incarnation,
                acknowledge_cleanup: args.acknowledge_cleanup,
                acknowledge_resources: vec![],
                reconcile_tools: args.reconcile_tools,
                expected_revision: args.expected_revision,
            })
            .await?
    } else {
        let actor =
            voyage_runtime::attachment::local_actor::LocalActorStore::open(&args.directory)?
                .identity()?;
        let journal =
            voyage_runtime::attachment::journal::Journal::open(args.directory.join("journal"))?;
        let (session, _) = journal.remote_local_binding(&actor)?;
        let mut command = vec![
            "legacy-recover".into(),
            "--directory".into(),
            args.directory.into_os_string(),
            "--session".into(),
            session.to_string().into(),
            "--remote".into(),
        ];
        for (flag, value) in [
            (
                "--acknowledge-cleanup",
                args.acknowledge_cleanup.map(|id| id.to_string()),
            ),
            (
                "--reconcile-tools",
                args.reconcile_tools.map(|id| id.to_string()),
            ),
            (
                "--expected-revision",
                args.expected_revision.map(|id| id.to_string()),
            ),
        ] {
            if let Some(value) = value {
                command.push(flag.into());
                command.push(value.into());
            }
        }
        crate::managed::maintenance::invoke(command).await?
    };
    write_notice(result.to_string()).await
}
pub(super) async fn write_notice(notice: String) -> Result<()> {
    println!("{}", helm::process_client::safe(&notice));
    Ok(())
}
fn read(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    for ancestor in path.ancestors() {
        ensure!(
            !std::fs::symlink_metadata(ancestor)?
                .file_type()
                .is_symlink(),
            "symlink in outbound receipt path"
        );
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 65536,
        "invalid outbound receipt"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "outbound receipt must be private"
        );
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "outbound receipt too large");
    Ok(bytes)
}
fn persist(path: &Path, value: &serde_json::Value) -> Result<()> {
    let parent = path.parent().context("receipt parent missing")?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub(super) fn supervised_directory(directory: &Path) -> Result<PathBuf> {
    let manifest = directory.join("outbound-launch.json");
    if !manifest.exists() {
        return Ok(directory.to_path_buf());
    }
    let command: VesselCommand = serde_json::from_slice(&read(&manifest)?)?;
    let VesselCommand::StartOutbound { session_id, .. } = command else {
        anyhow::bail!("invalid outbound receipt")
    };
    let target = process_client::cli::default_directory()
        .join("sessions")
        .join(session_id.to_string());
    ensure!(
        target.join("registration.json").is_file(),
        "supervised outbound registration missing"
    );
    Ok(target)
}
