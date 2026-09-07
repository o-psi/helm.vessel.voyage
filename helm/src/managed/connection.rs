use super::*;
use helm::process_client::{local, transport::Client};
use voyage_protocol::vessel::{ProcessInfo, VesselCommand};

pub(super) async fn connect(directory: &std::path::Path) -> Result<Client> {
    ensure!(
        directory.is_absolute(),
        "managed directory must be absolute"
    );
    local::connect(directory.join("vessel"), true).await
}

pub(super) async fn session(
    client: &Client,
    directory: &std::path::Path,
    id: Uuid,
    config: Option<&Config>,
    workspace: Option<PathBuf>,
) -> Result<ProcessInfo> {
    let catalogue: Vec<ProcessInfo> =
        serde_json::from_value(client.request(VesselCommand::Catalogue).await?)?;
    if let Some(process) = catalogue.into_iter().find(|p| p.session_id == id) {
        return Ok(process);
    }
    ensure!(
        directory.join("journal/journal.sqlite3").is_file(),
        "unknown managed session"
    );
    let saved = Journal::open(directory.join("journal"))?.load_session(id)?;
    if let Some(workspace) = workspace {
        ensure!(
            workspace.canonicalize()? == saved.session.workspace.canonicalize()?,
            "managed workspace does not match canonical session"
        );
    }
    let mut config = match config {
        Some(config) => config.clone(),
        None => Config::load(None)?,
    };
    config.model = saved.session.model;
    let config_path = helm::process_client::frontend::persist_launch(
        &config,
        &saved.session.workspace,
        &client.directory,
    )?;
    let command_id = Uuid::new_v4();
    eprintln!("Migrating managed voyage {id} · command {command_id}");
    Ok(serde_json::from_value(
        client
            .request(VesselCommand::ManagedImport {
                command_id,
                session_id: id,
                workspace: saved.session.workspace,
                source_directory: directory.to_owned(),
                expected_revision: saved.revision,
                config_path: Some(config_path),
            })
            .await?,
    )?)
}

pub(super) async fn live(client: &Client, process: ProcessInfo) -> Result<ProcessInfo> {
    if process.state == voyage_protocol::vessel::ProcessState::Stopped {
        return Ok(serde_json::from_value(
            client
                .request(VesselCommand::Restart {
                    command_id: Uuid::new_v4(),
                    session_id: process.session_id,
                    incarnation: process.incarnation,
                })
                .await?,
        )?);
    }
    ensure!(
        matches!(
            process.state,
            voyage_protocol::vessel::ProcessState::Live
                | voyage_protocol::vessel::ProcessState::Suspended
        ),
        "managed owner unavailable; use explicit recover after observing its cleanup"
    );
    Ok(process)
}

pub(super) fn deadline() -> Result<u64> {
    Ok(u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )? + 300000)
}
