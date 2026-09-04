use anyhow::Result;
use async_trait::async_trait;
use clap::{Parser, Subcommand};
use helm::{
    Agent, AgentEvent, Config, EventSink,
    config::ApprovalMode,
    policy::Policy,
    provider,
    session::{Session, SessionStore},
    tools::{Approver, ToolContext, ToolRegistry},
};
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};
use tracing_subscriber::EnvFilter;
use voyage_protocol::{
    ApiError, HealthResponse, HeartbeatRequest, HelmDescriptor, RegistrationRequest, TaskRequest,
    TaskResponse,
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
    #[command(subcommand)]
    command: Option<Command>,
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
    /// Expose this Helm for remote work and optional Vessel management.
    Serve {
        #[arg(long, default_value = "127.0.0.1:9470")]
        bind: String,
        #[arg(long)]
        public_url: Option<String>,
        #[arg(long)]
        vessel: Option<String>,
        #[arg(long, env = "HELM_SERVER_TOKEN")]
        token: Option<String>,
        #[arg(long, default_value = "helm")]
        name: String,
    },
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
    match cli.command.unwrap_or(Command::Chat { resume: None }) {
        Command::Config => {
            println!("{}", toml::to_string_pretty(&config)?);
            Ok(())
        }
        Command::Sessions => list_sessions().await,
        Command::Serve {
            bind,
            public_url,
            vessel,
            token,
            name,
        } => serve(config, cli.workspace, bind, public_url, vessel, token, name).await,
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

#[derive(Clone)]
struct ServeState {
    agent: Arc<Agent>,
    store: SessionStore,
    descriptor: HelmDescriptor,
    token: Option<String>,
}

async fn serve(
    config: Config,
    workspace_arg: Option<PathBuf>,
    bind: String,
    public_url: Option<String>,
    vessel: Option<String>,
    token: Option<String>,
    name: String,
) -> Result<()> {
    use axum::{
        Router,
        routing::{get, post},
    };
    let workspace = config.resolve_workspace(workspace_arg)?;
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let endpoint = public_url.unwrap_or_else(|| format!("http://{bind}"));
    let descriptor = HelmDescriptor {
        id: uuid::Uuid::new_v4(),
        name,
        endpoint,
        version: env!("CARGO_PKG_VERSION").into(),
        model: config.model.clone(),
        capabilities: vec!["shell".into(), "filesystem".into(), "search".into()],
    };
    let state = ServeState {
        agent: Arc::new(build_agent(&config, workspace.clone()).await?),
        store: SessionStore::default(),
        descriptor,
        token,
    };
    if let Some(vessel_url) = vessel {
        spawn_registration(vessel_url, state.descriptor.clone());
    }
    let app = Router::new()
        .route("/health", get(health))
        .route("/v1/info", get(info))
        .route("/v1/tasks", post(remote_task))
        .with_state(state.clone());
    eprintln!(
        "Helm {} serving {} from {}",
        state.descriptor.id,
        bind,
        workspace.display()
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn health() -> axum::Json<HealthResponse> {
    axum::Json(HealthResponse {
        status: "ok".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}
async fn info(
    axum::extract::State(state): axum::extract::State<ServeState>,
) -> axum::Json<HelmDescriptor> {
    axum::Json(state.descriptor)
}

async fn remote_task(
    axum::extract::State(state): axum::extract::State<ServeState>,
    headers: axum::http::HeaderMap,
    axum::Json(request): axum::Json<TaskRequest>,
) -> Result<axum::Json<TaskResponse>, (axum::http::StatusCode, axum::Json<ApiError>)> {
    authorize(&state, &headers)?;
    let mut session = match request.session_id {
        Some(id) => state.store.load(id).await.map_err(internal)?,
        None => Session::new(
            state.agent_workspace().to_owned(),
            state.descriptor.model.clone(),
        ),
    };
    let outcome = state
        .agent
        .run(session.messages.clone(), request.prompt)
        .await
        .map_err(internal)?;
    session.messages = outcome.messages;
    session.usage.input_tokens += outcome.usage.input_tokens;
    session.usage.output_tokens += outcome.usage.output_tokens;
    state.store.save(&mut session).await.map_err(internal)?;
    Ok(axum::Json(TaskResponse {
        session_id: session.id,
        answer: outcome.answer,
        input_tokens: outcome.usage.input_tokens,
        output_tokens: outcome.usage.output_tokens,
    }))
}

impl ServeState {
    fn agent_workspace(&self) -> &std::path::Path {
        self.agent.workspace()
    }
}

fn authorize(
    state: &ServeState,
    headers: &axum::http::HeaderMap,
) -> Result<(), (axum::http::StatusCode, axum::Json<ApiError>)> {
    let Some(expected) = &state.token else {
        return Ok(());
    };
    let supplied = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if supplied == Some(expected) {
        Ok(())
    } else {
        Err((
            axum::http::StatusCode::UNAUTHORIZED,
            axum::Json(ApiError {
                error: "invalid or missing bearer token".into(),
            }),
        ))
    }
}

fn internal(error: impl std::fmt::Display) -> (axum::http::StatusCode, axum::Json<ApiError>) {
    (
        axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(ApiError {
            error: error.to_string(),
        }),
    )
}

fn spawn_registration(vessel: String, helm: HelmDescriptor) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let base = vessel.trim_end_matches('/');
        if let Err(error) = client
            .post(format!("{base}/v1/helms/register"))
            .json(&RegistrationRequest { helm: helm.clone() })
            .send()
            .await
        {
            eprintln!("Vessel registration failed: {error}");
            return;
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            if let Err(error) = client
                .post(format!("{base}/v1/helms/heartbeat"))
                .json(&HeartbeatRequest { id: helm.id })
                .send()
                .await
            {
                eprintln!("Vessel heartbeat failed: {error}");
            }
        }
    });
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
