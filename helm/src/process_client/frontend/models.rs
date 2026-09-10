//! Model discovery must not create a conversation, even temporarily.
use super::*;
pub async fn discover(
    config: &crate::Config,
    workspace: &std::path::Path,
) -> Result<Vec<crate::provider::ModelInfo>> {
    let client = local::connect(super::super::cli::default_directory(), true).await?;
    let capabilities = client.request(VesselCommand::Capabilities).await?;
    ensure!(
        capabilities["features"].as_array().is_some_and(|features| {
            features
                .iter()
                .any(|feature| feature == "sessionless_models")
        }),
        "Update the local Vessel for model discovery without creating a voyage"
    );
    let configuration = voyage_runtime::launch_config::LaunchConfig::capture(config, workspace)?;
    // This owner-local request carries private configuration only to this machine.
    // No launch file, session identity, creation command or deletion is needed.
    let value = client
        .request(VesselCommand::DiscoverModels {
            workspace: workspace.to_path_buf(),
            configuration: serde_json::to_value(configuration)?,
        })
        .await?;
    Ok(serde_json::from_value(value)?)
}
