use super::*;
use anyhow::Context;

pub(super) async fn open(
    client: &Client,
    config: &crate::Config,
    workspace: Option<PathBuf>,
    reference: &str,
) -> Result<ProcessInfo> {
    let processes: Vec<ProcessInfo> =
        serde_json::from_value(client.request(VesselCommand::Catalogue).await?)?;
    let mut matches = Vec::new();
    for process in processes {
        let id = process.session_id.to_string();
        if id == reference || id.starts_with(reference) {
            matches.push(process);
            continue;
        }
        if let Ok(snapshot) = client
            .forward(
                process.session_id,
                process.incarnation,
                RuntimeCommand::Snapshot,
            )
            .await
            && snapshot["name"].as_str() == Some(reference)
        {
            matches.push(process);
        }
    }
    ensure!(
        matches.len() <= 1,
        "session reference is ambiguous; use the full UUID"
    );
    if let Some(process) = matches.pop() {
        if let Some(workspace) = workspace {
            ensure!(
                workspace.canonicalize()? == process.workspace,
                "resume workspace differs from canonical owner"
            );
        }
        if process.state == voyage_protocol::process::ProcessState::Stopped {
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
                voyage_protocol::process::ProcessState::Live
                    | voyage_protocol::process::ProcessState::Suspended
            ),
            "owner unavailable or cleanup unconfirmed; inspect it before recovery"
        );
        return Ok(process);
    }
    let source_directory = crate::config::default_data_dir().join("sessions");
    if let Ok(id) = reference.parse::<Uuid>() {
        let plan = voyage_runtime::server::bootstrap::import_plan(
            voyage_runtime::server::bootstrap::ImportPlanArgs {
                source_directory: source_directory.clone(),
                session: id,
            },
        )?;
        ensure!(
            plan["transferred"] != true,
            "session was already transferred; reconnect to its recorded Vessel instead of creating another owner"
        );
    }
    let session = crate::session::SessionStore::new(source_directory.clone())
        .load_reference(reference)
        .await?;
    if let Some(workspace) = workspace {
        ensure!(
            workspace.canonicalize()? == session.workspace.canonicalize()?,
            "resume workspace differs from source session"
        );
    }
    let plan = voyage_runtime::server::bootstrap::import_plan(
        voyage_runtime::server::bootstrap::ImportPlanArgs {
            source_directory: source_directory.clone(),
            session: session.id,
        },
    )?;
    let config_path = super::launch::persist(config, &session.workspace, &client.directory)?;
    let command_id = Uuid::new_v4();
    eprintln!(
        "Migrating voyage {} to its independent owner · command {command_id}",
        session.id
    );
    Ok(serde_json::from_value(
        client
            .request(VesselCommand::Import {
                command_id,
                session_id: session.id,
                workspace: session.workspace,
                source_directory,
                expected_revision: plan["expected_revision"]
                    .as_u64()
                    .context("import revision missing")?,
                source_sha256: plan["source_sha256"]
                    .as_str()
                    .context("import digest missing")?
                    .into(),
                config_path: Some(config_path),
            })
            .await?,
    )?)
}
