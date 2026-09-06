//! Session discovery retains legacy metadata until explicit import.
use anyhow::Result;
use helm::session::SessionStore;
/// Resolve a finite retained-record snapshot without transferring lease ownership.
pub(crate) async fn list_sessions() -> Result<()> {
    let client =
        helm::process_client::local::connect(helm::process_client::cli::default_directory(), true)
            .await?;
    let processes: Vec<voyage_protocol::process::ProcessInfo> = serde_json::from_value(
        client
            .request(voyage_protocol::process::VesselCommand::Catalogue)
            .await?,
    )?;
    let ids: std::collections::HashSet<_> = processes.iter().map(|p| p.session_id).collect();
    for process in processes {
        let snapshot = client
            .forward(
                process.session_id,
                process.incarnation,
                voyage_protocol::process::RuntimeCommand::Snapshot,
            )
            .await;
        let name = snapshot
            .as_ref()
            .ok()
            .and_then(|v| v["name"].as_str())
            .unwrap_or("unnamed");
        println!(
            "{}  {:?}  {}  {}",
            process.session_id,
            process.state,
            helm::process_client::safe(name),
            helm::process_client::safe(&process.workspace.display().to_string())
        );
    }
    for session in SessionStore::default().list().await? {
        if !ids.contains(&session.id) {
            println!(
                "{}  legacy · import on resume  {}",
                session.id,
                helm::process_client::safe(&session.display_name())
            );
        }
    }
    Ok(())
}
