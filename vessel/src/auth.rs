//! Local credential enrollment; credentials stay on this machine.
use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;
pub(crate) fn chatgpt_token_store() -> Result<voyage_runtime::provider::ChatGptTokenStore> {
    Ok(voyage_runtime::provider::ChatGptTokenStore::new(
        voyage_runtime::provider::ChatGptTokenStore::default_path()?,
    ))
}

pub(crate) fn token_status_json(
    status: &voyage_runtime::provider::TokenStatus,
) -> serde_json::Value {
    serde_json::json!({
        "authenticated": status.authenticated,
        "expires_at": status.expires_at,
        "refreshable": status.refreshable,
    })
}

pub(crate) async fn auth(command: &AuthCommand) -> Result<()> {
    let store = chatgpt_token_store()?;
    match command {
        AuthCommand::Status => {
            println!(
                "{}",
                serde_json::to_string_pretty(&token_status_json(&store.status().await?))?
            );
        }
        AuthCommand::Logout => {
            store.clear().await?;
            println!("ChatGPT credentials removed");
        }
        AuthCommand::ImportCodex { path, force } => {
            match path {
                Some(path) => store.import_codex(path, *force).await?,
                None => store.import_default_codex(*force).await?,
            };
            println!("Imported ChatGPT credentials");
        }
        AuthCommand::Login { device } => {
            let provider = voyage_runtime::provider::ChatGptOauthProvider::from_store(
                store,
                voyage_runtime::provider::OAuthEndpoints::default(),
            );
            if *device {
                let authorization = provider.begin_device().await?;
                eprintln!(
                    "Open {} and enter code {}",
                    authorization.verification_uri, authorization.user_code
                );
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
                loop {
                    match provider.poll_device(&authorization).await {
                        Ok(_) => {
                            println!("Signed in with ChatGPT");
                            break;
                        }
                        Err(voyage_runtime::provider::ProviderError::Unavailable(_))
                            if std::time::Instant::now() < deadline =>
                        {
                            tokio::time::sleep(std::time::Duration::from_secs(
                                authorization.interval.max(1),
                            ))
                            .await;
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            } else {
                provider
                    .login_browser(std::time::Duration::from_secs(600), |url| {
                        eprintln!("Open this URL to sign in:\n{url}");
                        try_open_browser(url);
                    })
                    .await?;
                println!("Signed in with ChatGPT");
            }
        }
    }
    Ok(())
}

pub(crate) fn try_open_browser(url: &str) {
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = std::process::Command::new("xdg-open").arg(url).spawn();
    if let Err(error) = result {
        eprintln!("Could not open a browser automatically: {error}");
    }
}

#[derive(Subcommand)]
pub(crate) enum AuthCommand {
    Status,
    Login {
        /// Use the headless device-code flow instead of browser callback login.
        #[arg(long)]
        device: bool,
    },
    Logout,
    ImportCodex {
        /// Codex auth.json to import; defaults to ~/.codex/auth.json.
        #[arg(long)]
        path: Option<PathBuf>,
        /// Replace existing local ChatGPT credentials.
        #[arg(long)]
        force: bool,
    },
}
