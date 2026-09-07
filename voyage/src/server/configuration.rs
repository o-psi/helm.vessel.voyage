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
    let (RuntimeCommand::Configure {
        command_id,
        expected_revision,
        ..
    }
    | RuntimeCommand::SetAccess {
        command_id,
        expected_revision,
        ..
    }) = &command
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
    let mut config = match &command {
        RuntimeCommand::Configure { config_path, .. } => {
            ensure!(
                config_path.is_absolute(),
                "configuration path must be absolute"
            );
            bootstrap::load_config(Some(config_path), &state.registration.workspace)?
        }
        RuntimeCommand::SetAccess { access, .. } => {
            let mut config = state.config.read().await.clone();
            let mode = match access.as_str() {
                "read-only" => crate::config::AccessMode::ReadOnly,
                "approval" => crate::config::AccessMode::Approval,
                "unrestricted" => crate::config::AccessMode::Unrestricted,
                _ => anyhow::bail!("use read-only, approval or unrestricted"),
            };
            // Validate existing sources before applying the one explicit override.
            crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?;
            config.model = state.owner.snapshot().await?.session.model.clone();
            config.access = Some(mode);
            config.policy_explicit.access = Some(mode);
            if let Some(selection) = config.policy_profile.take() {
                let mut request = selection.request().clone();
                request.explicit.access = Some(mode);
                let preview = crate::policy_profile::selection::Selection::preview(
                    &config,
                    &state.registration.workspace,
                    &request,
                )?;
                // Owner's confirmed command changes only access; other profile fields stay bound.
                config.policy_profile = Some(crate::policy_profile::selection::Selection::bind(
                    &config,
                    &state.registration.workspace,
                    request,
                    Some(&preview.transition_digest),
                )?);
            }
            config
        }
        _ => unreachable!(),
    };
    bootstrap::limit_participant(&mut config, &state.registration)?;
    if let RuntimeCommand::SetAccess { access, .. } = &command {
        ensure!(
            config.access_mode().to_string() == *access,
            "requested access exceeds parent policy"
        );
        let effective =
            crate::runtime_policy::RuntimePolicy::resolve(&config, &state.registration.workspace)?;
        ensure!(
            serde_json::to_value(effective.policy().access_mode())? == *access,
            "requested access is restricted by the executing machine or parent policy"
        );
    }
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
