//! Enrollment relay executes inside the same independent canonical owner.
use super::*;
use crate::attachment::{
    client::EnrollmentClient,
    journal::{RemoteBinding, RemoteGrantObserver, RemoteReplay, TurnAdmission},
    runtime::{Admission, RuntimeClock, SystemClock},
    transport::{self, ConnectionLease},
};
use crate::build::redactor;
use crate::{policy::Policy, tools::Redactor};
use anyhow::bail;
use std::time::Duration;
use voyage_protocol::{
    attachment::{Operation, VERSION},
    events::{Feature, Features},
    process::RuntimeInitialization,
    stream::{DenialCode, Frame, Reply},
};
mod authority;
use authority::{Authority, DispatchAuthority, features, public_reply};
mod initialize;
pub(super) use initialize::initialize;
pub(super) async fn run(state: Arc<State>) -> Result<()> {
    let Some(RuntimeInitialization::Outbound {
        enrollment_directory,
        origin,
        allow_insecure_loopback,
        ..
    }) = &state.registration.initialize
    else {
        bail!("not outbound owner")
    };
    let mut config = state.config.read().await.clone();
    let workspace = state.registration.workspace.clone();
    let launch_policy = Policy::new(&config, workspace.clone())?;
    let first_client =
        EnrollmentClient::open_existing(enrollment_directory, origin, *allow_insecure_loopback)?;
    let inspection = first_client.inspection();
    let binding = RemoteBinding {
        origin: first_client.origin().into(),
        machine_id: inspection.machine_id,
        owner_id: inspection.owner_id.context("enrollment inactive")?,
        epoch: inspection.epoch,
        local_installation_id: state.actor.installation_id,
        local_principal_id: state.actor.principal_id,
    };
    let session = state.registration.session_id;
    let owner = state.owner.clone();
    let grant = Arc::new(owner.remote_grant_observer(binding.clone()).await?);
    grant.check()?;
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
    // Keep signal subscriptions alive while a selected branch awaits storage,
    // output or connection cleanup. Recreating this future each iteration loses
    // signals delivered between the select and its next subscription.
    let interrupt = state.shutdown.cancelled();
    tokio::pin!(interrupt);
    loop {
        if let Err(error) = grant.check() {
            if !authority_store_busy(&error) {
                return Err(error);
            }
            // Stay disconnected until a fresh durable observation succeeds. This
            // retries only an authority read, never admission or a tool effect.
            tokio::select! {_ = &mut interrupt=>return Ok(()),_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
            continue;
        }
        let client = match first.take() {
            Some(client) => client,
            None => EnrollmentClient::open_existing(
                enrollment_directory,
                origin,
                *allow_insecure_loopback,
            )?,
        };
        let mut connection = tokio::select! {biased;
            _=&mut interrupt=>return Ok(()),
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
            consent: grant.clone(),
        });
        let dispatch_authority = Arc::new(DispatchAuthority {
            lease: authority.clone(),
            policy: launch_policy.clone(),
        });
        *state.outbound_status.lock().await = serde_json::json!({"state":"connected","grant":"active","connection_id":connection.context().connection_id});
        let connected = serde_json::json!({"event":"remote_session_connected","session_id":session,"machine_id":binding.machine_id,"connection_id":connection.context().connection_id});
        tracing::info!("{}", connected);
        let cancel = CancellationToken::new();
        let mut active: Option<
            tokio::task::JoinHandle<Result<crate::execution::ManagedExecution>>,
        > = None;
        let Reply::ExecutionSnapshot { latest, .. } = owner
            .remote_snapshot(binding.clone(), authority.clone())
            .await?
        else {
            bail!("remote snapshot unavailable")
        };
        let mut cursor = latest.get();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        let mut shutdown = false;
        let mut cleanup_observed = true;
        loop {
            tokio::select! {biased;
                _=&mut interrupt=>{shutdown=true;break;},
                result=async {match &mut active {Some(task)=>Some(task.await),None=>std::future::pending().await}}=>{
                    active=None;
                    if !matches!(result,Some(Ok(Ok(ref finished))) if finished.cleanup_observed) {cleanup_observed=false;break;}
                },
                frame=connection.receive()=>{
                    let Some(frame)=frame else {break};
                                        use crate::policy::ExecutionAuthority;
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
                                        let request=TurnAdmission{operator_name:None,command_id:command.command_id,machine_id:binding.machine_id,principal_id:binding.owner_id,session_id:session,expected_revision:*expected_revision,expires_at_ms:command.expires_at_ms,prompt:prompt.clone()};
                                        match owner.admit_authorized(request,dispatch_authority.clone()).await {
                                            Ok(Admission::Existing(run))=>Reply::Run{session_id:run.session_id,run_id:run.id,state:public_state(run.state)},
                                            Ok(Admission::New(mut run))=>{
                                                if active.is_some() || run.register_local_cleanup().await.is_err(){let _=run.fail_before_execution().await;Reply::Denied{code:DenialCode::Internal}}
                                                else {
                                                    let snapshot=owner.remote_snapshot(binding.clone(),authority.clone()).await;
                                                    let owner=owner.clone();let config=config.clone();let workspace=workspace.clone();let authority=dispatch_authority.clone();let execution_cancel=cancel.child_token();let interrupt=execution_cancel.clone();
                                                    active=Some(tokio::spawn(async move {crate::execution::execute_admitted(&owner,&mut run,&config,workspace,Arc::new(crate::agent::SilentSink),execution_cancel,async move{interrupt.cancelled().await},Some(authority),None).await}));
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
                        Frame::ReplayRequest{request_id,session_id,after,limit,..}=>{
                            // Observation never falls through to connection-loss cleanup for
                            // a caller-selected session or cursor. Reuse the v2 denial result;
                            // its command_id correlates this replay's request_id.
                            if session_id != session {
                                if connection.send(Frame::Result{connection_id:context.connection_id,command_id:request_id,reply:Reply::Denied{code:DenialCode::Unauthorized}}).is_err(){break;}
                                continue;
                            }
                            let frame=match owner.remote_replay(binding.clone(),after.get(),usize::from(limit),authority.clone()).await {
                                Ok(RemoteReplay::Events{events,latest})=>{Frame::Replay{connection_id:context.connection_id,request_id,session_id,after,latest:voyage_protocol::events::EventCursor::new(latest).map_err(anyhow::Error::msg)?,events}},
                                Ok(RemoteReplay::InvalidCursor)=>Frame::Result{connection_id:context.connection_id,command_id:request_id,reply:Reply::Denied{code:DenialCode::InvalidRequest}},
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
        *state.outbound_status.lock().await =
            serde_json::json!({"state":"disconnecting","grant":"inactive"});
        cancel.cancel();
        // Never abandon an admitted executor: it owns its bounded cleanup and durable blocker.
        if let Some(task) = active {
            cleanup_observed &=
                matches!(task.await, Ok(Ok(ref finished)) if finished.cleanup_observed);
        }
        drop(dispatch_authority);
        drop(authority);
        connection.close().await;
        ensure!(
            cleanup_observed,
            "remote owned-resource cleanup unconfirmed; inspect the dedicated journal and recover locally"
        );
        if shutdown {
            return Ok(());
        }
        tokio::select! {_ = &mut interrupt=>return Ok(()),_ = tokio::time::sleep(Duration::from_secs(1))=>{}}
    }
}

fn authority_store_busy(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<rusqlite::Error>(),
            Some(rusqlite::Error::SqliteFailure(code, _))
                if matches!(code.code, rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
        )
    })
}

fn public_state(state: crate::attachment::journal::RunState) -> voyage_protocol::stream::RunState {
    use crate::attachment::journal::RunState as Local;
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
