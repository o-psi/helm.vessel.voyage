//! Model discovery belongs to a temporary supervised runtime on the executing host.
use super::*;
pub async fn discover(
    config: &crate::Config,
    workspace: &std::path::Path,
) -> Result<Vec<crate::provider::ModelInfo>> {
    let (client, process) = open(config, Some(workspace.to_owned()), None, false, true).await?;
    let result = async {
        let value = client
            .voyage(
                process.session_id,
                process.incarnation,
                VoyageCommand::Controls {
                    run_id: None,
                    section: "models".into(),
                },
            )
            .await?;
        Ok::<_, anyhow::Error>(serde_json::from_value(value["value"].clone())?)
    }
    .await;
    discard(&client, &process).await?;
    result
}
