//! Configuration and bounded local dependency diagnostics.
use crate::auth::{chatgpt_token_store, token_status_json};
use anyhow::Result;
use helm::Config;
use std::path::PathBuf;
pub(crate) fn print_config(config: &Config) -> Result<()> {
    let profile = config.provider_profile();
    let access = match profile.access {
        helm::config::ProviderAccess::NativePublicApi => "native_public_api",
        helm::config::ProviderAccess::NativeChatgptOauth => "native_chatgpt_oauth",
    };
    println!("# provider_access = {access}");
    println!("# credential_requirement = {}", profile.credential);
    println!("# billing = {}", profile.billing);
    println!(
        "# endpoint_diagnostics = {}",
        helm::local_provider::diagnostics(config)
    );
    println!("# Secret values are concealed; this display cannot restore secret bindings.");
    println!("{}", config.diagnostic_toml()?);
    Ok(())
}

pub(crate) async fn list_models(
    config: &Config,
    workspace: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let workspace = config.resolve_workspace(workspace)?;
    let models = helm::process_client::frontend::models::discover(config, &workspace).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&models)?);
    } else {
        for model in models {
            println!(
                "{}{}\t{}\t{}",
                if model.id == config.model { "* " } else { "  " },
                model.id,
                model.display_name,
                model.reasoning_efforts.join(",")
            );
        }
    }
    Ok(())
}

pub(crate) async fn doctor(config: &Config, workspace: Option<PathBuf>) -> Result<()> {
    let workspace = config.resolve_workspace(workspace)?;
    let oauth_status = if config.provider == helm::ProviderKind::ChatGptOauth {
        Some(chatgpt_token_store()?.status().await?)
    } else {
        None
    };
    let provider_ready = match &oauth_status {
        Some(status) => status.authenticated && status.refreshable,
        None => !config.api_key_required || config.api_key().is_ok(),
    };
    let profile = config.provider_profile();
    let sandbox = helm::sandbox::diagnostics(config, &workspace);
    let sandbox_ready = sandbox["ready"].as_bool().unwrap_or(false);
    let report = serde_json::json!({
        "status": if provider_ready && sandbox_ready { "ok" } else { "action_required" },
        "version": env!("CARGO_PKG_VERSION"),
        "workspace": workspace,
        "workspace_readable": workspace.is_dir(),
        "provider_ready": provider_ready,
        "provider": profile,
        "endpoint_diagnostics": helm::local_provider::diagnostics(config),
        "native_chatgpt_oauth": oauth_status.as_ref().map(token_status_json),
        "provider_credential_present": if let Some(status) = &oauth_status { serde_json::Value::Bool(status.authenticated) } else if !config.api_key_required { serde_json::Value::Null } else { serde_json::Value::Bool(config.api_key().is_ok()) },
        "sessions_directory": helm::config::default_data_dir().join("sessions"),
        "access": config.access_mode(),
        "sandbox": sandbox,
        "unattended_approval": config.unattended_approval,
        "inherited_environment": config.inherit_env,
        "mcp_servers": config.mcp_servers.keys().collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
