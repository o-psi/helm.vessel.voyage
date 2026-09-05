//! Explicit local managed-session frontend. No legacy SessionStore is reachable.
use super::*;
use anyhow::ensure;
use helm::attachment::{
    journal::{CancelRequestOutcome, Journal, LocalCancelRequest, RunRecord, RunState},
    local_actor::{LocalActor, LocalActorStore},
    runtime::{ManagedSessionOwner, RuntimeClock, SystemClock},
};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

#[derive(clap::Args)]
pub(super) struct Args {
    /// Dedicated absolute installation directory (alternate roots use distinct local identities).
    #[arg(long)]
    directory: PathBuf,
    #[arg(long)]
    json: bool,
    #[command(subcommand)]
    command: ManagedCommand,
}
#[derive(Subcommand)]
enum ManagedCommand {
    /// Create once. Use list to recover a lost response; creation never retries implicitly.
    Create {
        #[arg(long)]
        id: Option<Uuid>,
        #[arg(long)]
        name: Option<String>,
    },
    /// List bounded session metadata, excluding conversation and provider state.
    List {
        #[arg(long)]
        after: Option<Uuid>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Execute one foreground turn against an explicit saved revision.
    Submit {
        session: Uuid,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long, requires = "expires_at_ms")]
        command_id: Option<Uuid>,
        #[arg(long, requires = "command_id")]
        expires_at_ms: Option<i64>,
        #[arg(required = true)]
        prompt: Vec<String>,
    },
    /// Request cancellation of one exact run; requested does not mean stopped.
    Cancel {
        session: Uuid,
        #[arg(long)]
        run: Uuid,
    },
    /// Mark abandoned work interrupted without replaying tools.
    Recover {
        session: Uuid,
        /// Attest that you independently stopped this run's previous effects.
        #[arg(long)]
        acknowledge_cleanup: Option<Uuid>,
    },
    /// Explicitly upgrade a quiescent journal. Stop older Helm processes first.
    Upgrade,
}
impl Args {
    pub(super) fn administrative(&self) -> bool {
        !matches!(
            self.command,
            ManagedCommand::Create { .. } | ManagedCommand::Submit { .. }
        )
    }
}

// A blocked output pipe cannot occupy a Tokio worker or hold execution ownership.
// Queue capacity and per-write timeout bound the producer's resource use.
#[derive(Clone)]
struct Output {
    sender: std::sync::mpsc::SyncSender<(String, tokio::sync::oneshot::Sender<bool>)>,
    json: bool,
}
impl Output {
    fn new(json: bool) -> Result<Self> {
        let (sender, receiver) =
            std::sync::mpsc::sync_channel::<(String, tokio::sync::oneshot::Sender<bool>)>(16);
        std::thread::Builder::new()
            .name("managed-output".into())
            .spawn(move || {
                let mut stdout = io::stdout().lock();
                for (line, receipt) in receiver {
                    let written = writeln!(stdout, "{line}")
                        .and_then(|_| stdout.flush())
                        .is_ok();
                    let _ = receipt.send(written);
                    if !written {
                        break;
                    }
                }
            })?;
        Ok(Self { sender, json })
    }
    async fn emit(&self, value: Value) -> Result<()> {
        let line = if self.json {
            serde_json::to_string(&value)?
        } else {
            // Structured human output is control-safe even for stored workspace/name text.
            safe_diagnostic(&serde_json::to_string(&value)?)
        };
        ensure!(line.len() <= 1024 * 1024, "managed output record too large");
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.sender
            .try_send((line, sender))
            .map_err(|_| anyhow::anyhow!("managed output unavailable"))?;
        ensure!(
            tokio::time::timeout(Duration::from_secs(5), receiver)
                .await
                .context("managed output timed out")?
                .unwrap_or(false),
            "managed output failed"
        );
        Ok(())
    }
}
fn installation(
    directory: &std::path::Path,
    create: bool,
) -> Result<(LocalActorStore, LocalActor, PathBuf)> {
    ensure!(
        directory.is_absolute(),
        "managed directory must be absolute"
    );
    let journal = directory.join("journal");
    if !create {
        ensure!(
            directory.join("actor.json").exists() && journal.join("journal.sqlite3").exists(),
            "managed installation does not exist; create explicitly"
        );
    }
    let store = LocalActorStore::open(directory)?;
    let actor = store.identity()?;
    Ok((store, actor, journal))
}
fn record(run: &RunRecord) -> Value {
    json!({"id":run.id,"session_id":run.session_id,"command_id":run.command_id,
        "state":run.state,"usage":run.usage,"reason":run.terminal_reason})
}
pub(super) async fn run(
    args: Args,
    config: Option<Config>,
    workspace: Option<PathBuf>,
    model_overridden: bool,
) -> Result<()> {
    let output = Output::new(args.json)?;
    let create = matches!(args.command, ManagedCommand::Create { .. });
    let (_identity, actor, directory) = installation(&args.directory, create)?;
    match args.command {
        ManagedCommand::Create { id, name } => {
            let config = config.context("managed create requires configuration")?;
            let mut session = Session::new(config.resolve_workspace(workspace)?, config.model);
            if let Some(id) = id {
                ensure!(!id.is_nil(), "nil session ID");
                session.id = id;
            }
            if let Some(name) = name {
                ensure!(
                    !name.trim().is_empty()
                        && name.len() <= 256
                        && name.chars().all(|c| !c.is_control()),
                    "invalid session name"
                );
                session.set_name(name);
            }
            let mut journal = Journal::open(directory)?;
            journal.create_session(&session)?;
            output.emit(json!({"event":"session_created","session":{"id":session.id,"revision":0,"name":session.name,"model":session.model,"workspace":session.workspace}})).await
        }
        ManagedCommand::List { after, limit } => {
            let journal = Journal::open(directory)?;
            let page = journal.list_session_summaries(after, limit)?;
            output.emit(json!({"event":"session_list","sessions":page.sessions,"next_after":page.next_after})).await
        }
        ManagedCommand::Cancel { session, run } => {
            let mut journal = Journal::open(directory)?;
            let outcome = journal.request_cancel_local(&LocalCancelRequest {
                session_id: session,
                run_id: run,
                installation_id: actor.installation_id,
                principal_id: actor.principal_id,
                expires_at_ms: SystemClock
                    .now_ms()?
                    .checked_add(60_000)
                    .context("clock overflow")?,
            })?;
            match outcome {
                CancelRequestOutcome::Requested { duplicate } => output.emit(json!({"event":"cancel_requested","session_id":session,"run_id":run,"duplicate":duplicate})).await,
                CancelRequestOutcome::AlreadyTerminal { state } => output.emit(json!({"event":"already_terminal","session_id":session,"run_id":run,"state":state})).await,
            }
        }
        ManagedCommand::Recover {
            session,
            acknowledge_cleanup,
        } => {
            let owner = ManagedSessionOwner::open(directory, session).await?;
            let recovered = owner.recover_interrupted().await?;
            if let Some(run) = acknowledge_cleanup {
                owner.attest_local_cleanup(run, actor).await?;
            }
            output.emit(json!({"event":"session_recovered","session_id":session,"run":recovered.as_ref().map(record),"cleanup":if acknowledge_cleanup.is_some(){"operator_attested"}else{"unchanged"}})).await
        }
        ManagedCommand::Upgrade => {
            let mut journal = Journal::open(directory)?;
            journal.upgrade_quiescent()?;
            output.emit(json!({"event":"journal_upgraded"})).await
        }
        ManagedCommand::Submit {
            session,
            expected_revision,
            command_id,
            expires_at_ms,
            prompt,
        } => {
            submit(
                directory,
                actor,
                output,
                config.context("managed submit requires configuration")?,
                workspace,
                model_overridden,
                session,
                expected_revision,
                command_id,
                expires_at_ms,
                prompt,
            )
            .await
        }
    }
}

struct Progress {
    output: Output,
    cancel: tokio_util::sync::CancellationToken,
    failed: std::sync::atomic::AtomicBool,
    streamed: std::sync::atomic::AtomicBool,
}
#[async_trait]
impl EventSink for Progress {
    async fn emit(&self, event: AgentEvent) {
        use std::sync::atomic::Ordering;
        let value = match event {
            AgentEvent::AssistantTextDelta(text) => {
                self.streamed.store(true, Ordering::SeqCst);
                Some(json!({"event":"provisional_text","text":text}))
            }
            AgentEvent::AssistantText(text) => {
                if self.streamed.swap(false, Ordering::SeqCst) {
                    None
                } else {
                    Some(json!({"event":"provisional_text","text":text}))
                }
            }
            AgentEvent::Thinking { turn } => Some(json!({"event":"thinking","turn":turn})),
            AgentEvent::ToolStarted { name, .. } => {
                Some(json!({"event":"tool_started","name":name}))
            }
            AgentEvent::ToolFinished { name, success, .. } => {
                Some(json!({"event":"tool_finished","name":name,"success":success}))
            }
            // Agent completion is not authoritative until the journal's terminal transaction.
            // Provider diagnostics and raw tool arguments/results are not echoed.
            _ => None,
        };
        if let Some(value) = value {
            if self.output.emit(value).await.is_err() {
                self.failed.store(true, Ordering::SeqCst);
                self.cancel.cancel();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn submit(
    directory: PathBuf,
    actor: LocalActor,
    output: Output,
    mut config: Config,
    workspace: Option<PathBuf>,
    model_overridden: bool,
    session: Uuid,
    expected_revision: u64,
    command_id: Option<Uuid>,
    expires_at_ms: Option<i64>,
    prompt: Vec<String>,
) -> Result<()> {
    use helm::attachment::{journal::TurnAdmission, runtime::Admission};
    use tokio_util::sync::CancellationToken;
    ensure!(!session.is_nil(), "nil session ID");
    let command_id = command_id.unwrap_or_else(Uuid::new_v4);
    let expires_at_ms = expires_at_ms.unwrap_or(
        SystemClock
            .now_ms()?
            .checked_add(300_000)
            .context("clock overflow")?,
    );
    let request = TurnAdmission {
        command_id,
        machine_id: actor.installation_id,
        principal_id: actor.principal_id,
        session_id: session,
        expected_revision,
        expires_at_ms,
        prompt: prompt.join(" "),
    };
    let journal = Journal::open(directory.clone())?;
    if let Some(existing) = journal.lookup_command(&request)? {
        output
            .emit(json!({"event":"existing_run","run":record(&existing)}))
            .await?;
        return Ok(());
    }
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session).await?;
    let saved = owner.snapshot().await?;
    if let Some(workspace) = workspace {
        ensure!(
            config.resolve_workspace(Some(workspace))? == saved.session.workspace.canonicalize()?,
            "managed workspace does not match saved session"
        );
    }
    ensure!(
        !model_overridden || config.model == saved.session.model,
        "managed model does not match saved session"
    );
    config.model = saved.session.model.clone();
    ensure!(
        config.provider != helm::ProviderKind::CodexSubscription,
        "managed submit requires a native provider; compatibility bridge cleanup is not yet observable"
    );
    ensure!(
        config.access_mode() == AccessMode::ReadOnly || config.mcp_servers.is_empty(),
        "managed submit does not yet support effectful MCP servers; cleanup adapters are required"
    );
    // Validate cheap policy before admission. Runtime construction starts only after obligation.
    Policy::new(&config, saved.session.workspace.clone())?;
    let mut run = match owner.admit(request).await? {
        Admission::Existing(existing) => {
            output
                .emit(json!({"event":"existing_run","run":record(&existing)}))
                .await?;
            return Ok(());
        }
        Admission::New(run) => run,
    };
    run.register_local_cleanup().await?;
    let run_id = run.record().await?.id;
    if let Err(error) = output.emit(json!({"event":"run_accepted","session_id":session,"run_id":run_id,
        "command_id":command_id,"expected_revision":expected_revision,"expires_at_ms":expires_at_ms})).await {
        run.fail_before_execution().await?;
        run.confirm_local_cleanup_observed().await?; // No resources have been constructed.
        return Err(error);
    }
    let cancel = CancellationToken::new();
    let progress = Arc::new(Progress {
        output: output.clone(),
        cancel: cancel.clone(),
        failed: std::sync::atomic::AtomicBool::new(false),
        streamed: std::sync::atomic::AtomicBool::new(false),
    });
    let resources = match build_agent_bundle(
        &config,
        saved.session.workspace,
        false,
        Some(progress.clone()),
    )
    .await
    {
        Ok(resources) => resources,
        Err(_) => {
            let actual = run.fail_before_execution().await?;
            output
                .emit(json!({"event":"run_terminal","run":record(&actual),"cleanup":"unconfirmed"}))
                .await?;
            bail!("managed runtime construction failed; cleanup obligation retained")
        }
    };
    let stop_watch = CancellationToken::new();
    let _stop_on_drop = stop_watch.clone().drop_guard();
    let watcher = {
        let owner = owner.clone();
        let done = stop_watch.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! { biased;
                    _=done.cancelled()=> return Ok::<_,anyhow::Error>(()),
                    _=tokio::time::sleep(Duration::from_millis(50))=>{}
                }
                match tokio::time::timeout(
                    Duration::from_secs(5),
                    owner.local_cancel_requested(run_id),
                )
                .await
                {
                    Ok(Ok(true)) => {
                        cancel.cancel();
                        return Ok(());
                    }
                    Ok(Ok(false)) => {}
                    _ => {
                        cancel.cancel();
                        bail!("durable cancellation polling failed");
                    }
                }
            }
        })
    };
    let result = {
        let mut execution = std::pin::pin!(run.execute(&resources.agent, cancel.clone(), None));
        let result = tokio::select! { biased;
            result=&mut execution=>Some(result),
            _=attachment_interrupt()=> {cancel.cancel(); tokio::time::timeout(Duration::from_secs(15), &mut execution).await.ok()},
        _=cancel.cancelled()=> tokio::time::timeout(Duration::from_secs(15), &mut execution).await.ok(),
        };
        result
    };
    stop_watch.cancel();
    let watcher_ok = matches!(
        tokio::time::timeout(Duration::from_secs(6), watcher).await,
        Ok(Ok(Ok(())))
    );
    let children_observed =
        tokio::time::timeout(Duration::from_secs(15), resources.subagents.shutdown())
            .await
            .is_ok();
    // The terminal helper is integrated in the next dependency commit.
    let terminals_observed = resources.terminals.is_none();
    let observed = children_observed && terminals_observed && result.is_some() && watcher_ok;
    drop(resources);
    if observed {
        run.confirm_local_cleanup_observed().await?;
    }
    let actual = run.record().await?;
    output.emit(json!({"event":"run_terminal","run":record(&actual),"cleanup":if observed{"observed"}else{"unconfirmed"}})).await?;
    ensure!(
        observed,
        "managed cleanup unconfirmed; obligation retained; recover explicitly"
    );
    ensure!(
        !progress.failed.load(std::sync::atomic::Ordering::SeqCst),
        "managed output failed"
    );
    ensure!(
        actual.state == RunState::Completed,
        "managed run did not complete"
    );
    Ok(())
}
