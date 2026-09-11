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
    if let AuthCommand::Accounts { command } = command {
        return accounts(command).await;
    }
    let store = chatgpt_token_store()?;
    match command {
        AuthCommand::Accounts { .. } => unreachable!(),
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
    /// Named executing-host accounts. API input is private terminal only.
    Accounts {
        #[command(subcommand)]
        command: AccountCommand,
    },
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

#[derive(Subcommand)]
pub(crate) enum AccountCommand {
    List,
    Connections,
    /// Add an explicit approved connection, separate from credentials.
    Connect {
        #[arg(long)]
        label: String,
        #[arg(long)]
        endpoint: String,
        /// Comma separated: openai-responses,openai-chat,chatgpt-oauth,anthropic.
        #[arg(long, value_delimiter = ',')]
        transports: Vec<String>,
    },
    Add {
        #[arg(long)]
        connection: uuid::Uuid,
        #[arg(long)]
        account: String,
        /// Bind an execution-host environment NAME, never a key argument.
        #[arg(long)]
        env: Option<String>,
    },
    Login {
        #[arg(long)]
        connection: uuid::Uuid,
        #[arg(long)]
        account: String,
    },
    Status {
        #[arg(long)]
        account: uuid::Uuid,
    },
    Rename {
        #[arg(long)]
        account: uuid::Uuid,
        #[arg(long)]
        alias: String,
        #[arg(long)]
        label: String,
    },
    Logout {
        #[arg(long)]
        account: uuid::Uuid,
        #[arg(long)]
        remove: bool,
    },
    Import {
        #[arg(long)]
        connection: uuid::Uuid,
        #[arg(long)]
        account: String,
        #[arg(long)]
        path: PathBuf,
    },
    RotateApi {
        #[arg(long)]
        account: uuid::Uuid,
        #[arg(long)]
        generation: u64,
        #[arg(long)]
        env: Option<String>,
        /// Owner attestation only, not provider verification of API billing identity.
        #[arg(long)]
        attest_same_identity: bool,
    },
    Reauthenticate {
        #[arg(long)]
        account: uuid::Uuid,
        #[arg(long)]
        generation: u64,
        #[arg(long)]
        path: PathBuf,
        /// Explicitly replace the provider identity; invalidates admitted bindings.
        #[arg(long)]
        replace_identity: bool,
    },
    /// Requires stopping every old binary; a marker cannot fence mixed-version writers.
    MigrateLegacy {
        #[arg(long)]
        old_writers_stopped: bool,
    },
    EnrollmentStatus {
        #[arg(long)]
        enrollment: uuid::Uuid,
    },
    CancelEnrollment {
        #[arg(long)]
        enrollment: uuid::Uuid,
    },
}

fn local_actor() -> voyage_protocol::accounts::EnrollmentActor {
    voyage_protocol::accounts::EnrollmentActor {
        principal: "execution-host-owner".into(),
        workspace: "execution-host".into(),
    }
}

async fn accounts(command: &AccountCommand) -> Result<()> {
    use voyage_runtime::accounts::device::DeviceService;
    use voyage_runtime::accounts::{
        ApiKeyInput, EnrollmentRequest, EnrollmentState, Registry, Transport,
    };
    let registry = Registry::default_host()?;
    let service = DeviceService::new(
        registry.clone(),
        std::sync::Arc::new(|actor, _| actor == &local_actor()),
    );
    match command {
        AccountCommand::List => println!(
            "{}",
            serde_json::to_string_pretty(&registry.list(|_| true)?)?
        ),
        AccountCommand::Connections => println!(
            "{}",
            serde_json::to_string_pretty(&registry.connections()?)?
        ),
        AccountCommand::Connect {
            label,
            endpoint,
            transports,
        } => {
            let transports = transports
                .iter()
                .map(|value| match value.as_str() {
                    "openai-responses" => Ok(Transport::OpenaiResponses),
                    "openai-chat" => Ok(Transport::OpenaiChat),
                    "chatgpt-oauth" => Ok(Transport::ChatgptOauth),
                    "anthropic" => Ok(Transport::Anthropic),
                    _ => anyhow::bail!("unsupported transport"),
                })
                .collect::<Result<Vec<_>>>()?;
            println!(
                "{}",
                serde_json::to_string(&registry.add_connection(
                    label.clone(),
                    endpoint.clone(),
                    transports
                )?)?
            );
        }
        AccountCommand::Add {
            connection,
            account,
            env,
        } => {
            let input = if let Some(name) = env {
                ApiKeyInput::Environment(name.clone())
            } else {
                match voyage_runtime::accounts::private_input::api_key()? {
                    Some(key) => ApiKeyInput::Stored(key),
                    None => {
                        println!("Cancelled; no account created");
                        return Ok(());
                    }
                }
            };
            println!(
                "{}",
                serde_json::to_string(&registry.add_api(
                    *connection,
                    account.clone(),
                    account.clone(),
                    input
                )?)?
            );
        }
        AccountCommand::Login {
            connection,
            account,
        } => {
            use std::io::IsTerminal;
            anyhow::ensure!(
                std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
                "device sign-in requires a private execution-host terminal; use the authenticated private enrollment API for Helm"
            );
            let request = EnrollmentRequest {
                command_id: uuid::Uuid::new_v4(),
                enrollment_id: uuid::Uuid::new_v4(),
                connection_id: *connection,
                alias: account.clone(),
                label: account.clone(),
                actor: local_actor(),
            };
            println!("Enrollment {}", request.enrollment_id);
            let mut status = service.start(request.clone()).await?;
            let private = service.status(request.enrollment_id, &request.actor)?;
            if let (Some(url), Some(code)) = (private.verification_uri, private.user_code) {
                eprintln!("Private sign-in: open {url} and enter {code}");
            }
            if matches!(
                status.state,
                EnrollmentState::Pending | EnrollmentState::Exchanging | EnrollmentState::Starting
            ) {
                status = tokio::select! {
                    result = service.run(request.enrollment_id, &request.actor) => result?,
                    interrupted = tokio::signal::ctrl_c() => {
                        interrupted?;
                        service.cancel(uuid::Uuid::new_v4(), request.enrollment_id, &request.actor)?
                    }
                };
            }
            println!("{}", serde_json::to_string(&status)?);
            if status.state != EnrollmentState::Succeeded {
                anyhow::bail!(
                    "sign-in did not publish an account; inspect the original enrollment ID (uncertain effects are not replayed)"
                );
            }
        }
        AccountCommand::Status { account } => println!(
            "{}",
            serde_json::to_string(&registry.list(|a| a.id == *account)?)?
        ),
        AccountCommand::Rename {
            account,
            alias,
            label,
        } => registry.rename(*account, alias.clone(), label.clone())?,
        AccountCommand::Logout { account, remove } => {
            registry.logout(*account, *remove)?;
            println!(
                "Local binding revoked. Already dispatched requests cannot be recalled; upstream tokens were not revoked."
            );
        }
        AccountCommand::Import {
            connection,
            account,
            path,
        } => {
            let tokens =
                voyage_runtime::provider::ChatGptTokenStore::read_import_tokens(path).await?;
            println!(
                "{}",
                serde_json::to_string(&registry.add_oauth(
                    *connection,
                    account.clone(),
                    account.clone(),
                    tokens
                )?)?
            );
        }
        AccountCommand::RotateApi {
            account,
            generation,
            env,
            attest_same_identity,
        } => {
            let input = if let Some(name) = env {
                ApiKeyInput::Environment(name.clone())
            } else {
                match voyage_runtime::accounts::private_input::api_key()? {
                    Some(key) => ApiKeyInput::Stored(key),
                    None => {
                        println!("Cancelled; credentials unchanged");
                        return Ok(());
                    }
                }
            };
            println!(
                "{}",
                serde_json::to_string(&registry.reauthenticate_api(
                    *account,
                    *generation,
                    input,
                    *attest_same_identity
                )?)?
            );
        }
        AccountCommand::Reauthenticate {
            account,
            generation,
            path,
            replace_identity,
        } => {
            let tokens =
                voyage_runtime::provider::ChatGptTokenStore::read_import_tokens(path).await?;
            println!(
                "{}",
                serde_json::to_string(&registry.reauthenticate_oauth(
                    *account,
                    *generation,
                    tokens,
                    *replace_identity
                )?)?
            );
        }
        AccountCommand::MigrateLegacy {
            old_writers_stopped,
        } => println!(
            "{}",
            serde_json::to_string(&registry.migrate_legacy_oauth(*old_writers_stopped)?)?
        ),
        AccountCommand::EnrollmentStatus { enrollment } => {
            // CLI status is deliberately safe; code disclosure is confined to Login's private terminal.
            println!(
                "{}",
                serde_json::to_string(&service.status(*enrollment, &local_actor())?.status)?
            );
        }
        AccountCommand::CancelEnrollment { enrollment } => println!(
            "{}",
            serde_json::to_string(&service.cancel(
                uuid::Uuid::new_v4(),
                *enrollment,
                &local_actor()
            )?)?
        ),
    }
    Ok(())
}
