use super::*;
use voyage_protocol::process::RuntimeCommand;
pub(super) async fn configure(
    state: &Arc<State>,
    command: RuntimeCommand,
    authorization: super::authorization::Authorization,
) -> Result<serde_json::Value> {
    ensure!(
        authorization.grant.is_none(),
        "configuration requires executing-account owner authority"
    );
    let RuntimeCommand::Configure {
        command_id,
        config_path,
        expected_revision,
        ..
    } = &command
    else {
        anyhow::bail!("not configure")
    };
    let _admission = state.admission.lock().await;
    ensure!(
        !state.shutdown.is_cancelled() && state.active.lock().await.is_none(),
        "configuration requires idle runtime"
    );
    if let Some(receipt) = state.owner.process_receipt(*command_id).await? {
        return Ok(receipt);
    }
    ensure!(
        config_path.is_absolute(),
        "configuration path must be absolute"
    );
    let mut config = bootstrap::load_config(Some(config_path), &state.registration.workspace)?;
    bootstrap::limit_participant(&mut config, &state.registration)?;
    let secrets = crate::build::redactor(&config);
    crate::provider::validate_models_for_display(
        &[crate::provider::ModelInfo::minimal(config.model.clone())],
        |value| secrets.contains_secret(value),
    )?;
    let launch =
        crate::launch_config::LaunchConfig::capture(&config, &state.registration.workspace)?;
    ensure!(
        state.owner.snapshot().await?.revision == *expected_revision,
        "session revision conflict"
    );
    if !launch.matches_config(&*state.config.read().await)? {
        state
            .controls
            .close_for_command(&state.owner, &command)
            .await?;
    }
    let receipt = state
        .owner
        .configure(
            command,
            serde_json::to_string(&launch)?,
            config.model.clone(),
        )
        .await?;
    *state.config.write().await = config;
    Ok(receipt)
}
