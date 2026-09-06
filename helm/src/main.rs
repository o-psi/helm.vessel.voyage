use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use helm::{
    Agent, AgentEvent, Config, EventSink,
    agent::RetryPolicy,
    config::{AccessMode, ApprovalMode},
    policy::Policy,
    provider,
    session::{Session, SessionStore},
    subagent::SubagentRuntime,
    tools::{
        ApprovalOutcome, ApprovalRequest, Approver, InteractionMode, Redactor, ToolContext,
        UnattendedApprover,
    },
};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
};
use tracing_subscriber::EnvFilter;
use voyage_runtime::build::{
    ManagedAgent, ManagedResources, build_subagents_managed, build_tools, redactor, todo_tool,
    tool_environment,
};
mod managed;
mod remote_consent;
mod remote_worker;
mod tui_runtime;
#[derive(Parser)]
#[command(version, about = "A general-purpose LLM harness for terminal work")]
struct Cli {
    #[command(flatten)]
    policy: helm::policy_profile::cli::SelectionArgs,
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
    #[arg(short, long, global = true, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
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
    /// Connect to independent voyage processes through local or SSH-accessed Vessels.
    Connect(helm::process_client::cli::ConnectArgs),
    /// Inspect GitHub context and publish only after exact attended review.
    Github(GithubArgs),
    /// Install, inspect and explicitly enable declarative packages.
    Extension(helm::extensions::cli::ExtensionArgs),
    /// Inspect and configure local session/project inference attempt allowances.
    Inference(helm::inference::cli::InferenceArgs),
    /// Manage named policy profiles and preview explicit launch selection.
    Policy(helm::policy_profile::cli::PolicyArgs),
    /// Discover, inspect and run saved nonsecret workflows.
    Workflow(helm::workflow::WorkflowArgs),
    /// Inspect a repository and explicitly review generated project guidance.
    Onboard(helm::onboarding::OnboardArgs),
    /// Use private local sessions with an authoritative SQLite journal.
    Managed(managed::Args),
    /// Run one explicitly exported dedicated managed session in the foreground.
    RemoteWorker(remote_worker::Args),
    /// Inspect and permanently withdraw a dedicated remote grant locally.
    RemoteConsent(remote_consent::Args),
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
        #[arg(long, action = clap::ArgAction::Set, num_args = 0..=1, default_missing_value = "true", default_value = "false", require_equals = true)]
        plain: bool,
    },
    Sessions,
    /// List models available to the configured provider/account.
    Models {
        /// Emit stable machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect resolved configuration with secret bindings concealed.
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

#[derive(clap::Args)]
struct GithubArgs {
    /// Existing local voyage whose references and operation scope to use.
    #[arg(long)]
    session: Option<String>,
    #[command(flatten)]
    args: helm::github::operator::Args,
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
        if helm::plain_terminal::owns_terminal() {
            return ApprovalOutcome::Unavailable;
        }
        if request.action.starts_with("github.") {
            return helm::github::approval::approve_terminal(request).await;
        }
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
        if helm::plain_terminal::owns_terminal() {
            return;
        }
        match event {
            AgentEvent::CompletionState {
                phase,
                readiness,
                detail,
            } => {
                let phase = format!("{phase:?}").to_lowercase();
                let detail = detail.as_deref().map(safe_diagnostic).unwrap_or_default();
                let counts = readiness
                    .map(|r| {
                        format!(
                            " · {} unresolved · {} accounted unfinished",
                            r.total.saturating_sub(r.accounted),
                            r.incomplete
                        )
                    })
                    .unwrap_or_default();
                println!("\n[completion: {phase}{counts}] {detail}");
                let _ = io::stdout().flush();
            }
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
            AgentEvent::InferenceWarning(status) => {
                eprintln!("[inference warning: {}]", status.summary())
            }
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
            if matches!(&args.command, Some(helm::process_client::cli::ConnectedCommand::New { .. })));
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
        Command::Github(args) => github_cli(&config, cli.workspace, args).await,
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
            let prepared = prepare_workflow(args, &workspace)
                .await?
                .context("workflow run required")?;
            let cancellation = prepared
                .input_monitor
                .as_ref()
                .map(|monitor| monitor.cancellation());
            let _input_monitor = prepared.input_monitor;
            execute_workflow(
                config,
                Some(workspace),
                None,
                prepared.prompt,
                prepared.no_save,
                model_overridden,
                Some((prepared.invocation, prepared.secrets)),
                cancellation,
            )
            .await
            .map(|_| ())
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
    println!("# Secret values are concealed; this display cannot restore secret bindings.");
    println!("{}", config.diagnostic_toml()?);
    Ok(())
}

async fn list_models(config: &Config, workspace: Option<PathBuf>, json: bool) -> Result<()> {
    let workspace = config.resolve_workspace(workspace)?;
    let resolved = helm::runtime_policy::RuntimePolicy::resolve(config, &workspace)?;
    let config = resolved.config();
    let provider = provider::from_config(config, workspace)?;
    let mut models = tokio::time::timeout(config.timeout(), provider.models())
        .await
        .context("model discovery timed out")??;
    if !models.iter().any(|model| model.id == config.model) {
        models.push(helm::provider::ModelInfo::minimal(config.model.clone()));
    }
    let secrets = redactor(config);
    helm::provider::validate_models_for_display(&models, |value| secrets.contains_secret(value))?;
    helm::provider::normalize_models(&mut models);
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

/// Resolve a finite retained-record snapshot without transferring lease ownership.
async fn build_agent(config: &Config, workspace: PathBuf, attended: bool) -> Result<Agent> {
    Ok(build_agent_bundle(config, workspace, attended, None)
        .await?
        .agent)
}
async fn build_agent_bundle(
    config: &Config,
    workspace: PathBuf,
    attended: bool,
    sink: Option<Arc<dyn EventSink>>,
) -> Result<ManagedAgent> {
    build_authorized_agent_bundle(config, workspace, attended, sink, None).await
}
async fn build_authorized_agent_bundle(
    config: &Config,
    workspace: PathBuf,
    attended: bool,
    sink: Option<Arc<dyn EventSink>>,
    authority: Option<Arc<dyn helm::policy::ExecutionAuthority>>,
) -> Result<ManagedAgent> {
    let terminal = Arc::new(Terminal::default());
    let approver: Option<Arc<dyn Approver>> =
        attended.then(|| terminal.clone() as Arc<dyn Approver>);
    voyage_runtime::build::build_agent_bundle_with_output(
        config, workspace, attended, sink, authority, approver, terminal,
    )
    .await
}

fn github_context(config: &Config, workspace: &std::path::Path) -> Result<ToolContext> {
    let resolved = helm::runtime_policy::RuntimePolicy::resolve(config, workspace)?;
    let config = resolved.config();
    let attended = io::stdin().is_terminal() && io::stderr().is_terminal();
    Ok(ToolContext {
        github: helm::github::Credential::from_config(config),
        completion: None,
        policy: Arc::new(resolved.policy().clone()),
        approver: if attended {
            Arc::new(Terminal::default())
        } else {
            Arc::new(UnattendedApprover { allow: false })
        },
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
    })
}

async fn github_import(
    context: &ToolContext,
    result: &mut helm::github::operator::CommandResult,
) -> Result<()> {
    let Some(feedback) = result.feedback.take() else {
        return Ok(());
    };
    context.policy.check_current()?;
    let workspace = context.policy.workspace();
    let root = helm::config::default_data_dir().join("completion");
    let mut directories = std::fs::DirBuilder::new();
    directories.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directories.mode(0o700);
    }
    directories.create(&root)?;
    let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    let coordinator = helm::completion::runtime::Coordinator::open(root.join(key), workspace)?;
    let todo = todo_tool(workspace, coordinator)
        .store()
        .import_github_feedback(
            feedback,
            context.policy.clone(),
            context.cancellation.clone(),
        )
        .await?;
    result.display.push_str(&format!(
        "\nLocal task {} · {:?}; existing edits/status are retained on repeated import.",
        todo.id.0, todo.status
    ));
    Ok(())
}

async fn github_cli(
    config: &Config,
    workspace_arg: Option<PathBuf>,
    args: GithubArgs,
) -> Result<()> {
    let (owner, mut session) = if let Some(reference) = args.session {
        let (owner, session) = SessionStore::default().load_owned(&reference).await?;
        if let Some(workspace) = &workspace_arg {
            anyhow::ensure!(
                workspace.canonicalize()? == session.workspace.canonicalize()?,
                "selected workspace differs from the voyage"
            );
        }
        (Some(owner), Some(session))
    } else {
        (None, None)
    };
    let workspace = if let Some(session) = &session {
        session.workspace.clone()
    } else {
        config.resolve_workspace(workspace_arg)?
    };
    let context = github_context(config, &workspace)?;
    match &args.args.command {
        helm::github::operator::Command::References => {
            let session = session.as_ref().context("references requires --session")?;
            println!(
                "{}",
                safe_diagnostic(&serde_json::to_string_pretty(&session.github_references)?)
            );
            return Ok(());
        }
        helm::github::operator::Command::Unreference { url } => {
            anyhow::ensure!(
                context.policy.access_mode() != AccessMode::ReadOnly,
                "GitHub reference removal is denied in read-only mode"
            );
            let session = session.as_mut().context("unreference requires --session")?;
            let removed = session.forget_github(&helm::github::repository::Object::parse(url)?)?;
            owner.as_ref().expect("session owner").save(session).await?;
            println!("reference removed: {removed}");
            return Ok(());
        }
        _ => (),
    }
    let execution = helm::github::operator::execute_args(
        context.clone(),
        session.as_ref().map(|session| session.id),
        args.args,
    );
    let mut result = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => { signal?; context.cancellation.cancel(); anyhow::bail!("GitHub operation interrupted; inspect its receipt before repeating"); }
        result = execution => result?,
    };
    github_import(&context, &mut result).await?;
    if let Some(reference) = result.reference {
        context.policy.check_current()?;
        anyhow::ensure!(
            context.policy.access_mode() != AccessMode::ReadOnly,
            "GitHub reference update is denied in read-only mode"
        );
        let session = session.as_mut().context("reference needs --session")?;
        session.remember_github(reference)?;
        owner.as_ref().expect("session owner").save(session).await?;
    }
    println!("{}", safe_diagnostic(&result.display));
    Ok(())
}

async fn github_plain(
    config: &Config,
    agent: Option<&Agent>,
    store: &SessionStore,
    session: &mut Session,
    arguments: &str,
) -> Result<()> {
    let words = shell_words::split(arguments).context("invalid /github arguments")?;
    let context = github_context(config, &session.workspace)?;
    let authority = |write| -> Result<()> {
        if let Some(agent) = agent {
            agent.github_operator_authority(write)
        } else {
            context.policy.check_current()?;
            anyhow::ensure!(
                !write || context.policy.access_mode() != AccessMode::ReadOnly,
                "GitHub reference edits are denied in read-only mode"
            );
            Ok(())
        }
    };
    authority(false)?;
    if words.as_slice() == ["references"] {
        println!(
            "{}",
            safe_diagnostic(&serde_json::to_string_pretty(&session.github_references)?)
        );
        return Ok(());
    }
    if words.first().is_some_and(|word| word == "unreference") {
        anyhow::ensure!(words.len() == 2, "usage: /github unreference URL");
        authority(true)?;
        let removed =
            session.forget_github(&helm::github::repository::Object::parse(&words[1])?)?;
        store.save(session).await?;
        println!("reference removed: {removed}");
        return Ok(());
    }
    let cancel = context.cancellation.clone();
    let execution = async {
        if let Some(agent) = agent {
            agent
                .github_command(session.id, words, cancel.clone())
                .await
        } else {
            let mut result =
                helm::github::operator::execute(context.clone(), Some(session.id), words).await?;
            github_import(&context, &mut result).await?;
            Ok(result)
        }
    };
    let result = tokio::select! {
        biased;
        signal = tokio::signal::ctrl_c() => {
            signal?;
            cancel.cancel();
            anyhow::bail!("GitHub operation interrupted; inspect its receipt before repeating");
        }
        result = execution => result?,
    };
    authority(false)?;
    if let Some(reference) = result.reference {
        authority(true)?;
        session.remember_github(reference)?;
        store.save(session).await?;
    }
    println!("{}", safe_diagnostic(&result.display));
    Ok(())
}

async fn tui_chat(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    model_overridden: bool,
    verbose: bool,
    log_format: LogFormat,
) -> Result<()> {
    tui_runtime::chat(
        config,
        workspace_arg,
        resume,
        model_overridden,
        verbose,
        log_format,
    )
    .await
}

fn write_runtime_config(config: &Config) -> Result<tempfile::NamedTempFile> {
    write_runtime_config_in(config, &helm::config::default_data_dir())
}

fn write_runtime_config_in(
    config: &Config,
    directory: &std::path::Path,
) -> Result<tempfile::NamedTempFile> {
    std::fs::create_dir_all(directory)?;
    let mut document = toml::Value::try_from(config)?;
    if let Some(state) = &config.chat_preferences {
        document
            .as_table_mut()
            .context("configuration must be a table")?
            .insert("_chat_preferences".into(), toml::Value::try_from(state)?);
    }
    let contents = toml::to_string_pretty(&document)?;
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
    anyhow::ensure!(
        config.policy_profile.is_none() && config.policy_defaults.is_none(),
        "selected policy profile cannot cross a frontend relaunch; exit and explicitly reselect for the requested frontend"
    );
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

/// Once work starts, Ctrl-C requests cooperative cancellation instead of allowing
/// the OS to exit before finalization can return canonical recovery for saving.
async fn run_with_ctrl_c(
    agent: &Agent,
    history: Vec<helm::Message>,
    prompt: String,
    scope: Option<helm::completion::runtime::RunHandle>,
    checkpoint: &helm::session::SessionCheckpoint,
) -> std::result::Result<helm::agent::AgentOutcome, helm::agent::AgentError> {
    run_with_ctrl_c_and_secrets(agent, history, prompt, scope, checkpoint, None, None).await
}

async fn run_with_ctrl_c_and_secrets(
    agent: &Agent,
    history: Vec<helm::Message>,
    prompt: String,
    scope: Option<helm::completion::runtime::RunHandle>,
    checkpoint: &helm::session::SessionCheckpoint,
    bindings: Option<helm::workflow::secrets::RunBindings>,
    external_cancel: Option<tokio_util::sync::CancellationToken>,
) -> std::result::Result<helm::agent::AgentOutcome, helm::agent::AgentError> {
    let cancellation = external_cancel.clone().unwrap_or_default();
    let run = agent.run_checkpointed_scoped_with_workflow_secrets(
        history,
        prompt,
        cancellation.clone(),
        None,
        checkpoint,
        agent.model(),
        scope,
        bindings,
    );
    wait_for_run_interrupt(
        run,
        cancellation,
        async move {
            if let Some(token) = external_cancel {
                tokio::select! { _ = token.cancelled() => Ok(()), result = tokio::signal::ctrl_c() => result }
            } else { tokio::signal::ctrl_c().await }
        },
        std::time::Duration::from_secs(15),
    )
    .await
}

async fn wait_for_run_interrupt(
    run: impl std::future::Future<
        Output = std::result::Result<helm::agent::AgentOutcome, helm::agent::AgentError>,
    >,
    cancellation: tokio_util::sync::CancellationToken,
    interrupt: impl std::future::Future<Output = io::Result<()>>,
    cleanup_timeout: std::time::Duration,
) -> std::result::Result<helm::agent::AgentOutcome, helm::agent::AgentError> {
    tokio::pin!(run);
    tokio::select! {
        result = &mut run => result,
        signal = interrupt => {
            cancellation.cancel();
            if signal.is_err() {
                eprintln!("[interrupt handler unavailable; cancelling run]");
            } else {
                eprintln!("[interrupt requested; waiting for owned work to stop]");
            }
            match tokio::time::timeout(cleanup_timeout, &mut run).await {
                Ok(result) => result,
                Err(_) => Err(helm::agent::AgentError::Completion(
                    "cancellation cleanup timed out; completion was not confirmed".into(),
                )),
            }
        }
    }
}

/// Read exactly one idle prompt. Do not prefetch input intended for tool approval
/// or human questions while a run is active. A detached standard thread avoids
/// blocking Tokio shutdown if Ctrl-C ends chat while the terminal read is pending.
async fn read_plain_prompt() -> Result<Option<String>> {
    let (send, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("helm-plain-input".into())
        .spawn(move || {
            let mut prompt = String::new();
            let result = io::stdin()
                .read_line(&mut prompt)
                .map(|count| (count > 0).then_some(prompt));
            let _ = send.send(result);
        })?;
    wait_for_plain_input(receive, tokio::signal::ctrl_c()).await
}

async fn wait_for_plain_input(
    receive: tokio::sync::oneshot::Receiver<io::Result<Option<String>>>,
    interrupt: impl std::future::Future<Output = io::Result<()>>,
) -> Result<Option<String>> {
    tokio::select! {
        biased;
        signal = interrupt => { signal?; Ok(None) }
        result = receive => Ok(result.context("plain input reader stopped")??),
    }
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
    workflow: Option<(
        helm::workflow::Invocation,
        helm::workflow::secrets::SecretInputs,
    )>,
    cancellation: Option<tokio_util::sync::CancellationToken>,
) -> Result<Session> {
    anyhow::ensure!(
        !cancellation
            .as_ref()
            .is_some_and(|token| token.is_cancelled()),
        "workflow input cancelled"
    );
    let store = SessionStore::default();
    let (store, mut session) = if let Some(reference) = resume {
        store.load_owned(&reference).await?
    } else {
        let session = Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        );
        (store.with_execution(session.id).await?, session)
    };
    if model_overridden {
        session.switch_model(config.model.clone())?;
    }
    let mut active_config = config.clone();
    active_config.model = session.model.clone();
    let workflow_run = workflow.is_some();
    let mut secrets = None;
    if let Some((invocation, inputs)) = workflow {
        secrets = Some(inputs);
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
    let scope = agent.prepare_run(&session).await?;
    if let Some(scope) = &scope {
        session.completion_runs.push(scope.reference());
    }
    let history = session.messages.clone();
    session
        .messages
        .push(helm::Message::new(helm::Role::User, prompt.clone()));
    if let Some(scope) = &scope {
        session.begin_run_summary(scope.run_id());
    }
    if !no_save {
        store.save(&mut session).await?;
    }
    let run_id = scope
        .as_ref()
        .map(|scope| scope.run_id())
        .unwrap_or_else(uuid::Uuid::new_v4);
    let checkpoint = helm::session::SessionCheckpoint::new(
        session.clone(),
        (!no_save).then(|| store.clone()),
        run_id,
    );
    let bindings = secrets.map(|inputs| inputs.bind(run_id)).transpose()?;
    let result = run_with_ctrl_c_and_secrets(
        &agent,
        history,
        prompt,
        scope,
        &checkpoint,
        bindings,
        cancellation,
    )
    .await;
    session = checkpoint.snapshot_after_run(&result).await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => {
            if let Some(recovery) = error.recovery() {
                session.recover_context_failure(recovery)?;
            }
            if let helm::agent::AgentError::Finalization(failure) = &error {
                session.update_run_summary(
                    helm::agent::CompletionPhase::Interrupted,
                    failure.readiness.clone(),
                    None,
                );
            }
            session.interrupt_run_summary(safe_diagnostic(
                &redactor(&config).redact(error.to_string()),
            ));
            if !no_save {
                session.terminals = agent.terminal_metadata();
                store.save(&mut session).await?;
                eprintln!("[session {}]", session.id);
            }
            return Err(error.into());
        }
    };
    let completed = matches!(outcome.stop_reason, helm::agent::StopReason::Completed);
    let title_due = completed && session.title_due_after_turn();
    if completed {
        session.record_completed_turn();
    }
    session.replace_messages(outcome.messages);
    session.finish_run_summary(&outcome.stop_reason);
    session.usage.input_tokens += outcome.usage.input_tokens;
    session.usage.output_tokens += outcome.usage.output_tokens;
    session.terminals = agent.terminal_metadata();
    if !no_save {
        store.save(&mut session).await?;
        if title_due
            && let Some(result) = agent
                .generate_title_for_session(&session, tokio_util::sync::CancellationToken::new())
                .await
        {
            session.apply_generated_title(result);
            store.save(&mut session).await?;
        }
        eprintln!("[session {}]", session.id);
    }
    if let helm::agent::StopReason::Incomplete { reason, .. } = outcome.stop_reason {
        bail!("run incomplete: {}", safe_diagnostic(&reason));
    }
    Ok(session)
}

async fn chat(
    mut config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    interactive: bool,
    model_overridden: bool,
) -> Result<()> {
    let store = SessionStore::default();
    let (mut store, mut session) = if let Some(reference) = resume {
        store.load_owned(&reference).await?
    } else {
        let session = Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        );
        (store.with_execution(session.id).await?, session)
    };
    if model_overridden && session.switch_model(config.model.clone())? {
        store.save(&mut session).await?;
    }
    helm::chat_preferences::restore_profile(&mut config, &session.workspace)?;
    let mut agent: Option<Agent> = None;
    let mut pending = helm::plain_terminal::PendingInput::default();
    if interactive {
        eprintln!(
            "Helm · {} · {} · access: {} · {}\nType /help for commands.",
            session.display_name(),
            session.model,
            helm::runtime_policy::RuntimePolicy::resolve(&config, &session.workspace)?
                .config()
                .access_mode(),
            session.workspace.display()
        );
    }
    if interactive {
        helm::chat_preferences::remember(&config, &session.model, None)?;
    }
    let interrupt = attachment_interrupt();
    tokio::pin!(interrupt);
    loop {
        if interactive {
            print!("\nhelm> ");
            io::stdout().flush()?;
        }
        let next_prompt = if io::stdin().is_terminal() && io::stdout().is_terminal() {
            let cancel = tokio_util::sync::CancellationToken::new();
            let operation = helm::plain_terminal::pending_prompt(&mut pending, cancel.clone());
            tokio::pin!(operation);
            tokio::select! { biased;
                _ = &mut interrupt => { cancel.cancel(); let _ = (&mut operation).await; None }
                result = &mut operation => result?,
            }
        } else {
            read_plain_prompt().await?
        };
        let Some(prompt) = next_prompt else {
            break;
        };
        let prompt = prompt.trim();
        helm::chat_preferences::remember(&config, &session.model, None)?;
        if prompt.is_empty() {
            continue;
        }
        match prompt {
            "/quit" | "/exit" => break,
            "/help" => {
                println!(
                    "/help  /session  /inference  /new [TITLE]  /name TITLE  /access  /tools  /terminals  /terminal ID_OR_EXACT_NAME  /github COMMAND  /model [MODEL]  /models  /clear  /exit"
                );
                continue;
            }
            "/access" => {
                println!(
                    "{}",
                    helm::runtime_policy::RuntimePolicy::resolve(&config, &session.workspace)?
                        .config()
                        .access_mode()
                );
                continue;
            }
            "/name" => {
                eprintln!("usage: /name TITLE");
                continue;
            }
            "/inference" => {
                let id = session.id;
                let workspace = session.workspace.clone();
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tokio::task::spawn_blocking(move || {
                        helm::inference::cli::inspect_session(id, &workspace)
                    }),
                )
                .await
                {
                    Ok(Ok(Ok(()))) => {}
                    Ok(Ok(Err(error))) => eprintln!("{error}"),
                    _ => eprintln!(
                        "Local inference inspection unavailable; the conversation remains open"
                    ),
                }
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
        if prompt == "/terminals" || prompt == "/terminal" || prompt.starts_with("/terminal ") {
            let result: Result<bool> = async {
                if prompt == "/terminal" { anyhow::bail!("usage: /terminal ID_OR_EXACT_NAME (see /terminals)"); }
                let Some(current) = agent.as_ref() else {
                    if prompt == "/terminals" { println!("No live terminals in this workspace. Saved metadata cannot reattach a process.");return Ok(false); }
                    anyhow::bail!("No live terminals in this workspace. Saved metadata cannot reattach a process.");
                };
                let (manager, policy) = current.plain_terminals()?;
                let items = manager.list().await?;
                if prompt == "/terminals" {
                    if items.is_empty() { println!("No live terminals in this workspace."); }
                    for item in items { println!("{}  {:?}  {}", item.id, item.state, safe_diagnostic(&item.title)); }
                    return Ok(false);
                }
                let id = helm::plain_terminal::select(&items, prompt.strip_prefix("/terminal ").unwrap_or_default())?;
                let cancel = tokio_util::sync::CancellationToken::new();
                let operation = helm::plain_terminal::attach(manager, id, policy, cancel.clone());
                tokio::pin!(operation);
                let detached = tokio::select! { biased;
                    _ = &mut interrupt => { cancel.cancel(); let _ = (&mut operation).await; return Ok(true); }
                    result = &mut operation => result?,
                };
                pending.append(&detached.pending)?;
                if detached.delivery_failed { eprintln!("Private input delivery failed; its preceding bytes may be incomplete. Post-detach Helm input was retained."); }
                println!("{}", if detached.exited { "Terminal exited; returned to Helm." } else { "Detached; terminal remains private and running." });
                Ok(false)
            }.await;
            match result {
                Ok(true) => break,
                Ok(false) => (),
                Err(error) => {
                    let cleanup_failed = if error.is::<helm::plain_terminal::CleanupFailure>() {
                        true
                    } else if prompt != "/terminals" && io::stdin().is_terminal() {
                        // Failed selection/preflight also leaves an ambiguous private tail.
                        match helm::plain_terminal::discard_failed_attempt(&mut pending) {
                            Ok(()) => {
                                eprintln!(
                                    "Queued attachment input was discarded. No queued prompt was submitted; re-enter your next Helm command."
                                );
                                false
                            }
                            Err(_) => true,
                        }
                    } else {
                        false
                    };
                    if cleanup_failed {
                        // A failed privacy handoff cannot be retried as ordinary input.
                        if let Some(current) = agent.as_ref() {
                            let cleanup = current.shutdown_plain_terminals().await;
                            session.terminals = current.terminal_metadata();
                            store.save(&mut session).await?;
                            if !cleanup.observation_complete {
                                eprintln!("Terminal cleanup remains unobserved.");
                            }
                        }
                        return Err(helm::plain_terminal::CleanupFailure.into());
                    }
                    let message = agent
                        .as_ref()
                        .map(|current| current.redact_diagnostic(error.to_string()))
                        .unwrap_or_else(|| redactor(&config).redact(error.to_string()));
                    eprintln!("{}", safe_diagnostic(&message));
                }
            }
            continue;
        }
        if prompt == "/github" || prompt.starts_with("/github ") {
            if let Err(error) = github_plain(
                &config,
                agent.as_ref(),
                &store,
                &mut session,
                prompt.strip_prefix("/github").unwrap_or_default().trim(),
            )
            .await
            {
                eprintln!(
                    "{}",
                    safe_diagnostic(&redactor(&config).redact(error.to_string()))
                );
            }
            continue;
        }
        if prompt == "/new" || prompt.starts_with("/new ") {
            let name = prompt.strip_prefix("/new").unwrap_or_default().trim();
            let mut next = Session::new(session.workspace.clone(), session.model.clone());
            if !name.is_empty() {
                next.set_name(name.to_owned());
            }
            let next_owner = store.with_execution(next.id).await?;
            session = next;
            store = next_owner;
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
            for line in agent
                .as_ref()
                .expect("agent initialized")
                .tool_inventory_display()
            {
                println!("{line}");
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
            helm::chat_preferences::remember(&config, &session.model, None)?;
            println!("model switched to {model}");
            continue;
        }
        if prompt == "/models" {
            let mut active_config = config.clone();
            active_config.model = session.model.clone();
            let resolved =
                helm::runtime_policy::RuntimePolicy::resolve(&active_config, &session.workspace);
            let provider = resolved.and_then(|resolved| {
                let secrets = redactor(resolved.config());
                provider::from_config(resolved.config(), session.workspace.clone())
                    .map(|provider| (provider, secrets))
                    .map_err(anyhow::Error::from)
            });
            match provider {
                Ok((provider, secrets)) => {
                    match tokio::time::timeout(active_config.timeout(), provider.models()).await {
                        Ok(Ok(mut models)) => {
                            if let Err(error) =
                                helm::provider::validate_models_for_display(&models, |value| {
                                    secrets.contains_secret(value)
                                })
                            {
                                eprintln!("model discovery failed: {error}");
                                continue;
                            }
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
        let scope = match agent
            .as_ref()
            .expect("agent initialized")
            .prepare_run(&session)
            .await
        {
            Ok(scope) => scope,
            Err(helm::agent::AgentError::Policy(error)) => {
                eprintln!("Policy changed; restart or rebuild: {error}");
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if let Some(scope) = &scope {
            session.completion_runs.push(scope.reference());
        }
        let history = session.messages.clone();
        session
            .messages
            .push(helm::Message::new(helm::Role::User, prompt.to_owned()));
        if let Some(scope) = &scope {
            session.begin_run_summary(scope.run_id());
        }
        store.save(&mut session).await?;
        let checkpoint = helm::session::SessionCheckpoint::new(
            session.clone(),
            Some(store.clone()),
            scope
                .as_ref()
                .map(|scope| scope.run_id())
                .unwrap_or_else(uuid::Uuid::new_v4),
        );
        let result = run_with_ctrl_c(
            agent.as_ref().expect("agent initialized"),
            history,
            prompt.to_owned(),
            scope,
            &checkpoint,
        )
        .await;
        session = checkpoint.snapshot_after_run(&result).await;
        match result {
            Ok(outcome) => {
                let completed = matches!(outcome.stop_reason, helm::agent::StopReason::Completed);
                let title_due = completed && session.title_due_after_turn();
                if completed {
                    session.record_completed_turn();
                }
                session.replace_messages(outcome.messages);
                session.finish_run_summary(&outcome.stop_reason);
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
                        .generate_title_for_session(
                            &session,
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
                }
                if let helm::agent::AgentError::Finalization(failure) = &error {
                    session.update_run_summary(
                        helm::agent::CompletionPhase::Interrupted,
                        failure.readiness.clone(),
                        None,
                    );
                }
                session.interrupt_run_summary(safe_diagnostic(
                    &redactor(&config).redact(error.to_string()),
                ));
                store.save(&mut session).await?;
                eprintln!("[session {}]", session.id);
                eprintln!("error: {error:#}");
            }
        }
    }
    if let Some(current) = agent.take() {
        let cleanup = current.shutdown_plain_terminals().await;
        session.terminals = current.terminal_metadata();
        store.save(&mut session).await?;
        if !cleanup.observation_complete {
            eprintln!("Terminal cleanup remains unobserved.");
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
async fn prepare_workflow(
    args: helm::workflow::WorkflowArgs,
    workspace: &std::path::Path,
) -> Result<Option<helm::workflow::Prepared>> {
    match helm::workflow::prepare(args, workspace).await {
        Err(error)
            if error
                .downcast_ref::<helm::workflow::InputFailure>()
                .is_some() =>
        {
            let (message, status) = match error
                .downcast_ref::<helm::workflow::InputFailure>()
                .unwrap()
            {
                helm::workflow::InputFailure::Cancelled => ("workflow input cancelled", 130),
                helm::workflow::InputFailure::TimedOut => ("workflow input timed out", 1),
                helm::workflow::InputFailure::Restoration => {
                    ("workflow terminal restoration failed", 1)
                }
            };
            // A stalled terminal may hold stderr's lock. Restoration has already
            // completed; bound the notice and never await a blocked input thread.
            attachment_notice(message).await;
            std::process::exit(status)
        }
        result => result,
    }
}

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
