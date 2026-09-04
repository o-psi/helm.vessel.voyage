use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use helm::{
    Agent, AgentEvent, Config, EventSink,
    config::ApprovalMode,
    policy::Policy,
    provider,
    session::{Session, SessionStore},
    tools::{Approver, ToolContext, ToolRegistry},
    voyage::{Enrollment, EnrollmentStore, normalize_vessel_url},
};
use std::{
    io::{self, Write},
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

    let agent = build_agent(&config, workspace.clone()).await?;
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
                eprintln!("task {} failed: {error:#}", task.id);
                let _ = client
                    .post(format!("{vessel}/v1/worker/tasks/{}/failure", task.id))
                    .bearer_auth(&enrollment.worker_token)
                    .json(&TaskFailure {
                        task_id: task.id,
                        lease_id: task.lease_id,
                        error: format!("{error:#}"),
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
    },
    Sessions,
    Config,
}

struct Terminal;
#[async_trait]
impl Approver for Terminal {
    async fn approve(&self, reason: &str) -> bool {
        eprint!("\nApproval required: {reason}\nProceed? [y/N] ");
        let _ = io::stderr().flush();
        let mut answer = String::new();
        io::stdin().read_line(&mut answer).is_ok()
            && matches!(answer.trim().to_lowercase().as_str(), "y" | "yes")
    }
}
#[async_trait]
impl EventSink for Terminal {
    async fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::Thinking { turn } => eprintln!("[model turn {turn}]"),
            AgentEvent::AssistantText(text) => {
                if !text.is_empty() {
                    println!("{text}");
                }
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
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        .with_writer(io::stderr)
        .init();
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
    match cli.command.unwrap_or(Command::Chat { resume: None }) {
        Command::Config => {
            println!("{}", toml::to_string_pretty(&config)?);
            Ok(())
        }
        Command::Sessions => list_sessions().await,
        Command::Run {
            prompt,
            resume,
            no_save,
        } => execute(config, cli.workspace, resume, prompt.join(" "), no_save)
            .await
            .map(|_| ()),
        Command::Chat { resume } => chat(config, cli.workspace, resume).await,
    }
}

async fn build_agent(config: &Config, workspace: PathBuf) -> Result<Agent> {
    let policy = Arc::new(Policy::new(config, workspace)?);
    let terminal = Arc::new(Terminal);
    let context = ToolContext {
        policy,
        approver: terminal.clone(),
        timeout: config.timeout(),
        max_output_bytes: config.max_output_bytes,
        environment: config.env.clone(),
    };
    Ok(Agent::new(
        provider::from_config(config)?,
        ToolRegistry::standard(),
        context,
        terminal,
        config.model.clone(),
        config.system_prompt.clone(),
        config.max_turns,
        config.max_tokens,
        config.temperature,
    ))
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
    let agent = build_agent(&config, session.workspace.clone()).await?;
    let outcome = agent.run(session.messages.clone(), prompt).await?;
    session.messages = outcome.messages;
    session.usage.input_tokens += outcome.usage.input_tokens;
    session.usage.output_tokens += outcome.usage.output_tokens;
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
    let agent = build_agent(&config, session.workspace.clone()).await?;
    eprintln!(
        "Helm · {} · {}\nType /help for commands.",
        config.model,
        session.workspace.display()
    );
    loop {
        print!("\nhelm> ");
        io::stdout().flush()?;
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
                println!("conversation cleared");
                continue;
            }
            _ => {}
        }
        match agent.run(session.messages.clone(), prompt.to_owned()).await {
            Ok(outcome) => {
                session.messages = outcome.messages;
                session.usage.input_tokens += outcome.usage.input_tokens;
                session.usage.output_tokens += outcome.usage.output_tokens;
                store.save(&mut session).await?;
            }
            Err(error) => eprintln!("error: {error:#}"),
        }
    }
    store.save(&mut session).await?;
    eprintln!("Session saved as {}", session.id);
    Ok(())
}

async fn list_sessions() -> Result<()> {
    for session in SessionStore::default().list().await? {
        println!(
            "{}  {}  {}  {} messages",
            session.id,
            session.updated_at.format("%Y-%m-%d %H:%M UTC"),
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
