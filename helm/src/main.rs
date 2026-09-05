use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use helm::{
    Agent, AgentEvent, Config, EventSink,
    agent::RetryPolicy,
    config::{AccessMode, ApprovalMode, UnattendedApprovalMode},
    policy::Policy,
    provider,
    session::{Session, SessionStore},
    subagent::{
        AgentBudget, AgentPolicy, ApprovalPolicy, ExecutionContext, RuntimeLimits,
        SubagentExecutor, SubagentResult, SubagentRuntime, SubagentTool, WorktreeManager,
    },
    todo::{TodoScope, TodoStore},
    tools::{
        ApprovalOutcome, ApprovalRequest, Approver, InteractionMode, Redactor, TodoTool,
        ToolContext, ToolRegistry, UnattendedApprover,
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock, Weak},
};
use tracing_subscriber::EnvFilter;
#[derive(Parser)]
#[command(version, about = "A general-purpose LLM harness for terminal work")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Override a configuration value for this invocation (`KEY=VALUE`).
    #[arg(long = "set", global = true, value_name = "KEY=VALUE")]
    set: Vec<String>,
    #[arg(long, global = true)]
    model: Option<String>,
    /// Provider transport (`openai` retains its legacy Chat Completions behavior).
    #[arg(long, global = true, value_enum)]
    provider: Option<ProviderArg>,
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true, value_enum, hide = true)]
    approval: Option<ApprovalArg>,
    /// Agent authority: inspect only, ask on consequential actions, or proceed without prompts.
    #[arg(long, global = true, value_enum, conflicts_with = "approval")]
    access: Option<AccessArg>,
    #[arg(short, long, global = true)]
    verbose: bool,
    #[arg(long, global = true, value_enum, default_value = "text")]
    log_format: LogFormat,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, clap::ValueEnum)]
enum ApprovalArg {
    Always,
    OnRisk,
    Never,
}
#[derive(Clone, Copy, clap::ValueEnum)]
enum AccessArg {
    ReadOnly,
    Approval,
    Unrestricted,
}

impl From<AccessArg> for AccessMode {
    fn from(value: AccessArg) -> Self {
        match value {
            AccessArg::ReadOnly => Self::ReadOnly,
            AccessArg::Approval => Self::Approval,
            AccessArg::Unrestricted => Self::Unrestricted,
        }
    }
}
#[derive(Clone, clap::ValueEnum)]
enum ProviderArg {
    #[value(name = "openai-responses")]
    OpenaiResponses,
    #[value(name = "openai-chat", alias = "openai", alias = "openai-compatible")]
    OpenaiChat,
    #[value(name = "chatgpt-oauth")]
    ChatGptOauth,
    Anthropic,
    #[value(name = "codex-compatibility", alias = "codex-subscription")]
    CodexCompatibility,
}

impl From<ProviderArg> for helm::ProviderKind {
    fn from(value: ProviderArg) -> Self {
        match value {
            ProviderArg::OpenaiResponses => Self::OpenaiResponses,
            ProviderArg::OpenaiChat => Self::OpenaiChat,
            ProviderArg::ChatGptOauth => Self::ChatGptOauth,
            ProviderArg::Anthropic => Self::Anthropic,
            ProviderArg::CodexCompatibility => Self::CodexSubscription,
        }
    }
}
#[derive(Clone, Copy, clap::ValueEnum)]
enum LogFormat {
    Text,
    Json,
}
#[derive(Subcommand)]
enum Command {
    /// Discover, inspect and run saved nonsecret workflows.
    Workflow(helm::workflow::WorkflowArgs),
    /// Inspect a repository and explicitly review generated project guidance.
    Onboard(helm::onboarding::OnboardArgs),
    /// Manage dedicated Vessel enrollment; no worker is started.
    Attachment(helm::attachment::cli::AttachmentArgs),
    /// Manage Helm's native ChatGPT subscription credentials.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
    Run {
        #[arg(required = true)]
        prompt: Vec<String>,
        #[arg(long)]
        resume: Option<String>,
        #[arg(long)]
        no_save: bool,
    },
    Chat {
        #[arg(long)]
        resume: Option<String>,
        /// Use the line-oriented interface, even when attached to a terminal.
        #[arg(long)]
        plain: bool,
    },
    Sessions,
    /// List models available to the configured provider/account.
    Models {
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    Config,
    /// Explicit local/compatible endpoint setup and discovery.
    LocalProvider {
        #[command(subcommand)]
        command: helm::local_provider::Command,
    },
    /// Generate a shell completion script on stdout.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
    /// Generate a roff manpage on stdout.
    Manpage,
    /// Check configuration and local runtime dependencies without contacting a model.
    Doctor,
}

#[derive(Subcommand)]
enum AuthCommand {
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
        /// Replace existing Helm credentials.
        #[arg(long)]
        force: bool,
    },
}

struct Terminal {
    streamed: std::sync::Mutex<PlainStream>,
    styled: bool,
    width: usize,
}
#[derive(Default)]
struct PlainStream {
    buffer: String,
    emitted: bool,
}
impl Default for Terminal {
    fn default() -> Self {
        Self::for_output(
            io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
        )
    }
}
impl Terminal {
    fn for_output(terminal: bool, no_color: bool) -> Self {
        Self {
            streamed: std::sync::Mutex::new(PlainStream::default()),
            styled: terminal && !no_color,
            width: std::env::var("COLUMNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100)
                .max(20),
        }
    }
    fn assistant_delta(&self, text: &str) -> String {
        let mut state = self
            .streamed
            .lock()
            .expect("terminal stream state poisoned");
        let safe = safe_assistant(text);
        state.buffer.push_str(&safe);
        if self.styled {
            String::new()
        } else {
            state.emitted = true;
            safe
        }
    }
    fn assistant_completion(&self, text: &str) -> String {
        let mut streamed = self
            .streamed
            .lock()
            .expect("terminal stream state poisoned");
        if !self.styled && streamed.emitted {
            let newline = if streamed.buffer.ends_with('\n') {
                String::new()
            } else {
                "\n".into()
            };
            streamed.buffer.clear();
            streamed.emitted = false;
            return newline;
        }
        let source = if text.is_empty() {
            streamed.buffer.as_str()
        } else {
            text
        };
        let rendered = if self.styled {
            render_terminal_markdown(source, self.width)
        } else {
            safe_assistant(source)
        };
        streamed.buffer.clear();
        streamed.emitted = false;
        with_one_newline(rendered)
    }
}
#[async_trait]
impl Approver for Terminal {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        eprint!(
            "\nApproval {} required for {} on {}:\n{}\nProceed? [y/N] ",
            request.id,
            safe_diagnostic(&request.action),
            safe_diagnostic(&request.target),
            safe_diagnostic(&request.reason)
        );
        let _ = io::stderr().flush();
        let mut answer = String::new();
        let outcome = if io::stdin().read_line(&mut answer).is_ok()
            && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
        {
            ApprovalOutcome::Approved
        } else {
            ApprovalOutcome::Denied
        };
        tracing::info!(approval_id = %request.id, execution_id = %request.execution_id,
            action = %request.action, target = %request.target, outcome = ?outcome,
            "approval decided");
        outcome
    }
}

#[async_trait]
impl EventSink for Terminal {
    async fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::Thinking { turn } => eprintln!("[model turn {turn}]"),
            AgentEvent::AssistantTextDelta(text) => {
                print!("{}", self.assistant_delta(&text));
                let _ = io::stdout().flush();
            }
            AgentEvent::AssistantText(text) => {
                print!("{}", self.assistant_completion(&text));
                let _ = io::stdout().flush();
            }
            AgentEvent::ToolStarted { name, arguments } => eprintln!(
                "[tool {}] {}",
                safe_diagnostic(&name),
                safe_diagnostic(&arguments.to_string())
            ),
            AgentEvent::ToolFinished {
                name,
                result,
                success,
            } => eprintln!(
                "[{} {name}] {}",
                if success { "done" } else { "error" },
                summarize(&safe_diagnostic(&result))
            ),
            AgentEvent::ProviderRetry {
                attempt,
                delay,
                error,
            } => eprintln!(
                "[provider retry {attempt} in {:.1}s] {error}",
                delay.as_secs_f32(),
                error = safe_diagnostic(&error)
            ),
            AgentEvent::ContextBudget(report) => eprintln!(
                "[context: estimated {}/{} tokens, {} messages omitted]",
                report.estimated, report.limit, report.omitted_messages
            ),
            AgentEvent::SteeringApplied { .. } => eprintln!("[steering applied]"),
            AgentEvent::Cancelled => eprintln!("[cancelled]"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = match Cli::try_parse() {
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
    // Enrollment never loads provider config or initializes runtime/session logs.
    if let Some(Command::Attachment(args)) = cli.command {
        if matches!(
            &args.command,
            helm::attachment::cli::AttachmentCommand::Connect
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
        if let Some(prepared) = helm::workflow::prepare(args, &workspace)? {
            helm::workflow::print_value(
                &serde_json::json!({"prompt":prepared.prompt,"workflow":prepared.invocation}),
                json,
            )?;
        }
        return Ok(());
    }
    let mut config = Config::load(cli.config.as_deref())?;
    let set_overrides_model = cli.set.iter().any(|assignment| {
        assignment
            .split_once('=')
            .is_some_and(|(key, _)| matches!(key.trim(), "model" | "provider"))
    });
    for assignment in &cli.set {
        let (key, value) = assignment
            .split_once('=')
            .with_context(|| format!("invalid --set `{assignment}`; expected KEY=VALUE"))?;
        config.apply_override(key.trim(), value.trim())?;
    }
    let provider_overridden = cli.provider.is_some();
    if let Some(provider) = cli.provider {
        config.select_provider(provider.into());
    }
    let model_overridden = cli.model.is_some() || provider_overridden || set_overrides_model;
    if let Some(model) = cli.model {
        config.model = model;
    }
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
    match cli.command.unwrap_or(Command::Chat {
        resume: None,
        plain: false,
    }) {
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
            let prepared =
                helm::workflow::prepare(args, &workspace)?.context("workflow run required")?;
            execute_workflow(
                config,
                Some(workspace),
                None,
                prepared.prompt,
                prepared.no_save,
                model_overridden,
                Some(prepared.invocation),
            )
            .await
            .map(|_| ())
        }
        Command::Onboard(args) => helm::onboarding::run(args, &config, cli.workspace),
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
        } => execute(
            config,
            cli.workspace,
            resume,
            prompt.join(" "),
            no_save,
            model_overridden,
        )
        .await
        .map(|_| ()),
        Command::Chat { resume, plain } => {
            let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
            let full_screen = interactive
                && std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb");
            if plain || !full_screen {
                chat(config, cli.workspace, resume, interactive, model_overridden).await
            } else {
                tui_chat(
                    config,
                    cli.workspace,
                    resume,
                    model_overridden,
                    cli.verbose,
                    cli.log_format,
                )
                .await
            }
        }
    }
}

fn command_uses_full_screen_tui(
    cli: &Cli,
    stdin_terminal: bool,
    stdout_terminal: bool,
    term: Option<&str>,
) -> bool {
    stdin_terminal
        && stdout_terminal
        && term.is_some_and(|term| !term.is_empty() && term != "dumb")
        && matches!(
            &cli.command,
            None | Some(Command::Chat { plain: false, .. })
        )
}

fn tui_log_file() -> Result<File> {
    let directory = helm::config::default_data_dir().join("logs");
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "failed to create Helm log directory {}",
            directory.display()
        )
    })?;
    let path = directory.join("helm.log");
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .with_context(|| format!("failed to open Helm TUI log {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure Helm TUI log {}", path.display()))?;
    }
    Ok(file)
}

fn print_config(config: &Config) -> Result<()> {
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
    let mut displayed = config.clone();
    displayed.access = Some(config.access_mode());
    println!("{}", toml::to_string_pretty(&displayed)?);
    Ok(())
}

async fn list_models(config: &Config, workspace: Option<PathBuf>, json: bool) -> Result<()> {
    let workspace = config.resolve_workspace(workspace)?;
    let provider = provider::from_config(config, workspace)?;
    let mut models = tokio::time::timeout(config.timeout(), provider.models())
        .await
        .context("model discovery timed out")??;
    if !models.iter().any(|model| model.id == config.model) {
        models.push(helm::provider::ModelInfo::minimal(config.model.clone()));
        helm::provider::normalize_models(&mut models);
    }
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

async fn doctor(config: &Config, workspace: Option<PathBuf>) -> Result<()> {
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
    let report = serde_json::json!({
        "status": if provider_ready { "ok" } else { "action_required" },
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
        "unattended_approval": config.unattended_approval,
        "inherited_environment": config.inherit_env,
        "mcp_servers": config.mcp_servers.keys().collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn chatgpt_token_store() -> Result<helm::provider::ChatGptTokenStore> {
    Ok(helm::provider::ChatGptTokenStore::new(
        helm::provider::ChatGptTokenStore::default_path()?,
    ))
}

fn chatgpt_provider(config: &Config) -> Result<helm::provider::ChatGptOauthProvider> {
    let mut endpoints = helm::provider::OAuthEndpoints::default();
    if let Some(base) = config.chatgpt_base_url.as_deref() {
        let base = base.trim_end_matches('/');
        endpoints.responses = format!("{base}/responses");
        endpoints.models = format!("{base}/models");
    }
    Ok(helm::provider::ChatGptOauthProvider::from_store(
        chatgpt_token_store()?,
        endpoints,
    ))
}

fn token_status_json(status: &helm::provider::TokenStatus) -> serde_json::Value {
    serde_json::json!({
        "authenticated": status.authenticated,
        "expires_at": status.expires_at,
        "refreshable": status.refreshable,
    })
}

async fn auth(command: AuthCommand, config: &Config) -> Result<()> {
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
                Some(path) => store.import_codex(&path, force).await?,
                None => store.import_default_codex(force).await?,
            };
            println!("Imported ChatGPT credentials");
        }
        AuthCommand::Login { device } => {
            let provider = chatgpt_provider(config)?;
            if device {
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
                        Err(helm::provider::ProviderError::Unavailable(_))
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

fn try_open_browser(url: &str) {
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

struct CliSubagentExecutor {
    config: Config,
    workspace: PathBuf,
    runtime: OnceLock<Weak<SubagentRuntime>>,
    worktrees: Option<WorktreeManager>,
    model: Arc<RwLock<String>>,
}
#[async_trait]
impl SubagentExecutor for CliSubagentExecutor {
    async fn execute(
        &self,
        mut context: ExecutionContext,
    ) -> std::result::Result<SubagentResult, String> {
        context.progress("initializing provider and tools").await;
        let mut config = self.config.clone();
        config.model = self
            .model
            .read()
            .expect("subagent model lock poisoned")
            .clone();
        config.workspace = Some(
            context
                .worktree
                .clone()
                .unwrap_or_else(|| self.workspace.clone()),
        );
        config.allow_read = context.policy.readable_roots.clone();
        config.allow_write = context.policy.writable_roots.clone();
        config.max_tokens =
            (context.budget.max_tokens.min(u32::MAX as u64) as u32).min(config.max_tokens);
        let workspace = config.resolve_workspace(None).map_err(|e| e.to_string())?;
        let policy = Arc::new(Policy::new(&config, workspace.clone()).map_err(|e| e.to_string())?);
        let tool_context = ToolContext {
            policy,
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: config.timeout(),
            max_output_bytes: config.max_output_bytes,
            environment: tool_environment(&config),
            cancellation: context.cancellation.clone(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: redactor(&config),
        };
        let child_budget = context.budget.clone();
        let mut child_policy = context.policy.clone();
        child_policy.budget = child_budget.clone();
        let child_tool = self.runtime.get().and_then(Weak::upgrade).map(|runtime| {
            SubagentTool::new(runtime, child_policy, child_budget)
                .with_parent(context.id)
                .with_worktrees(self.worktrees.clone())
        });
        // Worktree-isolated children still coordinate through the parent's workspace plan.
        // Keying todos by the temporary worktree would silently fork task state.
        let mut tools = build_tools(&config, child_tool, Some(todo_tool(&self.workspace)))
            .await
            .map_err(|e| e.to_string())?;
        tools.retain_allowed(&context.policy.allowed_tools);
        let agent = Agent::new(
            provider::from_config(&config, workspace).map_err(|e| e.to_string())?,
            tools,
            tool_context,
            Arc::new(helm::agent::SilentSink),
            config.model.clone(),
            config.system_prompt.clone(),
            config.max_tokens,
            config.temperature,
        )
        .with_context_window(config.context_window)
        .with_retry_policy(RetryPolicy {
            max_attempts: config.provider_retry_attempts,
            initial_delay: std::time::Duration::from_millis(config.provider_retry_initial_ms),
            max_delay: std::time::Duration::from_millis(config.provider_retry_max_ms),
        });
        let mut inbox = context.take_inbox();
        let (input_tx, input_rx) = helm::agent::steering_channel(64);
        let input_cancel = context.cancellation.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {_=input_cancel.cancelled()=>break,message=inbox.recv()=>match message{Some(helm::subagent::InboxMessage::Message(text))=>{if input_tx.send(format!("Message from supervisor: {text}")).await.is_err(){break}},Some(helm::subagent::InboxMessage::FollowUp(text))=>{if input_tx.send(format!("Follow-up instruction: {text}")).await.is_err(){break}},None=>break}}
            }
        });
        let outcome = agent
            .run_with_cancel_and_input(
                Vec::new(),
                context.task,
                context.cancellation,
                Some(input_rx),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(SubagentResult {
            summary: outcome.answer,
        })
    }
}

struct SubagentBundle {
    runtime: Arc<SubagentRuntime>,
    tool: SubagentTool,
    model: Arc<RwLock<String>>,
}
async fn build_subagents(config: &Config, workspace: &std::path::Path) -> Result<SubagentBundle> {
    let standard = ToolRegistry::standard();
    let mut allowed_tools: std::collections::BTreeSet<String> = standard
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    allowed_tools.insert("subagent".to_string());
    allowed_tools.insert("todo".to_string());
    let budget = AgentBudget {
        max_tokens: config.max_tokens as u64,

        max_terminals: config.terminal_max_count.min(u32::MAX as usize) as u32,
    };
    let policy = AgentPolicy {
        readable_roots: std::iter::once(workspace.to_path_buf())
            .chain(config.allow_read.clone())
            .collect(),
        writable_roots: std::iter::once(workspace.to_path_buf())
            .chain(config.allow_write.clone())
            .collect(),
        allowed_tools,
        approval: ApprovalPolicy::Deny,
        budget: budget.clone(),
    };
    let workspace_key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    let worktrees = worktree_manager(workspace, &workspace_key);
    let model = Arc::new(RwLock::new(config.model.clone()));
    let executor = Arc::new(CliSubagentExecutor {
        config: config.clone(),
        workspace: workspace.to_path_buf(),
        runtime: OnceLock::new(),
        worktrees: worktrees.clone(),
        model: model.clone(),
    });
    let store = helm::subagent::AgentTreeStore::new(
        helm::config::default_data_dir()
            .join("subagents")
            .join(format!("{workspace_key}.json")),
    );
    let runtime = Arc::new(
        SubagentRuntime::new_persistent(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: config.subagent_max_concurrency,
                event_history: config.subagent_event_history,
            },
            store,
        )
        .await
        .map_err(anyhow::Error::msg)?,
    );
    executor
        .runtime
        .set(Arc::downgrade(&runtime))
        .map_err(|_| anyhow::anyhow!("subagent runtime already initialized"))?;
    let tool = SubagentTool::new(runtime.clone(), policy, budget).with_worktrees(worktrees);
    Ok(SubagentBundle {
        runtime,
        tool,
        model,
    })
}

fn worktree_manager(workspace: &std::path::Path, workspace_key: &str) -> Option<WorktreeManager> {
    let repository = if workspace.join(".git").exists() {
        workspace.to_path_buf()
    } else if workspace.join(".local-git/worktree.git/HEAD").is_file() {
        workspace.join(".local-git/worktree.git")
    } else {
        return None;
    };
    WorktreeManager::new(
        repository,
        helm::config::default_data_dir()
            .join("worktrees")
            .join(workspace_key),
    )
    .ok()
}

async fn build_agent(config: &Config, workspace: PathBuf, attended: bool) -> Result<Agent> {
    let policy = Arc::new(Policy::new(config, workspace.clone())?);
    let terminal = Arc::new(Terminal::default());
    let approver: Arc<dyn Approver> = if attended {
        terminal.clone()
    } else {
        Arc::new(UnattendedApprover {
            allow: config.unattended_approval == UnattendedApprovalMode::Allow,
        })
    };
    let context = ToolContext {
        policy,
        approver,
        timeout: config.timeout(),
        max_output_bytes: config.max_output_bytes,
        environment: tool_environment(config),
        cancellation: tokio_util::sync::CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: if attended {
            InteractionMode::Attended
        } else {
            InteractionMode::Unattended
        },
        redactor: redactor(config),
    };
    let subagents = build_subagents(config, &workspace).await?;
    let _runtime = subagents.runtime.clone();
    let tools = build_tools(config, Some(subagents.tool), Some(todo_tool(&workspace))).await?;
    Ok(Agent::new(
        provider::from_config(config, context.policy.workspace().to_owned())?,
        tools,
        context,
        terminal,
        config.model.clone(),
        config.system_prompt.clone(),
        config.max_tokens,
        config.temperature,
    )
    .with_context_window(config.context_window)
    .with_model_mirror(subagents.model)
    .with_retry_policy(RetryPolicy {
        max_attempts: config.provider_retry_attempts,
        initial_delay: std::time::Duration::from_millis(config.provider_retry_initial_ms),
        max_delay: std::time::Duration::from_millis(config.provider_retry_max_ms),
    }))
}

fn tool_environment(config: &Config) -> std::collections::BTreeMap<String, String> {
    let mut environment = config
        .inherit_env
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
        .collect::<std::collections::BTreeMap<_, _>>();
    environment.extend(config.env.clone());
    environment
}

fn redactor(config: &Config) -> Arc<Redactor> {
    let secrets = config
        .redact_values
        .iter()
        .cloned()
        .chain(config.env.values().cloned())
        .chain(
            config
                .mcp_servers
                .values()
                .flat_map(|server| server.env.values().cloned()),
        )
        .chain(config.api_key_for_redaction());
    Arc::new(Redactor::new(secrets))
}

async fn tui_chat(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    model_overridden: bool,
    verbose: bool,
    log_format: LogFormat,
) -> Result<()> {
    let store = SessionStore::default();
    let mut session = if let Some(reference) = resume {
        store.load_reference(&reference).await?
    } else {
        Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        )
    };
    if model_overridden && session.switch_model(config.model.clone())? {
        store.save(&mut session).await?;
    }
    let (bridge, receiver) = helm::tui::bridge();
    let mut active_config = config.clone();
    active_config.model = session.model.clone();
    let profile = active_config.provider_profile();
    let provider_label = format!(
        "{} ({})",
        profile.id,
        if profile.compatibility_bridge {
            "external bridge"
        } else {
            "native"
        }
    );
    let policy = Arc::new(Policy::new(&active_config, session.workspace.clone())?);
    let context = ToolContext {
        policy,
        approver: bridge.clone(),
        timeout: active_config.timeout(),
        max_output_bytes: active_config.max_output_bytes,
        environment: tool_environment(&active_config),
        cancellation: tokio_util::sync::CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: redactor(&active_config),
    };
    let subagents = build_subagents(&active_config, &session.workspace).await?;
    let subagent_runtime = subagents.runtime.clone();
    let todo = todo_tool(&session.workspace);
    let tools = build_tools(&active_config, Some(subagents.tool), Some(todo.clone())).await?;
    let terminals: Arc<dyn helm::terminal::InteractiveTerminals> =
        Arc::new(tools.terminals().unwrap_or_default());
    let agent = Arc::new(
        Agent::new(
            provider::from_config(&active_config, session.workspace.clone())?,
            tools,
            context,
            bridge.clone(),
            session.model.clone(),
            active_config.system_prompt.clone(),
            active_config.max_tokens,
            active_config.temperature,
        )
        .with_context_window(active_config.context_window)
        .with_model_mirror(subagents.model)
        .with_retry_policy(RetryPolicy {
            max_attempts: active_config.provider_retry_attempts,
            initial_delay: std::time::Duration::from_millis(
                active_config.provider_retry_initial_ms,
            ),
            max_delay: std::time::Duration::from_millis(active_config.provider_retry_max_ms),
        }),
    );
    let session_id = session.id;
    let exit = helm::tui::run(
        agent,
        store,
        session,
        receiver,
        bridge.sender(),
        terminals,
        Arc::new(helm::supervision::RuntimeAgentSupervisor::new(
            subagent_runtime,
        )),
        todo.store(),
        provider_label,
        active_config.access_mode(),
    )
    .await?;
    match exit {
        helm::tui::TuiExit::Quit => Ok(()),
        helm::tui::TuiExit::Launch(request) => {
            launch_from_tui(&active_config, session_id, request, verbose, log_format).await
        }
    }
}

fn write_runtime_config(config: &Config) -> Result<tempfile::NamedTempFile> {
    let directory = helm::config::default_data_dir();
    std::fs::create_dir_all(&directory)?;
    let contents = toml::to_string_pretty(config)?;
    let mut file = tempfile::Builder::new()
        .prefix("runtime-config-")
        .suffix(".toml")
        .tempfile_in(directory)?;
    file.write_all(contents.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

async fn run_helm_child(config_path: Option<&std::path::Path>, arguments: &[String]) -> Result<()> {
    let executable = std::env::current_exe().context("cannot locate the Helm executable")?;
    let mut command = tokio::process::Command::new(executable);
    if let Some(path) = config_path {
        command.arg("--config").arg(path);
    }
    let status = command
        .args(arguments)
        .status()
        .await
        .context("could not launch Helm command")?;
    if !status.success() {
        bail!("Helm command exited with {status}");
    }
    Ok(())
}

async fn launch_from_tui(
    config: &Config,
    session_id: uuid::Uuid,
    request: helm::tui::CliRequest,
    verbose: bool,
    log_format: LogFormat,
) -> Result<()> {
    let runtime_config = request
        .use_active_config
        .then(|| write_runtime_config(config))
        .transpose()?;
    let active_verbose = request.verbose.unwrap_or(verbose);
    let active_log_format = request.log_format.as_deref().unwrap_or(match log_format {
        LogFormat::Text => "text",
        LogFormat::Json => "json",
    });
    let mut arguments = Vec::new();
    if active_verbose {
        arguments.push("--verbose".into());
    }
    arguments.extend(["--log-format".into(), active_log_format.into()]);
    arguments.extend(request.arguments);
    let result = run_helm_child(
        runtime_config.as_ref().map(tempfile::NamedTempFile::path),
        &arguments,
    )
    .await;
    if request.resume_after || result.is_err() {
        if let Err(error) = &result {
            eprintln!("\nRequested Helm command failed: {error:#}");
        }
        eprint!("\nPress Enter to return to session {session_id}…");
        io::stderr().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let mut resume_arguments = Vec::new();
        if active_verbose {
            resume_arguments.push("--verbose".into());
        }
        resume_arguments.extend([
            "--log-format".into(),
            active_log_format.into(),
            "chat".into(),
            "--resume".into(),
            session_id.to_string(),
        ]);
        run_helm_child(
            runtime_config.as_ref().map(tempfile::NamedTempFile::path),
            &resume_arguments,
        )
        .await?;
        return Ok(());
    }
    result
}

fn todo_tool(workspace: &std::path::Path) -> TodoTool {
    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    TodoTool::new(Arc::new(TodoStore::new(
        helm::config::default_data_dir()
            .join("todos")
            .join(format!("{key}.json")),
        TodoScope::workspace(workspace),
    )))
}

async fn build_tools(
    config: &Config,
    subagents: Option<SubagentTool>,
    todos: Option<TodoTool>,
) -> Result<ToolRegistry> {
    let mut tools = ToolRegistry::standard_with_terminal_limits(
        config.terminal_max_count,
        config.terminal_max_unread_bytes,
    );
    if let Some(tool) = subagents {
        tools.register_subagents(tool)?;
    }
    if let Some(tool) = todos {
        tools.register_todos(tool)?;
    }
    if config.access_mode() == AccessMode::ReadOnly {
        // Do not even start external MCP servers in read-only mode: their
        // initialization and tool contracts are outside Helm's authority model.
        tools.retain_read_only();
        return Ok(tools);
    }
    for (name, server) in &config.mcp_servers {
        let mut environment = tool_environment(config);
        environment.extend(server.env.clone());
        let mcp = tokio::time::timeout(
            config.timeout(),
            helm::tools::mcp::McpServer::connect(name, &server.command, &server.args, &environment),
        )
        .await
        .with_context(|| format!("MCP server `{name}` initialization timed out"))?
        .with_context(|| format!("failed to initialize MCP server `{name}`"))?;
        let discovered = tokio::time::timeout(config.timeout(), mcp.discover())
            .await
            .with_context(|| format!("MCP server `{name}` discovery timed out"))?
            .with_context(|| format!("failed to discover tools from MCP server `{name}`"))?;
        for tool in discovered {
            tools
                .register_arc(tool)
                .with_context(|| format!("MCP server `{name}` exposed a duplicate tool"))?;
        }
    }
    Ok(tools)
}

async fn execute(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    prompt: String,
    no_save: bool,
    model_overridden: bool,
) -> Result<Session> {
    execute_workflow(
        config,
        workspace_arg,
        resume,
        prompt,
        no_save,
        model_overridden,
        None,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
async fn execute_workflow(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    prompt: String,
    no_save: bool,
    model_overridden: bool,
    workflow: Option<helm::workflow::Invocation>,
) -> Result<Session> {
    let store = SessionStore::default();
    let mut session = if let Some(reference) = resume {
        store.load_reference(&reference).await?
    } else {
        Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        )
    };
    if model_overridden {
        session.switch_model(config.model.clone())?;
    }
    let mut active_config = config.clone();
    active_config.model = session.model.clone();
    let workflow_run = workflow.is_some();
    if let Some(invocation) = workflow {
        anyhow::ensure!(
            session.workflow_runs.len() < 128,
            "session workflow history is full"
        );
        session.workflow_runs.push(invocation);
        if !no_save {
            store.save(&mut session).await?;
        }
    }
    let attended = !workflow_run || (io::stdin().is_terminal() && io::stdout().is_terminal());
    let agent = build_agent(&active_config, session.workspace.clone(), attended).await?;
    let outcome = match agent.run(session.messages.clone(), prompt).await {
        Ok(outcome) => outcome,
        Err(error) => {
            if !no_save && let Some(recovery) = error.recovery() {
                session.recover_context_failure(recovery)?;
                session.terminals = agent.terminal_metadata();
                store.save(&mut session).await?;
                eprintln!("[session {}]", session.id);
            }
            return Err(error.into());
        }
    };
    let title_due = session.title_due_after_turn();
    session.record_completed_turn();
    session.messages = outcome.messages;
    session.usage.input_tokens += outcome.usage.input_tokens;
    session.usage.output_tokens += outcome.usage.output_tokens;
    session.terminals = agent.terminal_metadata();
    if !no_save {
        store.save(&mut session).await?;
        if title_due
            && let Some(result) = agent
                .generate_title(
                    &session.messages,
                    tokio_util::sync::CancellationToken::new(),
                )
                .await
        {
            session.apply_generated_title(result);
            store.save(&mut session).await?;
        }
        eprintln!("[session {}]", session.id);
    }
    Ok(session)
}

async fn chat(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    interactive: bool,
    model_overridden: bool,
) -> Result<()> {
    let store = SessionStore::default();
    let mut session = if let Some(reference) = resume {
        store.load_reference(&reference).await?
    } else {
        Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        )
    };
    if model_overridden && session.switch_model(config.model.clone())? {
        store.save(&mut session).await?;
    }
    let mut agent: Option<Agent> = None;
    if interactive {
        eprintln!(
            "Helm · {} · {} · access: {} · {}\nType /help for commands.",
            session.display_name(),
            session.model,
            config.access_mode(),
            session.workspace.display()
        );
    }
    loop {
        if interactive {
            print!("\nhelm> ");
            io::stdout().flush()?;
        }
        let mut prompt = String::new();
        if io::stdin().read_line(&mut prompt)? == 0 {
            break;
        }
        let prompt = prompt.trim();
        if prompt.is_empty() {
            continue;
        }
        match prompt {
            "/quit" | "/exit" => break,
            "/help" => {
                println!(
                    "/help  /session  /new [TITLE]  /name TITLE  /access  /tools  /model [MODEL]  /models  /clear  /exit"
                );
                continue;
            }
            "/access" => {
                println!("{}", config.access_mode());
                continue;
            }
            "/name" => {
                eprintln!("usage: /name TITLE");
                continue;
            }
            "/session" => {
                println!(
                    "{} · {} ({} input, {} output tokens)",
                    session.display_name(),
                    session.id,
                    session.usage.input_tokens,
                    session.usage.output_tokens
                );
                continue;
            }
            "/model" => {
                println!("{}", session.model);
                continue;
            }
            "/clear" => {
                session.clear_conversation();
                store.save(&mut session).await?;
                println!("conversation cleared");
                continue;
            }
            _ => {}
        }
        if prompt == "/new" || prompt.starts_with("/new ") {
            let name = prompt.strip_prefix("/new").unwrap_or_default().trim();
            let mut next = Session::new(session.workspace.clone(), session.model.clone());
            if !name.is_empty() {
                next.set_name(name.to_owned());
            }
            session = next;
            println!("new session: {}", session.display_name());
            continue;
        }
        if let Some(name) = prompt.strip_prefix("/name ") {
            let name = name.trim();
            if name.is_empty() {
                eprintln!("usage: /name TITLE");
            } else {
                session.set_name(name.to_owned());
                store.save(&mut session).await?;
                println!("session renamed to {name}");
            }
            continue;
        }
        if prompt == "/tools" {
            if agent.is_none() {
                let mut active_config = config.clone();
                active_config.model = session.model.clone();
                agent = Some(
                    build_agent(&active_config, session.workspace.clone(), interactive).await?,
                );
            }
            for tool in agent.as_ref().expect("agent initialized").tool_inventory() {
                println!("{}\t{}", tool.name, tool.description);
            }
            continue;
        }
        if let Some(model) = prompt.strip_prefix("/model ") {
            let model = model.trim();
            if model.is_empty() {
                eprintln!("usage: /model MODEL");
                continue;
            }
            session.switch_model(model)?;
            if let Some(agent) = &agent {
                agent.set_model(model)?;
            }
            store.save(&mut session).await?;
            println!("model switched to {model}");
            continue;
        }
        if prompt == "/models" {
            let mut active_config = config.clone();
            active_config.model = session.model.clone();
            match provider::from_config(&active_config, session.workspace.clone()) {
                Ok(provider) => {
                    match tokio::time::timeout(active_config.timeout(), provider.models()).await {
                        Ok(Ok(mut models)) => {
                            helm::provider::normalize_models(&mut models);
                            for model in models {
                                println!(
                                    "{}{}\t{}",
                                    if model.id == session.model {
                                        "* "
                                    } else {
                                        "  "
                                    },
                                    model.id,
                                    model.display_name
                                );
                            }
                        }
                        Ok(Err(error)) => eprintln!("model discovery failed: {error}"),
                        Err(_) => eprintln!("model discovery timed out"),
                    }
                }
                Err(error) => eprintln!("model discovery failed: {error}"),
            }
            continue;
        }
        if agent.is_none() {
            let mut active_config = config.clone();
            active_config.model = session.model.clone();
            agent =
                Some(build_agent(&active_config, session.workspace.clone(), interactive).await?);
        }
        match agent
            .as_ref()
            .expect("agent initialized")
            .run(session.messages.clone(), prompt.to_owned())
            .await
        {
            Ok(outcome) => {
                let title_due = session.title_due_after_turn();
                session.record_completed_turn();
                session.messages = outcome.messages;
                session.usage.input_tokens += outcome.usage.input_tokens;
                session.usage.output_tokens += outcome.usage.output_tokens;
                session.terminals = agent
                    .as_ref()
                    .expect("agent initialized")
                    .terminal_metadata();
                store.save(&mut session).await?;
                if title_due
                    && let Some(result) = agent
                        .as_ref()
                        .expect("agent initialized")
                        .generate_title(
                            &session.messages,
                            tokio_util::sync::CancellationToken::new(),
                        )
                        .await
                {
                    session.apply_generated_title(result);
                    store.save(&mut session).await?;
                }
            }
            Err(error) => {
                if let Some(recovery) = error.recovery() {
                    session.recover_context_failure(recovery)?;
                    session.terminals = agent
                        .as_ref()
                        .expect("agent initialized")
                        .terminal_metadata();
                    store.save(&mut session).await?;
                    eprintln!("[session {}]", session.id);
                }
                eprintln!("error: {error:#}");
            }
        }
    }
    if interactive && !session.messages.is_empty() {
        eprintln!("Session {} saved as {}", session.display_name(), session.id);
    }
    Ok(())
}

async fn list_sessions() -> Result<()> {
    for session in SessionStore::default().list().await? {
        println!(
            "{}  {}  {:<24}  {}  {} messages",
            session.id,
            session.updated_at.format("%Y-%m-%d %H:%M UTC"),
            session.display_name(),
            session.workspace.display(),
            session.messages.len()
        );
    }
    Ok(())
}
fn summarize(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.chars().count() > 160 {
        format!("{}…", line.chars().take(160).collect::<String>())
    } else {
        line.into()
    }
}
fn with_one_newline(mut text: String) -> String {
    if text.is_empty() {
        return text;
    }
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    text
}
fn safe_diagnostic(text: &str) -> String {
    text.chars().filter(|ch|!ch.is_control()&&!matches!(*ch,'\u{7f}'|'\u{80}'..='\u{9f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}')).collect()
}
fn safe_assistant(text: &str) -> String {
    text.chars().filter(|ch| matches!(*ch,'\n'|'\t')||(!ch.is_control()&&!matches!(*ch,'\u{7f}'|'\u{80}'..='\u{9f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))).collect()
}
fn render_terminal_markdown(source: &str, width: usize) -> String {
    use crossterm::style::{
        Attribute, Color as CtColor, ResetColor, SetAttribute, SetBackgroundColor,
        SetForegroundColor,
    };
    use ratatui::style::{Color, Modifier};
    fn color(value: Color) -> CtColor {
        match value {
            Color::Reset => CtColor::Reset,
            Color::Black => CtColor::Black,
            Color::Red => CtColor::DarkRed,
            Color::Green => CtColor::DarkGreen,
            Color::Yellow => CtColor::DarkYellow,
            Color::Blue => CtColor::DarkBlue,
            Color::Magenta => CtColor::DarkMagenta,
            Color::Cyan => CtColor::DarkCyan,
            Color::Gray => CtColor::Grey,
            Color::DarkGray => CtColor::DarkGrey,
            Color::LightRed => CtColor::Red,
            Color::LightGreen => CtColor::Green,
            Color::LightYellow => CtColor::Yellow,
            Color::LightBlue => CtColor::Blue,
            Color::LightMagenta => CtColor::Magenta,
            Color::LightCyan => CtColor::Cyan,
            Color::White => CtColor::White,
            Color::Rgb(r, g, b) => CtColor::Rgb { r, g, b },
            Color::Indexed(i) => CtColor::AnsiValue(i),
        }
    }
    let text = helm::markdown::render_markdown(
        source,
        helm::markdown::RenderOptions {
            width,
            ..Default::default()
        },
    );
    let mut output = String::new();
    for (line_index, line) in text.lines.iter().enumerate() {
        for span in &line.spans {
            output.push_str(&SetAttribute(Attribute::Reset).to_string());
            output.push_str(
                &SetForegroundColor(color(span.style.fg.unwrap_or(Color::Reset))).to_string(),
            );
            if let Some(bg) = span.style.bg {
                output.push_str(&SetBackgroundColor(color(bg)).to_string())
            }
            for (modifier, attribute) in [
                (Modifier::BOLD, Attribute::Bold),
                (Modifier::ITALIC, Attribute::Italic),
                (Modifier::UNDERLINED, Attribute::Underlined),
                (Modifier::CROSSED_OUT, Attribute::CrossedOut),
                (Modifier::DIM, Attribute::Dim),
            ] {
                if span.style.add_modifier.contains(modifier) {
                    output.push_str(&SetAttribute(attribute).to_string())
                }
            }
            output.push_str(&span.content);
        }
        output.push_str(&ResetColor.to_string());
        output.push_str(&SetAttribute(Attribute::Reset).to_string());
        if line_index + 1 < text.lines.len() {
            output.push('\n')
        }
    }
    output
}

// A stalled terminal (or full stderr pipe) must not obstruct cancellation after
// native mode restoration. The process exits after this bounded best effort.
async fn attachment_notice(message: &'static str) {
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        tokio::task::spawn_blocking(move || eprintln!("{message}")),
    )
    .await;
}

async fn attachment_connect(args: helm::attachment::cli::AttachmentArgs) -> Result<()> {
    let cancel = tokio_util::sync::CancellationToken::new();
    let operation = helm::attachment::cli::run_cancellable(args, cancel.clone());
    tokio::pin!(operation);
    tokio::select! { biased;
        _ = attachment_interrupt() => {
            cancel.cancel();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(3), &mut operation).await;
            attachment_notice("attachment connection interrupted; enrollment unchanged").await;
            std::process::exit(130)
        }
        result = &mut operation => result.map_err(anyhow::Error::from),
    }
}

async fn attachment_interrupt() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let (Ok(mut term), Ok(mut hangup)) = (
            signal(SignalKind::terminate()),
            signal(SignalKind::hangup()),
        ) {
            tokio::select! { _=tokio::signal::ctrl_c()=>(), _=term.recv()=>(), _=hangup.recv()=>() }
            return;
        }
    }
    #[cfg(windows)]
    {
        if let Ok(mut interrupt) = tokio::signal::windows::ctrl_break() {
            tokio::select! { _=tokio::signal::ctrl_c()=>(), _=interrupt.recv()=>() }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod cli_tests {
    #[test]
    fn legacy_connectivity_is_not_a_cli_mode() {
        for args in [
            vec!["helm", "--voyage"],
            vec!["helm", "--voyage=http://127.0.0.1:9480"],
            vec!["helm", "chat", "--voyage"],
            vec!["helm", "--name", "workstation"],
            vec!["helm", "attach", "https://vessel.example", "not-a-real-key"],
        ] {
            assert!(Cli::try_parse_from(&args).is_err(), "accepted {args:?}");
        }
        let help = Cli::command().render_long_help().to_string();
        assert!(!help.contains("--voyage"));
        assert!(!help.contains("--name"));
        assert!(Cli::try_parse_from(["helm", "chat", "--plain"]).is_ok());
    }

    use super::*;

    #[test]
    fn only_interactive_full_screen_chat_routes_logs_away_from_stderr() {
        let chat = Cli::try_parse_from(["helm", "chat"]).unwrap();
        assert!(command_uses_full_screen_tui(
            &chat,
            true,
            true,
            Some("xterm-256color")
        ));
        assert!(!command_uses_full_screen_tui(
            &chat,
            true,
            true,
            Some("dumb")
        ));
        let plain = Cli::try_parse_from(["helm", "chat", "--plain"]).unwrap();
        assert!(!command_uses_full_screen_tui(
            &plain,
            true,
            true,
            Some("xterm")
        ));
        let doctor = Cli::try_parse_from(["helm", "doctor"]).unwrap();
        assert!(!command_uses_full_screen_tui(
            &doctor,
            true,
            true,
            Some("xterm")
        ));
    }

    #[test]
    fn accepts_canonical_and_legacy_provider_names() {
        let cli =
            Cli::try_parse_from(["helm", "--provider", "openai-responses", "config"]).unwrap();
        assert!(matches!(cli.provider, Some(ProviderArg::OpenaiResponses)));
        for name in ["openai", "openai-chat", "openai-compatible"] {
            let cli = Cli::try_parse_from(["helm", "--provider", name, "config"]).unwrap();
            assert!(matches!(cli.provider, Some(ProviderArg::OpenaiChat)));
        }
        let cli = Cli::try_parse_from(["helm", "--provider", "chatgpt-oauth", "config"]).unwrap();
        assert!(matches!(cli.provider, Some(ProviderArg::ChatGptOauth)));
        for name in ["codex-compatibility", "codex-subscription"] {
            let cli = Cli::try_parse_from(["helm", "--provider", name, "doctor"]).unwrap();
            assert!(matches!(
                cli.provider,
                Some(ProviderArg::CodexCompatibility)
            ));
        }
    }

    #[test]
    fn accepts_runtime_configuration_overrides() {
        let cli = Cli::try_parse_from([
            "helm",
            "--set",
            "max_tokens=32",
            "--set",
            "access=unrestricted",
            "config",
        ])
        .unwrap();
        assert_eq!(cli.set, ["max_tokens=32", "access=unrestricted"]);
    }

    #[test]
    fn parses_access_modes_and_rejects_conflicting_legacy_flag() {
        for (name, expected) in [
            ("read-only", AccessArg::ReadOnly),
            ("approval", AccessArg::Approval),
            ("unrestricted", AccessArg::Unrestricted),
        ] {
            let cli = Cli::try_parse_from(["helm", "--access", name, "config"]).unwrap();
            assert_eq!(cli.access.map(AccessMode::from), Some(expected.into()));
        }
        assert!(
            Cli::try_parse_from([
                "helm",
                "--access",
                "approval",
                "--approval",
                "never",
                "config",
            ])
            .is_err()
        );
    }

    #[test]
    fn parses_native_auth_lifecycle_commands() {
        assert!(matches!(
            Cli::try_parse_from(["helm", "auth", "status"])
                .unwrap()
                .command,
            Some(Command::Auth {
                command: AuthCommand::Status
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["helm", "auth", "login", "--device"])
                .unwrap()
                .command,
            Some(Command::Auth {
                command: AuthCommand::Login { device: true }
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["helm", "auth", "import-codex", "--force"])
                .unwrap()
                .command,
            Some(Command::Auth {
                command: AuthCommand::ImportCodex { force: true, .. }
            })
        ));
    }

    #[test]
    fn streamed_completion_is_emitted_once_with_one_newline() {
        let terminal = Terminal::for_output(false, false);
        assert_eq!(terminal.assistant_delta("hello\n"), "hello\n");
        assert_eq!(terminal.assistant_delta("world"), "world");
        assert_eq!(terminal.assistant_completion("hello\nworld\n\n"), "\n");
        assert_eq!(terminal.assistant_completion(""), "");
    }

    #[test]
    fn redirected_and_no_color_output_never_contains_ansi() {
        for terminal in [
            Terminal::for_output(false, false),
            Terminal::for_output(true, true),
        ] {
            assert_eq!(terminal.assistant_delta("**bo"), "**bo");
            assert_eq!(
                terminal.assistant_delta("ld** \u{1b}[31mred"),
                "ld** [31mred"
            );
            let rendered = terminal.assistant_completion("**bold** red");
            assert!(!rendered.contains('\u{1b}'));
            assert_eq!(rendered, "\n");
        }
    }

    #[test]
    fn tool_diagnostics_are_sanitized_without_markdown_interpretation() {
        assert_eq!(safe_diagnostic("**failure**\u{1b}[31m"), "**failure**[31m");
        assert_eq!(summarize(&"é".repeat(200)).chars().count(), 161);
    }
}
