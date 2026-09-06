use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use helm::{Config, config::ApprovalMode};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};
use tracing_subscriber::EnvFilter;
use voyage_runtime::build::redactor;
mod attachment_ui;
mod auth;
mod catalogue;
mod cli;
mod diagnostics;
mod logging;
mod workflow_input;
use attachment_ui::{attachment_connect, attachment_interrupt, attachment_notice};
use auth::auth;
use catalogue::list_sessions;
use cli::*;
use diagnostics::{doctor, list_models, print_config};
use logging::{command_uses_full_screen_tui, tui_log_file};
use workflow_input::prepare_workflow;
mod managed;
mod remote_consent;
mod remote_worker;
#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if std::env::args_os().any(|arg| arg == "attachment")
                && !matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
        {
            eprintln!("invalid attachment arguments; use helm attachment --help");
            std::process::exit(2);
        }
        Err(error) => error.exit(),
    };
    if matches!(&cli.command, Some(Command::Connect(_))) {
        let creating = matches!(&cli.command, Some(Command::Connect(args))
            if args.command.as_ref().is_some_and(helm::process_client::cli::ConnectedCommand::uses_host_workspace));
        anyhow::ensure!(
            cli.config.is_none()
                && cli.set.is_empty()
                && cli.model.is_none()
                && cli.provider.is_none()
                && (cli.workspace.is_none() || creating)
                && cli.access.is_none()
                && cli.approval.is_none()
                && cli.policy.policy_profile.is_none(),
            "connected voyages resolve execution configuration on their host; use connect command options"
        );
        if let Some(Command::Connect(args)) = cli.command {
            return helm::process_client::cli::run(args).await;
        }
        unreachable!();
    }
    let configuration_explicit = cli.config.is_some()
        || !cli.set.is_empty()
        || cli.provider.is_some()
        || cli.access.is_some()
        || cli.approval.is_some()
        || cli.policy.policy_profile.is_some();
    let is_chat = matches!(&cli.command, None | Some(Command::Chat { .. }));
    let remembered = if is_chat && cli.config.is_none() {
        helm::chat_preferences::load()?
    } else {
        None
    };
    if let Some(state) = remembered
        .as_ref()
        .and_then(|config| config.chat_preferences.as_ref())
    {
        let arguments: Vec<String> = std::env::args_os()
            .filter_map(|arg| arg.into_string().ok())
            .collect();
        if !arguments.iter().any(|arg| {
            arg == "--verbose"
                || arg == "-v"
                || arg.starts_with("--verbose=")
                || arg.starts_with("-v=")
        }) {
            cli.verbose = state.presentation.verbose;
        }
        if !arguments
            .iter()
            .any(|arg| arg == "--log-format" || arg.starts_with("--log-format="))
        {
            cli.log_format = if state.presentation.log_format == "json" {
                LogFormat::Json
            } else {
                LogFormat::Text
            };
        }
        if !arguments
            .iter()
            .any(|arg| arg == "--plain" || arg.starts_with("--plain="))
        {
            match &mut cli.command {
                Some(Command::Chat { plain, .. }) => *plain = state.presentation.plain,
                None => {
                    cli.command = Some(Command::Chat {
                        resume: None,
                        plain: state.presentation.plain,
                    })
                }
                _ => {}
            }
        }
    }
    if cli.policy.policy_profile.is_some() {
        anyhow::ensure!(
            matches!(
                &cli.command,
                None | Some(
                    Command::Run { .. }
                        | Command::Chat { .. }
                        | Command::Models { .. }
                        | Command::Github(_)
                )
            ) || matches!(&cli.command, Some(Command::Workflow(args)) if matches!(args.command, helm::workflow::WorkflowCommand::Run(_)))
                || matches!(&cli.command, Some(Command::Managed(args)) if !args.administrative())
                || matches!(&cli.command, Some(Command::RemoteWorker(args)) if !args.recover),
            "policy selection requires an execution or model-discovery invocation"
        );
    }
    // Enrollment never loads provider config or initializes runtime/session logs.
    if let Some(Command::Attachment(args)) = cli.command {
        if matches!(
            &args.command,
            helm::attachment::cli::AttachmentCommand::Connect(_)
        ) {
            return attachment_connect(args).await;
        }
        let prompt = helm::attachment::cli::prompt::PromptControl::default();
        return tokio::select! { biased;
            _=attachment_interrupt()=>{
                let notice = if prompt.cancel_and_restore().is_err(){"attachment terminal restoration failed"}else{"attachment operation interrupted; inspect status and resume pending work"};
                attachment_notice(notice).await;std::process::exit(130)
            },
            result=helm::attachment::cli::run_with_prompt(args,prompt.clone())=>match result {
                Err(helm::attachment::cli::CliError::Cancelled)=>{attachment_notice("attachment input cancelled").await;std::process::exit(130)},
                other=>other.map_err(anyhow::Error::from),
            },
        };
    }
    if matches!(&cli.command, Some(Command::RemoteConsent(_))) {
        let Some(Command::RemoteConsent(args)) = cli.command else {
            unreachable!()
        };
        return remote_consent::run(args)
            .await
            .map_err(remote_consent::safe_error);
    }
    if matches!(&cli.command,Some(Command::RemoteWorker(args)) if args.recover) {
        let Some(Command::RemoteWorker(args)) = cli.command else {
            unreachable!()
        };
        return remote_worker::recover(args).await.map_err(|_| {
            anyhow::anyhow!(
                "remote recovery failed; inspect the dedicated installation and exact run"
            )
        });
    }
    if matches!(&cli.command, Some(Command::Managed(args)) if args.administrative()) {
        let Some(Command::Managed(args)) = cli.command else {
            unreachable!()
        };
        return managed::run(args, None, cli.workspace, false)
            .await
            .map_err(managed::safe_error);
    }
    if matches!(&cli.command, Some(Command::Policy(args)) if !args.needs_config()) {
        let Some(Command::Policy(args)) = cli.command else {
            unreachable!()
        };
        return helm::policy_profile::cli::run(
            args,
            &cli.policy,
            None,
            None,
            Default::default(),
            cli.config.as_deref(),
        );
    }
    if let Some(Command::Extension(args)) = cli.command {
        return helm::extensions::cli::run(args, cli.workspace).await;
    }
    let filter = if cli.verbose {
        "helm=debug"
    } else {
        "helm=warn"
    };
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into());
    let term = std::env::var("TERM").ok();
    if command_uses_full_screen_tui(
        &cli,
        io::stdin().is_terminal(),
        io::stdout().is_terminal(),
        term.as_deref(),
    ) {
        let log = tui_log_file()?;
        match cli.log_format {
            LogFormat::Text => tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(log)
                .init(),
            LogFormat::Json => tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_writer(log)
                .init(),
        }
    } else {
        match cli.log_format {
            LogFormat::Text => tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_writer(io::stderr)
                .init(),
            LogFormat::Json => tracing_subscriber::fmt()
                .json()
                .with_env_filter(env_filter)
                .with_writer(io::stderr)
                .init(),
        }
    }
    if let Some(Command::LocalProvider { command }) = cli.command {
        return helm::local_provider::run(command).await;
    }
    if matches!(&cli.command, Some(Command::Workflow(args)) if !matches!(args.command, helm::workflow::WorkflowCommand::Run(_)))
    {
        let Some(Command::Workflow(args)) = cli.command else {
            unreachable!()
        };
        let json = args.json;
        let workspace = cli
            .workspace
            .unwrap_or(std::env::current_dir()?)
            .canonicalize()?;
        if let Some(prepared) = prepare_workflow(args, &workspace).await? {
            helm::workflow::print_value(
                &serde_json::json!({"prompt":prepared.prompt,"workflow":prepared.invocation}),
                json,
            )?;
        }
        return Ok(());
    }
    let defaults_command = matches!(&cli.command,Some(Command::Policy(args)) if matches!(args.command,helm::policy_profile::cli::PolicyCommand::Defaults(_)));
    let diagnostic_command = matches!(&cli.command, Some(Command::Config | Command::Doctor));
    let config_error = |error: anyhow::Error| {
        if defaults_command {
            anyhow::anyhow!("configuration input or override is invalid or unavailable")
        } else if diagnostic_command {
            anyhow::anyhow!(
                "configuration input or override is invalid or unavailable; check the config file and --set arguments (input details concealed)"
            )
        } else {
            error
        }
    };
    let mut config = if let Some(config) = remembered {
        Ok(config)
    } else if defaults_command {
        helm::policy_profile::defaults::cli::load_config(cli.config.as_deref())
    } else {
        Config::load(cli.config.as_deref())
    }
    .map_err(&config_error)?;
    let set_overrides_model = cli.set.iter().any(|assignment| {
        assignment
            .split_once('=')
            .is_some_and(|(key, _)| matches!(key.trim(), "model" | "provider"))
    });
    for assignment in &cli.set {
        let (key, value) = assignment
            .split_once('=')
            .with_context(|| format!("invalid --set `{assignment}`; expected KEY=VALUE"))
            .map_err(&config_error)?;
        config
            .apply_override(key.trim(), value.trim())
            .map_err(&config_error)?;
    }
    let provider_overridden = cli.provider.is_some();
    if let Some(provider) = cli.provider {
        config.select_provider(provider.into());
    }
    let model_overridden = cli.model.is_some() || provider_overridden || set_overrides_model;
    if let Some(model) = cli.model {
        config.model = model;
    }
    let explicit_access = cli.approval.is_some() || cli.access.is_some();
    if let Some(approval) = cli.approval {
        config.access = None;
        config.approval = match approval {
            ApprovalArg::Always => ApprovalMode::Always,
            ApprovalArg::OnRisk => ApprovalMode::OnRisk,
            ApprovalArg::Never => ApprovalMode::Never,
        };
    }
    if let Some(access) = cli.access {
        config.access = Some(access.into());
    }
    let policy_explicit = if config.policy_defaults.is_some()
        || cli.policy.policy_profile.is_some()
        || config
            .chat_preferences
            .as_ref()
            .is_some_and(|state| state.profile.is_some())
        || matches!(&cli.command, Some(Command::Policy(_)))
    {
        helm::policy_profile::cli::explicit(&config, &cli.set, explicit_access)?
    } else {
        Default::default()
    };
    // A new invocation changes only the explicitly supplied fields; remembered
    // policy overrides remain active when unrelated settings change.
    let policy_explicit = if config.chat_preferences.is_some() {
        let previous = &config.policy_explicit;
        helm::policy_profile::Overrides {
            access: policy_explicit.access.or(previous.access),
            unattended: policy_explicit
                .unattended
                .or_else(|| previous.unattended.clone()),
            read_roots: policy_explicit
                .read_roots
                .or_else(|| previous.read_roots.clone()),
            write_roots: policy_explicit
                .write_roots
                .or_else(|| previous.write_roots.clone()),
            deny_commands: policy_explicit
                .deny_commands
                .or_else(|| previous.deny_commands.clone()),
            inherit_env: policy_explicit
                .inherit_env
                .or_else(|| previous.inherit_env.clone()),
            github_enabled: policy_explicit.github_enabled.or(previous.github_enabled),
        }
    } else {
        policy_explicit
    };
    config.policy_explicit = policy_explicit.clone();
    if cli.policy.policy_profile.is_some() {
        let workspace = config.resolve_workspace(cli.workspace.clone())?;
        cli.policy
            .apply(&mut config, &workspace, policy_explicit.clone())?;
    }
    if is_chat {
        let mut state = config.chat_preferences.take().unwrap_or_default();
        state.presentation.verbose = cli.verbose;
        state.presentation.log_format = match cli.log_format {
            LogFormat::Text => "text",
            LogFormat::Json => "json",
        }
        .into();
        state.presentation.plain = matches!(&cli.command, Some(Command::Chat { plain: true, .. }));
        if cli.policy.policy_profile.is_some() {
            state.profile = None;
        }
        config.chat_preferences = Some(state);
        if let Some(workspace) = &cli.workspace {
            config.workspace = Some(workspace.canonicalize()?);
        }
    }
    match cli.command.unwrap_or(Command::Chat {
        resume: None,
        plain: false,
    }) {
        Command::Connect(_) => {
            unreachable!("connected client handled before execution configuration")
        }
        Command::Github(args) => {
            helm::process_client::frontend::github::run(
                config,
                cli.workspace,
                args.session,
                args.args,
                configuration_explicit,
            )
            .await
        }
        Command::Extension(_) => {
            unreachable!("extension command handled before provider configuration")
        }
        Command::Inference(args) => {
            let workspace = config.resolve_workspace(cli.workspace)?;
            helm::inference::cli::run(args, &workspace, &redactor(&config)).await
        }
        Command::Policy(args) => {
            let workspace = config.resolve_workspace(cli.workspace)?;
            helm::policy_profile::cli::run(
                args,
                &cli.policy,
                Some(&config),
                Some(&workspace),
                policy_explicit,
                cli.config.as_deref(),
            )
        }
        Command::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "helm", &mut io::stdout());
            Ok(())
        }
        Command::Manpage => {
            clap_mangen::Man::new(Cli::command()).render(&mut io::stdout())?;
            Ok(())
        }
        Command::Config => {
            print_config(&config)?;
            Ok(())
        }
        Command::Workflow(args) => {
            anyhow::ensure!(
                !args.json,
                "workflow run streams ordinary agent output; --json is for list, inspect, validate and preview"
            );
            let workspace = config.resolve_workspace(cli.workspace)?;
            let user_directory = args.user_directory.clone();
            let prepared = prepare_workflow(args, &workspace)
                .await?
                .context("workflow run required")?;
            helm::process_client::frontend::workflow::run(
                config,
                workspace,
                prepared,
                user_directory,
                model_overridden,
            )
            .await
        }
        Command::Onboard(args) => helm::onboarding::run(args, &config, cli.workspace),
        Command::Managed(args) => managed::run(args, Some(config), cli.workspace, model_overridden)
            .await
            .map_err(managed::safe_error),
        Command::RemoteConsent(_) => {
            unreachable!("administrative command handled before provider setup")
        }
        Command::RemoteWorker(args) => remote_worker::run(args, config, cli.workspace)
            .await
            .map_err(|_| {
                anyhow::anyhow!("remote worker stopped; inspect dedicated local session state")
            }),
        Command::Models { json } => list_models(&config, cli.workspace, json).await,
        Command::Doctor => doctor(&config, cli.workspace).await,
        Command::Auth { command } => auth(command, &config).await,
        Command::Attachment(_) | Command::LocalProvider { .. } => {
            unreachable!("handled before runtime initialization")
        }
        Command::Sessions => list_sessions().await,
        Command::Run {
            prompt,
            resume,
            no_save,
        } => {
            helm::process_client::frontend::run(
                config,
                cli.workspace,
                resume,
                prompt.join(" "),
                no_save,
                model_overridden,
                configuration_explicit,
            )
            .await
        }
        Command::Chat { resume, plain } => {
            let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
            let full_screen = interactive
                && std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb");
            helm::process_client::frontend::chat(
                config,
                cli.workspace,
                resume,
                plain || !full_screen,
                model_overridden,
                configuration_explicit,
            )
            .await
        }
    }
}

fn safe_diagnostic(text: &str) -> String {
    text.chars().filter(|ch|!ch.is_control()&&!matches!(*ch,'\u{7f}'|'\u{80}'..='\u{9f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')).collect()
}
