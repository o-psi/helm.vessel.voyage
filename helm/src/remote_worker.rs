//! Explicit foreground export of one dedicated managed session. Stored metadata is not a grant.
use super::*;
use anyhow::ensure;
use helm::attachment::{
    client::EnrollmentClient,
    journal::{Journal, RemoteBinding, RemoteReplay, TurnAdmission},
    local_actor::LocalActorStore,
    runtime::{Admission, ManagedSessionOwner, RuntimeClock, SystemClock},
    transport::{self, ConnectionLease},
};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use voyage_protocol::{
    attachment::{Operation, VERSION},
    events::{Feature, Features},
    stream::{DenialCode, Frame, Reply},
};

#[derive(clap::Args)]
pub(super) struct Args {
    /// Dedicated absolute installation directory. Never selects an existing private session.
    #[arg(long)]
    directory: PathBuf,
    #[arg(long, required_unless_present = "recover", conflicts_with = "recover")]
    enrollment_directory: Option<PathBuf>,
    #[arg(long, required_unless_present = "recover", conflicts_with = "recover")]
    origin: Option<String>,
    #[arg(long, conflicts_with = "recover")]
    allow_insecure_loopback: bool,
    /// Recover locally as Interrupted; never reconnect or replay tools.
    #[arg(long)]
    pub recover: bool,
    /// Attest that the exact run's effects have stopped. This is not observed cleanup.
    #[arg(long, requires = "recover")]
    acknowledge_cleanup: Option<uuid::Uuid>,
    /// Append explicit unknown results for unresolved tools, without execution.
    #[arg(long,requires_all=["recover","expected_revision"])]
    reconcile_tools: Option<uuid::Uuid>,
    #[arg(long, requires = "reconcile_tools")]
    expected_revision: Option<u64>,
}
#[derive(Debug)]
struct Authority {
    lease: ConnectionLease,
}
impl helm::policy::ExecutionAuthority for Authority {
    fn check(&self) -> Result<()> {
        ensure!(
            self.lease.is_active(),
            "foreground connection authority unavailable"
        );
        Ok(())
    }
}
fn features() -> Features {
    Features::new(vec![
        Feature::SequencedEvents,
        Feature::Replay,
        Feature::ToolActivity,
        Feature::Usage,
        Feature::ManagedExecution,
    ])
    .expect("fixed features")
}
fn public_reply(mut reply: Reply, redactor: &Redactor) -> Reply {
    match &mut reply {
        Reply::ExecutionSnapshot { session, .. } | Reply::Session { session } => {
            session.name = redactor.redact(&session.name);
            session.model = redactor.redact(&session.model);
        }
        Reply::Sessions { sessions } => {
            for session in sessions {
                session.name = redactor.redact(&session.name);
                session.model = redactor.redact(&session.model);
            }
        }
        _ => {}
    }
    reply
}
/// Caller selected this mode locally. Enrollment proves destination ownership only;
/// fixed supported operations below are the explicit foreground grant.
pub(super) async fn run(args: Args, mut config: Config, workspace: Option<PathBuf>) -> Result<()> {
    let enrollment_directory = args
        .enrollment_directory
        .as_ref()
        .context("enrollment directory required")?;
    let origin = args.origin.as_deref().context("origin required")?;
    ensure!(
        args.directory.is_absolute() && enrollment_directory.is_absolute(),
        "remote directories must be absolute"
    );
    let workspace = config.resolve_workspace(workspace)?;
    ensure!(
        config.provider != helm::ProviderKind::CodexSubscription,
        "remote execution requires a native provider with observable cleanup"
    );
    ensure!(
        config.access_mode() == AccessMode::ReadOnly || config.mcp_servers.is_empty(),
        "remote execution does not support effectful MCP servers without cleanup adapters"
    );
    Policy::new(&config, workspace.clone())?;
    std::time::Instant::now()
        .checked_add(config.timeout())
        .context("configured timeout exceeds clock range")?;
    let local = LocalActorStore::open(&args.directory)?;
    let actor = local.identity()?;
    let first_client = EnrollmentClient::open_existing(
        enrollment_directory,
        origin,
        args.allow_insecure_loopback,
    )?;
    let inspection = first_client.inspection();
    let binding = RemoteBinding {
        origin: first_client.origin().into(),
        machine_id: inspection.machine_id,
        owner_id: inspection.owner_id.context("enrollment is not active")?,
        epoch: inspection.epoch,
        local_installation_id: actor.installation_id,
        local_principal_id: actor.principal_id,
    };
    let directory = args.directory.join("journal");
    let mut journal = Journal::open(directory.clone())?;
    let session = match journal.remote_session(&binding)? {
        Some(id) => id,
        None => {
            let session = Session::new(workspace.clone(), config.model.clone());
            journal.create_remote_session(&session, &binding)?;
            session.id
        }
    };
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session).await?;
    let saved = owner.snapshot().await?;
    ensure!(
        saved.session.workspace.canonicalize()? == workspace,
        "remote workspace differs from dedicated session"
    );
    ensure!(
        saved.session.model == config.model,
        "remote model differs from dedicated session"
    );
    config.workspace = Some(workspace.clone());
    let redactor = redactor(&config);
    let mut first = Some(first_client);
    loop {
        let client = match first.take() {
            Some(client) => client,
            None => EnrollmentClient::open_existing(
                enrollment_directory,
                origin,
                args.allow_insecure_loopback,
            )?,
        };
        let mut connection = tokio::select! {biased;
            _=attachment_interrupt()=>return Ok(()),
            result=transport::connect(client,features(),CancellationToken::new())=>result?,
        };
        ensure!(
            connection.context().machine_id == binding.machine_id
                && connection.context().owner_id == binding.owner_id
                && connection.context().epoch == binding.epoch,
            "remote enrollment generation changed"
        );
        ensure!(
            connection.context().features == features(),
            "remote lifecycle features unavailable"
        );
        let authority = Arc::new(Authority {
            lease: connection.lease(),
        });
        let connected = serde_json::json!({"event":"remote_session_connected","session_id":session,"machine_id":binding.machine_id,"connection_id":connection.context().connection_id});
        write_notice(connected.to_string()).await?;
        let cancel = CancellationToken::new();
        let mut active: Option<tokio::task::JoinHandle<Result<managed::ManagedExecution>>> = None;
        let Reply::ExecutionSnapshot { latest, .. } = owner
            .remote_snapshot(binding.clone(), authority.clone())
            .await?
        else {
            bail!("remote snapshot unavailable")
        };
        let mut cursor = latest.get();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut shutdown = false;
        loop {
            tokio::select! {biased;
                _=attachment_interrupt()=>{shutdown=true;break;},
                result=async {match &mut active {Some(task)=>Some(task.await),None=>std::future::pending().await}}=>{
                    active=None;
                    if !matches!(result,Some(Ok(Ok(ref finished))) if finished.cleanup_observed) {break;}
                },
                frame=connection.receive()=>{
                    let Some(frame)=frame else {break};
                    use helm::policy::ExecutionAuthority;
                    if authority.check().is_err(){break;}
                    let context=connection.context().clone();
                    match frame {
                        Frame::Command{command}=>{
                            let reply=if command.machine_id!=binding.machine_id || command.principal_id!=binding.owner_id || command.connection_id!=context.connection_id || command.version!=VERSION {
                                Reply::Denied{code:DenialCode::Unauthorized}
                            } else if !matches!(&command.operation, Operation::Submit {..} | Operation::Cancel {..}) && SystemClock.now_ms().ok().is_none_or(|now|command.validate(now).is_err()) {
                                Reply::Denied{code:DenialCode::Expired}
                            } else {
                                match &command.operation {
                                    Operation::List{after,limit} if *limit>0=>match owner.remote_snapshot(binding.clone(),authority.clone()).await {
                                        Ok(Reply::ExecutionSnapshot{session:metadata,..})=>Reply::Sessions{sessions:if after.is_none_or(|after|session>after){vec![metadata]}else{vec![]}},
                                        _=>Reply::Denied{code:DenialCode::Internal},
                                    },
                                    Operation::Inspect{session_id} if *session_id==session=>owner.remote_snapshot(binding.clone(),authority.clone()).await.unwrap_or(Reply::Denied{code:DenialCode::Internal}),
                                    Operation::Submit{session_id,expected_revision,prompt} if *session_id==session=>{
                                        let request=TurnAdmission{command_id:command.command_id,machine_id:binding.machine_id,principal_id:binding.owner_id,session_id:session,expected_revision:*expected_revision,expires_at_ms:command.expires_at_ms,prompt:prompt.clone()};
                                        match owner.admit_authorized(request,authority.clone()).await {
                                            Ok(Admission::Existing(run))=>Reply::Run{session_id:run.session_id,run_id:run.id,state:public_state(run.state)},
                                            Ok(Admission::New(mut run))=>{
                                                if active.is_some() || run.register_local_cleanup().await.is_err(){let _=run.fail_before_execution().await;Reply::Denied{code:DenialCode::Internal}}
                                                else {
                                                    let snapshot=owner.remote_snapshot(binding.clone(),authority.clone()).await;
                                                    let owner=owner.clone();let config=config.clone();let workspace=workspace.clone();let authority=authority.clone();let execution_cancel=cancel.child_token();let interrupt=execution_cancel.clone();
                                                    active=Some(tokio::spawn(async move {managed::execute_admitted(&owner,&mut run,&config,workspace,Arc::new(helm::agent::SilentSink),execution_cancel,async move{interrupt.cancelled().await},Some(authority)).await}));
                                                    snapshot.unwrap_or(Reply::Denied{code:DenialCode::Internal})
                                                }
                                            },
                                            Err(_)=>Reply::Denied{code:DenialCode::Conflict},
                                        }
                                    },
                                    Operation::Cancel{session_id,..} if *session_id==session=>match owner.remote_cancel(binding.clone(),command.clone(),authority.clone()).await {Ok(_)=>Reply::Accepted{},Err(_)=>Reply::Denied{code:DenialCode::Conflict}},
                                    _=>Reply::Denied{code:DenialCode::Unauthorized},
                                }
                            };
                            if connection.send(Frame::Result{connection_id:context.connection_id,command_id:command.command_id,reply:public_reply(reply,&redactor)}).is_err(){break;}
                        },
                        Frame::ReplayRequest{request_id,session_id,after,limit,..} if session_id==session=>{
                            let frame=match owner.remote_replay(binding.clone(),after.get(),usize::from(limit),authority.clone()).await {
                                Ok(RemoteReplay::Events{events,latest})=>{Frame::Replay{connection_id:context.connection_id,request_id,session_id,after,latest:voyage_protocol::events::EventCursor::new(latest).map_err(anyhow::Error::msg)?,events}},
                                Ok(RemoteReplay::SnapshotRequired{latest})=>Frame::SnapshotRequired{connection_id:context.connection_id,request_id,session_id,after,latest:voyage_protocol::events::EventCursor::new(latest).map_err(anyhow::Error::msg)?},
                                Err(_)=>break,
                            };
                            if connection.send(frame).is_err(){break;}
                        },
                        _=>break,
                    }
                },
                _=interval.tick()=>{
                    match owner.remote_replay(binding.clone(),cursor,4,authority.clone()).await {
                        Ok(RemoteReplay::Events{events,..})=>{

                            let mut failed=false;
                            for event in events {cursor=event.cursor.get();if connection.send(Frame::Event{connection_id:connection.context().connection_id,session_id:session,event}).is_err(){failed=true;break;}}
                            if failed {break;}
                        },
                        _=>break,
                    }
                }
            }
        }
        cancel.cancel();
        // Never abandon an admitted executor: it owns its bounded cleanup and durable blocker.
        if let Some(task) = active {
            let _ = task.await;
        }
        drop(authority);
        connection.close().await;
        if shutdown {
            return Ok(());
        }
        tokio::select! {_ = attachment_interrupt()=>return Ok(()),_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
}

async fn write_notice(notice: String) -> Result<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("remote-notice".into())
        .spawn(move || {
            use std::io::Write;
            let mut output = std::io::stdout().lock();
            let ok = writeln!(output, "{notice}")
                .and_then(|()| output.flush())
                .is_ok();
            let _ = sender.send(ok);
        })?;
    ensure!(
        matches!(
            tokio::time::timeout(Duration::from_secs(2), receiver).await,
            Ok(Ok(true))
        ),
        "remote output unavailable"
    );
    Ok(())
}

fn public_state(state: helm::attachment::journal::RunState) -> voyage_protocol::stream::RunState {
    use helm::attachment::journal::RunState as Local;
    use voyage_protocol::stream::RunState as Public;
    match state {
        Local::Accepted => Public::Accepted,
        Local::Running => Public::Running,
        Local::Completed => Public::Completed,
        Local::Incomplete => Public::Incomplete,
        Local::Cancelled => Public::Cancelled,
        Local::Failed => Public::Failed,
        Local::Interrupted => Public::Interrupted,
    }
}

pub(super) async fn recover(args: Args) -> Result<()> {
    ensure!(
        args.recover && args.directory.is_absolute(),
        "explicit absolute remote recovery directory required"
    );
    let directory = args.directory.join("journal");
    ensure!(
        args.directory.join("actor.json").is_file() && directory.join("journal.sqlite3").is_file(),
        "existing remote installation required"
    );
    let local = LocalActorStore::open(&args.directory)?;
    let actor = local.identity()?;
    let journal = Journal::open(directory.clone())?;
    let (session, binding) = journal.remote_local_binding(&actor)?;
    drop(journal);
    let owner = ManagedSessionOwner::open(directory, session).await?;
    let recovered = owner.recover_interrupted().await?;
    if let Some(run) = args.acknowledge_cleanup {
        owner
            .attest_remote_cleanup(binding.clone(), run, actor.clone())
            .await?;
    }
    let reconciliation = if let Some(run_id) = args.reconcile_tools {
        Some(
            owner
                .reconcile_remote_tools(
                    binding,
                    helm::attachment::journal::LocalReconcileRequest {
                        session_id: session,
                        run_id,
                        installation_id: actor.installation_id,
                        principal_id: actor.principal_id,
                        expected_revision: args.expected_revision.context("revision required")?,
                    },
                )
                .await?,
        )
    } else {
        None
    };
    write_notice(serde_json::json!({"event":"remote_session_recovered","session_id":session,"run":recovered.map(|run|serde_json::json!({"id":run.id,"state":run.state})),"cleanup":if args.acknowledge_cleanup.is_some(){"operator_attested"}else{"unchanged"},"reconciliation":reconciliation}).to_string()).await
}
