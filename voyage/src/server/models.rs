//! Bounded provider metadata without a session, journal or agent loop.
use anyhow::{Result, ensure};
use voyage_protocol::model_discovery::{ErrorResponse, Failure};
fn failure(f: Failure) -> anyhow::Error {
    anyhow::anyhow!("[model_catalog:{}]", f.code())
}
use std::path::Path;
use voyage_protocol::process::{read_frame, write_frame};
use voyage_protocol::vessel::VesselCommand;

/// Shared with existing-session controls so policy and display validation agree.
pub(crate) async fn discover(
    config: &crate::Config,
    workspace: &Path,
) -> Result<Vec<crate::provider::ModelInfo>> {
    let resolved = crate::runtime_policy::RuntimePolicy::resolve(config, workspace)
        .map_err(|_| failure(Failure::Policy))?;
    let provider = crate::provider::from_config(resolved.config())
        .map_err(|_| failure(Failure::Configuration))?;
    let mut models = tokio::time::timeout(
        std::time::Duration::from_secs(20).min(config.timeout()),
        provider.models(),
    )
    .await
    .map_err(|_| failure(Failure::Timeout))?
    .map_err(|e| {
        failure(match e.category() {
            "authentication" => Failure::Authentication,
            "rate_limit" | "usage_limit" => Failure::RateLimit,
            "transport" | "connection" => Failure::Network,
            "timeout" | "transport_timeout" => Failure::Timeout,
            "invalid_response" => Failure::InvalidResponse,
            _ => Failure::Unavailable,
        })
    })?;
    if !models.iter().any(|model| model.id == config.model) {
        models.push(crate::provider::ModelInfo::minimal(config.model.clone()));
    }
    let secrets = crate::build::redactor(resolved.config());
    crate::provider::validate_models_for_display(&models, |value| secrets.contains_secret(value))
        .map_err(|_| failure(Failure::DisplayValidation))?;
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
        let workspace = workspace
            .canonicalize()
            .map_err(|_| failure(Failure::Workspace))?;
        ensure!(
            serde_json::to_vec(&configuration)?.len() <= 1024 * 1024,
            "model configuration exceeds limit"
        );
        let config = serde_json::from_value::<crate::launch_config::LaunchConfig>(configuration)?
            .resolve(&workspace)
            .map_err(|_| failure(Failure::Configuration))?;
        let models = discover(&config, &workspace).await?;
        write_frame(&mut tokio::io::stdout(), &models).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    // Neither provider bodies nor configuration diagnostics enter the frame stream.
    match result {
        Ok(Ok(())) => Ok(()),
        other => {
            let category = match other {
                Err(_) => Failure::Timeout,
                Ok(Err(e)) => {
                    Failure::from_diagnostic(&e.to_string()).unwrap_or(Failure::Configuration)
                }
                _ => unreachable!(),
            };
            write_frame(
                &mut tokio::io::stdout(),
                &ErrorResponse {
                    model_catalog_error: category,
                },
            )
            .await?;
            Ok(())
        }
    }
}
