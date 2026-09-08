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
        helm::config::ProviderAccess::ExternalCompatibilityBridge => {
            "external_compatibility_bridge"
        }
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
    let subscription = (config.provider == helm::ProviderKind::CodexSubscription)
        .then(|| probe_codex_compatibility(&config.codex_command));
    let oauth_status = if config.provider == helm::ProviderKind::ChatGptOauth {
        Some(chatgpt_token_store()?.status().await?)
    } else {
        None
    };
    let provider_ready = match (&subscription, &oauth_status) {
        (Some(probe), _) => probe.executable && probe.app_server && probe.logged_in,
        (_, Some(status)) => status.authenticated && status.refreshable,
        (None, None) => !config.api_key_required || config.api_key().is_ok(),
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
        "provider_credential_present": if matches!(config.provider, helm::ProviderKind::CodexSubscription) { serde_json::Value::Null } else if let Some(status) = &oauth_status { serde_json::Value::Bool(status.authenticated) } else if !config.api_key_required { serde_json::Value::Null } else { serde_json::Value::Bool(config.api_key().is_ok()) },
        "codex_compatibility": subscription,
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

#[derive(serde::Serialize)]
struct CodexCompatibilityProbe {
    executable: bool,
    version: Option<String>,
    app_server: bool,
    logged_in: bool,
    remediation: Option<&'static str>,
}

fn probe_codex_compatibility(command: &str) -> CodexCompatibilityProbe {
    let version_output = std::process::Command::new(command)
        .arg("--version")
        .output();
    let executable = version_output
        .as_ref()
        .is_ok_and(|output| output.status.success());
    let version = version_output.ok().and_then(|output| {
        String::from_utf8(output.stdout)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    });
    let app_server = executable
        && std::process::Command::new(command)
            .args(["app-server", "--help"])
            .output()
            .is_ok_and(|output| output.status.success());
    let logged_in = executable
        && std::process::Command::new(command)
            .args(["login", "status"])
            .output()
            .is_ok_and(|output| output.status.success());
    let remediation = if !executable {
        Some("install Codex CLI and ensure codex_command is on PATH")
    } else if !app_server {
        Some("upgrade Codex CLI to a version with app-server support")
    } else if !logged_in {
        Some("run `codex login` interactively, then rerun `helm doctor`")
    } else {
        None
    };
    CodexCompatibilityProbe {
        executable,
        version,
        app_server,
        logged_in,
        remediation,
    }
}
