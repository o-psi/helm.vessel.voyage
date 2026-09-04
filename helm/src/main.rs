use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use helm::{
    Agent, AgentEvent, Config, EventSink,
    agent::RetryPolicy,
    config::{ApprovalMode, UnattendedApprovalMode},
    policy::Policy,
    provider,
    session::{Session, SessionStore},
    subagent::{
        AgentBudget, AgentPolicy, ApprovalPolicy, ExecutionContext, RuntimeLimits,
        SubagentExecutor, SubagentResult, SubagentRuntime, SubagentTool,
    },
    tools::{
        ApprovalOutcome, ApprovalRequest, Approver, InteractionMode, Redactor, ToolContext,
        ToolRegistry, UnattendedApprover,
    },
    voyage::{Enrollment, EnrollmentStore, normalize_vessel_url},
};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
};
use tracing_subscriber::EnvFilter;
use voyage_protocol::{
    HeartbeatRequest, HelmDescriptor, HelmStatus, PROTOCOL_VERSION, PairingStartRequest,
    PairingStartResponse, PairingStatus, TaskCompletion, TaskEnvelope, TaskFailure, TaskResult,
};

#[derive(Parser)]
#[command(version, about = "A general-purpose LLM harness for terminal work")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    model: Option<String>,
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true, value_enum)]
    approval: Option<ApprovalArg>,
    #[arg(short, long, global = true)]
    verbose: bool,
    #[arg(long, global = true, value_enum, default_value = "text")]
    log_format: LogFormat,
    /// Pair with Vessel and work over an outbound connection.
    #[arg(long, global = true, num_args = 0..=1, default_missing_value = "http://127.0.0.1:9480")]
    voyage: Option<String>,
    #[arg(long, global = true, default_value = "helm")]
    name: String,
    #[command(subcommand)]
    command: Option<Command>,
}

async fn voyage_worker(
    config: Config,
    workspace_arg: Option<PathBuf>,
    vessel: String,
    name: String,
) -> Result<()> {
    let vessel = normalize_vessel_url(&vessel)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let workspace = config.resolve_workspace(workspace_arg)?;
    let enrollment_store = EnrollmentStore::new(EnrollmentStore::default_path()?);
    let saved = enrollment_store.load().await?;
    let descriptor = HelmDescriptor {
        id: saved
            .as_ref()
            .filter(|e| e.vessel_url == vessel)
            .map_or_else(uuid::Uuid::new_v4, |e| e.helm_id),
        name,
        version: env!("CARGO_PKG_VERSION").into(),
        model: config.model.clone(),
        capabilities: vec!["shell".into(), "filesystem".into(), "search".into()],
    };
    let enrollment = if let Some(saved) = saved.filter(|e| e.vessel_url == vessel) {
        eprintln!("Reconnecting to Vessel as {}", saved.helm_id);
        saved
    } else {
        let pairing: PairingStartResponse = client
            .post(format!("{vessel}/v1/pairings/start"))
            .json(&PairingStartRequest {
                helm: descriptor.clone(),
                protocol_version: PROTOCOL_VERSION,
            })
            .send()
            .await
            .context("could not reach Vessel")?
            .error_for_status()
            .context("Vessel rejected pairing")?
            .json()
            .await
            .context("invalid pairing response")?;

        println!("{}", pairing.connection_string());
        eprintln!(
            "Give this one-time string to Vessel. Waiting for approval until {}…",
            pairing.expires_at
        );
        loop {
            if chrono::Utc::now() >= pairing.expires_at {
                bail!("pairing code expired");
            }
            let status: PairingStatus = client
                .get(format!("{vessel}/v1/pairings/{}", pairing.code))
                .bearer_auth(&pairing.worker_token)
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if status.claimed {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        let enrollment = Enrollment {
            helm_id: descriptor.id,
            vessel_url: vessel.clone(),
            worker_token: pairing.worker_token,
            name: descriptor.name.clone(),
        };
        enrollment_store.save(&enrollment).await?;
        enrollment
    };
    eprintln!(
        "Paired with Vessel as {}. Helm uses outbound connections only.",
        descriptor.id
    );

    let agent = build_agent(&config, workspace.clone(), false).await?;
    let store = SessionStore::default();
    let mut last_heartbeat = std::time::Instant::now() - std::time::Duration::from_secs(60);
    let mut reconnect_delay = std::time::Duration::from_secs(1);
    loop {
        if last_heartbeat.elapsed() >= std::time::Duration::from_secs(20) {
            let heartbeat = client
                .post(format!("{vessel}/v1/worker/heartbeat"))
                .bearer_auth(&enrollment.worker_token)
                .json(&HeartbeatRequest {
                    status: HelmStatus::Online,
                    protocol_version: PROTOCOL_VERSION,
                })
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if let Err(error) = heartbeat {
                eprintln!("Vessel connection lost: {error}; retrying in {reconnect_delay:?}");
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(std::time::Duration::from_secs(30));
                continue;
            }
            last_heartbeat = std::time::Instant::now();
            reconnect_delay = std::time::Duration::from_secs(1);
        }
        let response = match client
            .get(format!("{vessel}/v1/worker/tasks/next"))
            .bearer_auth(&enrollment.worker_token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
        {
            Ok(response) => response,
            Err(error) => {
                eprintln!("could not pull task: {error}; retrying in {reconnect_delay:?}");
                tokio::time::sleep(reconnect_delay).await;
                reconnect_delay = (reconnect_delay * 2).min(std::time::Duration::from_secs(30));
                continue;
            }
        };
        let task: Option<TaskEnvelope> = match response.json().await {
            Ok(task) => task,
            Err(error) => {
                eprintln!("invalid task response: {error}");
                continue;
            }
        };
        reconnect_delay = std::time::Duration::from_secs(1);
        let Some(task) = task else {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            continue;
        };
        eprintln!("[Vessel task {}]", task.id);
        let mut session = match task.request.session_id {
            Some(id) => store.load(id).await?,
            None => Session::new(workspace.clone(), config.model.clone()),
        };
        match agent
            .run(session.messages.clone(), task.request.prompt)
            .await
        {
            Ok(outcome) => {
                session.messages = outcome.messages;
                session.usage.input_tokens += outcome.usage.input_tokens;
                session.usage.output_tokens += outcome.usage.output_tokens;
                store.save(&mut session).await?;
                let result = TaskResult {
                    task_id: task.id,
                    session_id: session.id,
                    answer: outcome.answer,
                    input_tokens: outcome.usage.input_tokens,
                    output_tokens: outcome.usage.output_tokens,
                };
                client
                    .post(format!("{vessel}/v1/worker/tasks/{}/result", task.id))
                    .bearer_auth(&enrollment.worker_token)
                    .json(&TaskCompletion {
                        lease_id: task.lease_id,
                        result,
                    })
                    .send()
                    .await?
                    .error_for_status()?;
            }
            Err(error) => {
                let safe_error = redactor(&config).redact(format!("{error:#}"));
                eprintln!("task {} failed: {safe_error}", task.id);
                let _ = client
                    .post(format!("{vessel}/v1/worker/tasks/{}/failure", task.id))
                    .bearer_auth(&enrollment.worker_token)
                    .json(&TaskFailure {
                        task_id: task.id,
                        lease_id: task.lease_id,
                        error: safe_error,
                        retryable: true,
                    })
                    .send()
                    .await;
            }
        }
    }
}

#[derive(Clone, clap::ValueEnum)]
enum ApprovalArg {
    Always,
    OnRisk,
    Never,
}
#[derive(Clone, clap::ValueEnum)]
enum LogFormat {
    Text,
    Json,
}
#[derive(Subcommand)]
enum Command {
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
    Config,
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

#[derive(Default)]
struct Terminal {
    streamed: std::sync::Mutex<bool>,
}
#[async_trait]
impl Approver for Terminal {
    async fn approve(&self, request: &ApprovalRequest) -> ApprovalOutcome {
        eprint!(
            "\nApproval {} required for {} on {}:\n{}\nProceed? [y/N] ",
            request.id, request.action, request.target, request.reason
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
                print!("{text}");
                let _ = io::stdout().flush();
                *self
                    .streamed
                    .lock()
                    .expect("terminal stream state poisoned") = true;
            }
            AgentEvent::AssistantText(text) => {
                let mut streamed = self
                    .streamed
                    .lock()
                    .expect("terminal stream state poisoned");
                if *streamed {
                    println!();
                } else if !text.is_empty() {
                    println!("{text}");
                }
                *streamed = false;
            }
            AgentEvent::ToolStarted { name, arguments } => eprintln!("[tool {name}] {arguments}"),
            AgentEvent::ToolFinished {
                name,
                result,
                success,
            } => eprintln!(
                "[{} {name}] {}",
                if success { "done" } else { "error" },
                summarize(&result)
            ),
            AgentEvent::ProviderRetry {
                attempt,
                delay,
                error,
            } => eprintln!(
                "[provider retry {attempt} in {:.1}s] {error}",
                delay.as_secs_f32()
            ),
            AgentEvent::Cancelled => eprintln!("[cancelled]"),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let filter = if cli.verbose {
        "helm=debug"
    } else {
        "helm=warn"
    };
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into());
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
    let mut config = Config::load(cli.config.as_deref())?;
    if let Some(model) = cli.model {
        config.model = model;
    }
    if let Some(approval) = cli.approval {
        config.approval = match approval {
            ApprovalArg::Always => ApprovalMode::Always,
            ApprovalArg::OnRisk => ApprovalMode::OnRisk,
            ApprovalArg::Never => ApprovalMode::Never,
        };
    }
    if let Some(vessel) = cli.voyage {
        return voyage_worker(config, cli.workspace, vessel, cli.name).await;
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
            println!("{}", toml::to_string_pretty(&config)?);
            Ok(())
        }
        Command::Doctor => doctor(&config, cli.workspace),
        Command::Sessions => list_sessions().await,
        Command::Run {
            prompt,
            resume,
            no_save,
        } => execute(config, cli.workspace, resume, prompt.join(" "), no_save)
            .await
            .map(|_| ()),
        Command::Chat { resume, plain } => {
            let interactive = io::stdin().is_terminal() && io::stdout().is_terminal();
            let full_screen = interactive
                && std::env::var("TERM").is_ok_and(|term| !term.is_empty() && term != "dumb");
            if plain || !full_screen {
                chat(config, cli.workspace, resume, interactive).await
            } else {
                tui_chat(config, cli.workspace, resume).await
            }
        }
    }
}

fn doctor(config: &Config, workspace: Option<PathBuf>) -> Result<()> {
    let workspace = config.resolve_workspace(workspace)?;
    let subscription = (config.provider == helm::ProviderKind::CodexSubscription)
        .then(|| probe_codex_subscription(&config.codex_command));
    let provider_ready = match &subscription {
        Some(probe) => probe.executable && probe.app_server && probe.logged_in,
        None => std::env::var_os(&config.api_key_env).is_some(),
    };
    let report = serde_json::json!({
        "status": if provider_ready { "ok" } else { "action_required" },
        "version": env!("CARGO_PKG_VERSION"),
        "workspace": workspace,
        "workspace_readable": workspace.is_dir(),
        "provider_ready": provider_ready,
        "provider_credential_present": if config.provider == helm::ProviderKind::CodexSubscription { serde_json::Value::Null } else { serde_json::Value::Bool(std::env::var_os(&config.api_key_env).is_some()) },
        "codex_subscription": subscription,
        "sessions_directory": helm::config::default_data_dir().join("sessions"),
        "approval": config.approval,
        "unattended_approval": config.unattended_approval,
        "inherited_environment": config.inherit_env,
        "mcp_servers": config.mcp_servers.keys().collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

#[derive(serde::Serialize)]
struct CodexSubscriptionProbe {
    executable: bool,
    version: Option<String>,
    app_server: bool,
    logged_in: bool,
    remediation: Option<&'static str>,
}

fn probe_codex_subscription(command: &str) -> CodexSubscriptionProbe {
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
    CodexSubscriptionProbe {
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
}
#[async_trait]
impl SubagentExecutor for CliSubagentExecutor {
    async fn execute(
        &self,
        context: ExecutionContext,
    ) -> std::result::Result<SubagentResult, String> {
        context.progress("initializing provider and tools").await;
        let mut config = self.config.clone();
        config.workspace = Some(
            context
                .worktree
                .clone()
                .unwrap_or_else(|| self.workspace.clone()),
        );
        config.allow_read = context.policy.readable_roots.clone();
        config.allow_write = context.policy.writable_roots.clone();
        config.max_turns = (context.budget.max_turns as usize).min(config.max_turns);
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
        let mut tools = build_tools(&config, None)
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
            config.max_turns,
            config.max_tokens,
            config.temperature,
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: config.provider_retry_attempts,
            initial_delay: std::time::Duration::from_millis(config.provider_retry_initial_ms),
            max_delay: std::time::Duration::from_millis(config.provider_retry_max_ms),
        });
        let outcome = agent
            .run_with_cancel(Vec::new(), context.task, context.cancellation)
            .await
            .map_err(|e| e.to_string())?;
        Ok(SubagentResult {
            summary: outcome.answer,
        })
    }
}

async fn build_subagents(config: &Config, workspace: &std::path::Path) -> Result<SubagentTool> {
    let standard = ToolRegistry::standard();
    let allowed_tools = standard
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    let budget = AgentBudget {
        max_turns: config.max_turns.min(u32::MAX as usize) as u32,
        max_tokens: config.max_tokens as u64,
        max_runtime_secs: config.command_timeout_secs,
        max_children: 8,
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
    let executor = Arc::new(CliSubagentExecutor {
        config: config.clone(),
        workspace: workspace.to_path_buf(),
    });
    let store = helm::subagent::AgentTreeStore::new(
        helm::config::default_data_dir().join("subagents.json"),
    );
    let runtime = SubagentRuntime::new_persistent(
        executor,
        RuntimeLimits {
            max_concurrency: config.subagent_max_concurrency,
            max_agents: config.subagent_max_agents,
            event_history: config.subagent_event_history,
        },
        store,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    Ok(SubagentTool::new(Arc::new(runtime), policy, budget))
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
    let tools = build_tools(config, Some(subagents)).await?;
    Ok(Agent::new(
        provider::from_config(config, context.policy.workspace().to_owned())?,
        tools,
        context,
        terminal,
        config.model.clone(),
        config.system_prompt.clone(),
        config.max_turns,
        config.max_tokens,
        config.temperature,
    )
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
        .chain(config.api_key().ok());
    Arc::new(Redactor::new(secrets))
}

async fn tui_chat(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
) -> Result<()> {
    let store = SessionStore::default();
    let session = if let Some(reference) = resume {
        store.load_reference(&reference).await?
    } else {
        Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        )
    };
    let (bridge, receiver) = helm::tui::bridge();
    let policy = Arc::new(Policy::new(&config, session.workspace.clone())?);
    let context = ToolContext {
        policy,
        approver: bridge.clone(),
        timeout: config.timeout(),
        max_output_bytes: config.max_output_bytes,
        environment: tool_environment(&config),
        cancellation: tokio_util::sync::CancellationToken::new(),
        execution_id: uuid::Uuid::new_v4(),
        interaction: InteractionMode::Attended,
        redactor: redactor(&config),
    };
    let subagents = build_subagents(&config, &session.workspace).await?;
    let tools = build_tools(&config, Some(subagents)).await?;
    let terminals: Arc<dyn helm::terminal::InteractiveTerminals> =
        Arc::new(tools.terminals().unwrap_or_default());
    let agent = Arc::new(
        Agent::new(
            provider::from_config(&config, session.workspace.clone())?,
            tools,
            context,
            bridge.clone(),
            config.model.clone(),
            config.system_prompt.clone(),
            config.max_turns,
            config.max_tokens,
            config.temperature,
        )
        .with_retry_policy(RetryPolicy {
            max_attempts: config.provider_retry_attempts,
            initial_delay: std::time::Duration::from_millis(config.provider_retry_initial_ms),
            max_delay: std::time::Duration::from_millis(config.provider_retry_max_ms),
        }),
    );
    helm::tui::run(agent, store, session, receiver, bridge.sender(), terminals).await
}

async fn build_tools(config: &Config, subagents: Option<SubagentTool>) -> Result<ToolRegistry> {
    let mut tools = ToolRegistry::standard_with_terminal_limits(
        config.terminal_max_count,
        config.terminal_max_unread_bytes,
    );
    if let Some(tool) = subagents {
        tools.register_subagents(tool)?;
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
    let agent = build_agent(&config, session.workspace.clone(), true).await?;
    let outcome = agent.run(session.messages.clone(), prompt).await?;
    session.messages = outcome.messages;
    session.usage.input_tokens += outcome.usage.input_tokens;
    session.usage.output_tokens += outcome.usage.output_tokens;
    session.terminals = agent.terminal_metadata();
    if !no_save {
        store.save(&mut session).await?;
        eprintln!("[session {}]", session.id);
    }
    Ok(session)
}

async fn chat(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    interactive: bool,
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
    let mut agent = None;
    if interactive {
        eprintln!(
            "Helm · {} · {}\nType /help for commands.",
            config.model,
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
                println!("/help  /session  /clear  /exit");
                continue;
            }
            "/session" => {
                println!(
                    "{} ({} input, {} output tokens)",
                    session.id, session.usage.input_tokens, session.usage.output_tokens
                );
                continue;
            }
            "/clear" => {
                session.messages.clear();
                store.save(&mut session).await?;
                println!("conversation cleared");
                continue;
            }
            _ => {}
        }
        if agent.is_none() {
            agent = Some(build_agent(&config, session.workspace.clone(), interactive).await?);
        }
        match agent
            .as_ref()
            .expect("agent initialized")
            .run(session.messages.clone(), prompt.to_owned())
            .await
        {
            Ok(outcome) => {
                session.messages = outcome.messages;
                session.usage.input_tokens += outcome.usage.input_tokens;
                session.usage.output_tokens += outcome.usage.output_tokens;
                session.terminals = agent
                    .as_ref()
                    .expect("agent initialized")
                    .terminal_metadata();
                store.save(&mut session).await?;
            }
            Err(error) => eprintln!("error: {error:#}"),
        }
    }
    if interactive && !session.messages.is_empty() {
        eprintln!("Session saved as {}", session.id);
    }
    Ok(())
}

async fn list_sessions() -> Result<()> {
    for session in SessionStore::default().list().await? {
        println!(
            "{}  {}  {:<24}  {}  {} messages",
            session.id,
            session.updated_at.format("%Y-%m-%d %H:%M UTC"),
            session.name.as_deref().unwrap_or("untitled"),
            session.workspace.display(),
            session.messages.len()
        );
    }
    Ok(())
}
fn summarize(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default();
    if line.len() > 160 {
        format!("{}…", &line[..160])
    } else {
        line.into()
    }
}
