//! Bounded provider metadata without a session, journal or agent loop.
use anyhow::{Result, ensure};
use std::path::Path;
use voyage_protocol::process::{read_frame, write_frame};
use voyage_protocol::vessel::VesselCommand;

/// Shared with existing-session controls so policy and display validation agree.
pub(crate) async fn discover(
    config: &crate::Config,
    workspace: &Path,
) -> Result<Vec<crate::provider::ModelInfo>> {
    let resolved = crate::runtime_policy::RuntimePolicy::resolve(config, workspace)?;
    let provider = crate::provider::from_config(resolved.config())?;
    let mut models = tokio::time::timeout(
        std::time::Duration::from_secs(20).min(config.timeout()),
        provider.models(),
    )
    .await??;
    if !models.iter().any(|model| model.id == config.model) {
        models.push(crate::provider::ModelInfo::minimal(config.model.clone()));
    }
    let secrets = crate::build::redactor(resolved.config());
    crate::provider::validate_models_for_display(&models, |value| secrets.contains_secret(value))?;
    crate::provider::normalize_models(&mut models);
    ensure!(
        serde_json::to_vec(&models)?.len() <= 1024 * 1024,
        "model catalogue exceeds display limit"
    );
    Ok(models)
}

pub async fn run() -> Result<()> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(12), async {
        let request: VesselCommand = read_frame(&mut tokio::io::stdin()).await?;
        let VesselCommand::DiscoverModels {
            workspace,
            configuration,
        } = request
        else {
            anyhow::bail!("unsupported discovery request");
        };
        ensure!(workspace.is_absolute(), "workspace must be absolute");
        let workspace = workspace.canonicalize()?;
        ensure!(
            serde_json::to_vec(&configuration)?.len() <= 1024 * 1024,
            "model configuration exceeds limit"
        );
        let config = serde_json::from_value::<crate::launch_config::LaunchConfig>(configuration)?
            .resolve(&workspace)?;
        let models = discover(&config, &workspace).await?;
        write_frame(&mut tokio::io::stdout(), &models).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    // Neither provider bodies nor configuration diagnostics enter the frame stream.
    ensure!(matches!(result, Ok(Ok(()))), "model discovery failed");
    Ok(())
}
