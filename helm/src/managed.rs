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
        /// Append unknown/interrupted results after terminal cleanup; never replay tools.
        #[arg(long, requires = "expected_revision")]
        reconcile_tools: Option<Uuid>,
        #[arg(long, requires = "reconcile_tools")]
        expected_revision: Option<u64>,
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
            human_record(&value)
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
fn human_record(value: &Value) -> String {
    fn text(value: &Value, key: &str) -> String {
        safe_diagnostic(value.get(key).and_then(Value::as_str).unwrap_or("unknown"))
    }
    match value["event"].as_str().unwrap_or("") {
        "session_created" => {
            let session = &value["session"];
            format!(
                "Created session {} (revision 0).\n{} · {}",
                text(session, "id"),
                text(session, "name"),
                text(session, "model")
            )
        }
        "session_list" => {
            let mut lines = Vec::new();
            for session in value["sessions"].as_array().into_iter().flatten() {
                let mut line = format!(
                    "{}  revision {}  {}",
                    text(session, "id"),
                    session["revision"],
                    text(session, "name")
                );
                if !session["active_run"].is_null() {
                    line.push_str(&format!(
                        "  active {} ({})",
                        text(&session["active_run"], "id"),
                        text(&session["active_run"], "state")
                    ));
                }
                if !session["pending_cleanup_run"].is_null() {
                    line.push_str(&format!(
                        "  cleanup required: {}",
                        text(session, "pending_cleanup_run")
                    ));
                }
                lines.push(line);
            }
            if lines.is_empty() {
                lines.push("No managed sessions.".into());
            }
            if !value["next_after"].is_null() {
                lines.push(format!("Next page: --after {}", text(value, "next_after")));
            }
            lines.join("\n")
        }
        "run_accepted" => format!(
            "Accepted run {} for session {}.\nRetry receipt: --expected-revision {} --command-id {} --expires-at-ms {}\nModel output is provisional until durable completion and cleanup.",
            text(value, "run_id"),
            text(value, "session_id"),
            value["expected_revision"],
            text(value, "command_id"),
            value["expires_at_ms"]
        ),
        "provisional_text" => safe_assistant(value["text"].as_str().unwrap_or("")),
        "thinking" => format!("[model turn {}]", value["turn"]),
        "tool_started" => format!("[tool {}]", text(value, "name")),
        "tool_finished" => format!(
            "[tool {}: {}]",
            text(value, "name"),
            if value["success"] == true {
                "done"
            } else {
                "failed"
            }
        ),
        "cancel_requested" => format!(
            "Cancellation requested for run {}. Work may still be stopping.",
            text(value, "run_id")
        ),
        "already_terminal" => format!(
            "Run {} is already {}.",
            text(value, "run_id"),
            text(value, "state")
        ),
        "existing_run" => format!(
            "Existing run {}: {}. No work replayed; use list to inspect cleanup obligations.",
            text(&value["run"], "id"),
            text(&value["run"], "state")
        ),
        "run_unconfirmed" => format!(
            "Run {} has unconfirmed execution; saved state {}; cleanup obligation retained.",
            text(&value["run"], "id"),
            text(&value["run"], "state")
        ),
        "run_terminal" => format!(
            "Run {}: {}; cleanup {}.",
            text(&value["run"], "id"),
            text(&value["run"], "state"),
            text(value, "cleanup")
        ),
        "session_recovered" if !value["reconciliation"].is_null() => format!(
            "Reconciled {} tool calls in run {}; revision {}. Outcomes remain unknown; no tools replayed.{}",
            value["reconciliation"]["tool_call_ids"]
                .as_array()
                .map_or(0, Vec::len),
            text(value, "run_id"),
            value["reconciliation"]["revision"],
            if value["reconciliation"]["duplicate"] == true {
                " Existing receipt."
            } else {
                ""
            }
        ),
        "session_recovered" => format!(
            "Session {} recovered; prior run {}. Cleanup {}. No tools replayed.",
            text(value, "session_id"),
            value["run"]
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("not active"),
            text(value, "cleanup")
        ),
        "journal_upgraded" => "Managed journal upgraded.".into(),
        _ => "Managed operation finished.".into(),
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
fn retry_busy<T>(mut operation: impl FnMut() -> Result<T>) -> Result<T> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match operation() {
            Err(error)
                if error.chain().any(|cause| {
                    matches!(cause.downcast_ref::<rusqlite::Error>(),
                Some(rusqlite::Error::SqliteFailure(code, _)) if matches!(code.code,
                    rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked))
                }) && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            result => return result,
        }
    }
}
fn open_journal(directory: &std::path::Path) -> Result<Journal> {
    retry_busy(|| Journal::open(directory.to_owned()))
}
fn record(run: &RunRecord) -> Value {
    json!({"id":run.id,"session_id":run.session_id,"command_id":run.command_id,
        "state":run.state,"usage":run.usage,"reason":run.terminal_reason})
}
fn run_result(actual: &RunRecord, observed: bool) -> Value {
    let terminal = !matches!(actual.state, RunState::Accepted | RunState::Running);
    json!({"event":if terminal {"run_terminal"}else{"run_unconfirmed"},"run":record(actual),
        "cleanup":if observed && terminal {"observed"}else{"unconfirmed"}})
}
pub(super) fn safe_error(error: anyhow::Error) -> anyhow::Error {
    let text = safe_diagnostic(&format!("{error:#}"));
    anyhow::anyhow!(text.chars().take(4096).collect::<String>())
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
            let mut journal = open_journal(&directory)?;
            journal.create_session(&session)?;
            output.emit(json!({"event":"session_created","session":{"id":session.id,"revision":0,"name":session.name,"model":session.model,"workspace":session.workspace}})).await
        }
        ManagedCommand::List { after, limit } => {
            let journal = open_journal(&directory)?;
            let page = journal.list_session_summaries(after, limit)?;
            output.emit(json!({"event":"session_list","sessions":page.sessions,"next_after":page.next_after})).await
        }
        ManagedCommand::Cancel { session, run } => {
            let mut journal = open_journal(&directory)?;
            let request = LocalCancelRequest {
                session_id: session,
                run_id: run,
                installation_id: actor.installation_id,
                principal_id: actor.principal_id,
                expires_at_ms: SystemClock
                    .now_ms()?
                    .checked_add(60_000)
                    .context("clock overflow")?,
            };
            let outcome = retry_busy(|| journal.request_cancel_local(&request))?;
            match outcome {
                CancelRequestOutcome::Requested { duplicate } => output.emit(json!({"event":"cancel_requested","session_id":session,"run_id":run,"duplicate":duplicate})).await,
                CancelRequestOutcome::AlreadyTerminal { state } => output.emit(json!({"event":"already_terminal","session_id":session,"run_id":run,"state":state})).await,
            }
        }
        ManagedCommand::Recover {
            session,
            acknowledge_cleanup,
            reconcile_tools,
            expected_revision,
        } => {
            ensure!(
                acknowledge_cleanup
                    .zip(reconcile_tools)
                    .is_none_or(|(a, b)| a == b),
                "cleanup attestation and reconciliation must select the same run"
            );
            let owner = ManagedSessionOwner::open(directory, session).await?;
            // A reconciliation CAS must refer to the caller's observed terminal revision.
            // Do not implicitly interrupt an active run and invalidate that revision first.
            let recovered = if reconcile_tools.is_none() {
                owner.recover_interrupted().await?
            } else {
                None
            };
            if let Some(run) = acknowledge_cleanup {
                owner.attest_local_cleanup(run, actor).await?;
            }
            let reconciliation = if let Some(run_id) = reconcile_tools {
                Some(
                    owner
                        .reconcile_local_tools(helm::attachment::journal::LocalReconcileRequest {
                            session_id: session,
                            run_id,
                            installation_id: actor.installation_id,
                            principal_id: actor.principal_id,
                            expected_revision: expected_revision
                                .context("reconciliation requires expected revision")?,
                        })
                        .await?,
                )
            } else {
                None
            };
            output.emit(json!({"event":"session_recovered","session_id":session,"run":recovered.as_ref().map(record),
                "run_id":reconcile_tools,"reconciliation":reconciliation,
                "cleanup":if acknowledge_cleanup.is_some(){"operator_attested"}else{"unchanged"}})).await
        }
        ManagedCommand::Upgrade => {
            let mut journal = open_journal(&directory)?;
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
        if let Some(value) = value
            && self.output.emit(value).await.is_err()
        {
            self.failed.store(true, Ordering::SeqCst);
            self.cancel.cancel();
        }
    }
}

// Keep this observation bounded independently of the foreground execution.
async fn poll_local_cancellation<F, Fut>(
    mut read: F,
    stopped: &tokio_util::sync::CancellationToken,
) -> anyhow::Result<bool>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<bool>>,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if stopped.is_cancelled() {
                return Ok(false);
            }
            match read().await {
                Err(error)
                    if error
                        .downcast_ref::<rusqlite::Error>()
                        .and_then(rusqlite::Error::sqlite_error_code)
                        == Some(rusqlite::ErrorCode::DatabaseBusy) =>
                {
                    // Independent managed commands can briefly hold the journal.
                    // Retry only this read, with the same run identity, after its
                    // transaction has returned. Other errors remain fail-closed.
                    // No blocking read is in flight here. A finished foreground
                    // run can stop its watcher without manufacturing a timeout.
                    tokio::select! {
                        biased;
                        _ = stopped.cancelled() => return Ok(false),
                        _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                    }
                }
                result => return result,
            }
        }
    })
    .await
    .context("durable cancellation polling deadline elapsed")?
}

async fn await_execution<T>(
    execution: impl std::future::Future<Output = T>,
    cancel: tokio_util::sync::CancellationToken,
    interrupt: impl std::future::Future<Output = ()>,
    grace: Duration,
) -> Option<T> {
    let mut execution = std::pin::pin!(execution);
    tokio::select! { biased;
        result=&mut execution=>Some(result),
        _=interrupt=> { cancel.cancel(); tokio::time::timeout(grace,&mut execution).await.ok() },
        _=cancel.cancelled()=>tokio::time::timeout(grace,&mut execution).await.ok(),
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
    let expires_at_ms = match expires_at_ms {
        Some(expiry) => expiry,
        None => SystemClock
            .now_ms()?
            .checked_add(300_000)
            .context("clock overflow")?,
    };
    let request = TurnAdmission {
        command_id,
        machine_id: actor.installation_id,
        principal_id: actor.principal_id,
        session_id: session,
        expected_revision,
        expires_at_ms,
        prompt: prompt.join(" "),
    };
    let journal = open_journal(&directory)?;
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
    std::time::Instant::now()
        .checked_add(config.timeout())
        .context("configured timeout exceeds the local clock range")?;
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
    let execution = execute_admitted(
        &owner,
        &mut run,
        &config,
        saved.session.workspace,
        progress.clone(),
        cancel,
        attachment_interrupt(),
        None,
    )
    .await?;
    let actual = execution.actual;
    let observed = execution.cleanup_observed;
    output.emit(run_result(&actual, observed)).await?;
    ensure!(
        !execution.construction_failed,
        "managed runtime construction failed; cleanup obligation retained"
    );
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

/// Actual durable terminal state and independent owned-resource cleanup observation.
/// An unconfirmed result retains the durable admission blocker.
pub(super) struct ManagedExecution {
    pub actual: RunRecord,
    pub cleanup_observed: bool,
    pub construction_failed: bool,
}
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_admitted(
    owner: &ManagedSessionOwner,
    run: &mut helm::attachment::runtime::RunOwner,
    config: &Config,
    workspace: PathBuf,
    sink: Arc<dyn EventSink>,
    cancel: tokio_util::sync::CancellationToken,
    interrupt: impl std::future::Future<Output = ()>,
    authority: Option<Arc<dyn helm::policy::ExecutionAuthority>>,
) -> Result<ManagedExecution> {
    use tokio_util::sync::CancellationToken;
    let run_id = run.record().await?.id;
    if authority.is_some() {
        run.configure_remote_redaction(redactor(config)).await?;
    }
    let resources = match build_authorized_agent_bundle(
        config,
        workspace,
        false,
        Some(sink),
        authority,
    )
    .await
    {
        Ok(resources) => resources,
        Err(_) => {
            let actual = run.fail_before_execution().await?;
            return Ok(ManagedExecution {
                actual,
                cleanup_observed: false,
                construction_failed: true,
            });
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
                match poll_local_cancellation(|| owner.local_cancel_requested(run_id), &done).await
                {
                    Ok(true) => {
                        cancel.cancel();
                        return Ok(());
                    }
                    Ok(false) => {}
                    _ => {
                        cancel.cancel();
                        bail!("durable cancellation polling failed");
                    }
                }
            }
        })
    };
    let result = await_execution(
        run.execute(&resources.agent, cancel.clone(), None),
        cancel.clone(),
        interrupt,
        Duration::from_secs(15),
    )
    .await;
    stop_watch.cancel();
    let watcher_ok = matches!(
        tokio::time::timeout(Duration::from_secs(6), watcher).await,
        Ok(Ok(Ok(())))
    );
    let retained = resources
        .resources
        .as_ref()
        .context("missing managed resources")?
        .close()?;
    let children_observed =
        tokio::time::timeout(Duration::from_secs(15), resources.subagents.shutdown())
            .await
            .is_ok();
    let terminal_reports = tokio::time::timeout(
        Duration::from_secs(15),
        futures_util::future::join_all(
            retained
                .terminals
                .iter()
                .map(|terminals| terminals.shutdown(Duration::from_secs(10))),
        ),
    )
    .await;
    let terminals_observed = terminal_reports
        .as_ref()
        .is_ok_and(|reports| reports.iter().all(|report| report.observation_complete));
    let shell_reports = tokio::time::timeout(
        Duration::from_secs(15),
        futures_util::future::join_all(
            retained
                .shells
                .iter()
                .map(|shell| shell.shutdown(Duration::from_secs(10))),
        ),
    )
    .await;
    let shells_observed = shell_reports
        .as_ref()
        .is_ok_and(|reports| reports.iter().all(|report| report.observation_complete));
    let mcp_reports = tokio::time::timeout(
        Duration::from_secs(15),
        futures_util::future::join_all(retained.mcp.iter().map(|server| server.shutdown())),
    )
    .await;
    let mcp_observed = retained.mcp.iter().all(|server| server.observed())
        && mcp_reports
            .as_ref()
            .is_ok_and(|reports| reports.iter().all(Result::is_ok));
    let actual = run.record().await?;
    let terminal_persisted = !matches!(actual.state, RunState::Accepted | RunState::Running);
    let observed = terminal_persisted
        && mcp_observed
        && children_observed
        && terminals_observed
        && shells_observed
        && result.is_some()
        && watcher_ok;
    drop(resources);
    if observed {
        run.confirm_local_cleanup_observed().await?;
    }
    Ok(ManagedExecution {
        actual,
        cleanup_observed: observed,
        construction_failed: false,
    })
}
