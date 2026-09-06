//! Explicit control-capable presence. No provider or assignment executor is built.
use super::{CliError, EnrollmentClient};
use crate::attachment::{local_actor::LocalActorStore, transport};
use crate::session::SessionStore;
use clap::Args;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::{
    control::{Address, ClientOperation, Machine, Reply},
    events::{Feature, Features},
    stream::Frame,
};
#[derive(Args, Default)]
pub struct ConnectArgs {
    /// Maintain negotiated nomination control leases; no task execution.
    #[arg(long)]
    pub coordination: bool,
    /// Fresh preview only after confirmed absence of the expired prior intent.
    #[arg(long, requires = "session", conflicts_with = "confirm_registration")]
    pub new_registration: bool,
    /// Explicit existing local session reference to register (never exports its history).
    #[arg(long, requires = "coordination")]
    pub session: Option<String>,
    #[arg(long, requires = "session")]
    pub session_directory: Option<PathBuf>,
    /// Stable private local installation identity, independent of enrollment.
    #[arg(long, requires = "session")]
    pub installation_directory: Option<PathBuf>,
    /// Exact metadata preview digest. Omit to preview without network publication.
    #[arg(long,requires_all=["session","registration_id","registration_expires_at_ms"])]
    pub confirm_registration: Option<String>,
    #[arg(long, requires = "session")]
    pub registration_id: Option<Uuid>,
    #[arg(long, requires = "session")]
    pub registration_expires_at_ms: Option<i64>,
}
fn clock() -> Result<i64, CliError> {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CliError::Arguments)?
            .as_millis(),
    )
    .map_err(|_| CliError::Arguments)
}
async fn exchange(
    connection: &mut transport::Connection,
    operation: ClientOperation,
    cancel: &CancellationToken,
) -> Result<Reply, CliError> {
    let request_id = Uuid::new_v4();
    let connection_id = connection.context().connection_id;
    connection.send(Frame::ControlRequest {
        connection_id,
        request_id,
        operation,
    })?;
    tokio::select! {biased;
        _=cancel.cancelled()=>Err(CliError::Cancelled),
        result=tokio::time::timeout(Duration::from_secs(5),connection.receive())=>match result{
            Ok(Some(Frame::ControlResult{request_id:id,connection_id:c,reply})) if id==request_id&&c==connection_id=>Ok(reply),
            _=>Err(CliError::Control),
        }
    }
}
pub(super) async fn run(
    client: EnrollmentClient,
    args: ConnectArgs,
    cancel: CancellationToken,
) -> Result<(), CliError> {
    let mut registration = None;
    let mut source_owner = None;
    let mut observe_only = false;
    let mut client = Some(client);
    let mut connection = None;
    if let Some(reference) = &args.session {
        let store = match args.session_directory {
            Some(path) if path.is_absolute() => SessionStore::new(path),
            Some(_) => return Err(CliError::Arguments),
            None => SessionStore::default(),
        };
        let (owner, session) = tokio::select! {biased;_=cancel.cancelled()=>return Err(CliError::Cancelled),result=store.load_owned(reference)=>result.map_err(|_|CliError::Session)?};
        let directory = args
            .installation_directory
            .unwrap_or_else(crate::config::default_data_dir);
        if !directory.is_absolute() {
            return Err(CliError::Arguments);
        }
        let actor = LocalActorStore::open(&directory)
            .and_then(|store| store.identity())
            .map_err(|_| CliError::Intent)?;
        let intents =
            super::control_intent::Store::open(&directory).map_err(|_| CliError::Intent)?;
        let info = client.as_ref().expect("client available").inspection();
        let owner_id = info.owner_id.ok_or(CliError::Arguments)?;
        let machine = Machine {
            machine_id: info.machine_id,
            epoch: info.epoch,
        };
        let address = Address {
            installation_id: actor.installation_id,
            session_id: session.id,
        };
        let previous = intents.latest(address).map_err(|_| CliError::Intent)?;
        if let Some(old) = &previous
            && (old.origin != client.as_ref().expect("client available").origin()
                || old.owner_id != owner_id)
        {
            return Err(CliError::Intent);
        }
        if args.new_registration {
            let old = previous.as_ref().ok_or(CliError::Arguments)?;
            let mut peer =
                connect(client.take().expect("client available"), cancel.clone()).await?;
            match exchange(&mut peer, old.observe(), &cancel).await {
                Ok(Reply::NotRegistered {
                    command_id,
                    address: observed,
                    ..
                }) if command_id == old.id() && observed == address => connection = Some(peer),
                _ => {
                    peer.close().await;
                    return Err(CliError::Control);
                }
            }
        }
        let intent = if !args.new_registration
            && let Some(original) = previous.as_ref()
        {
            let ClientOperation::Register {
                command_id,
                expires_at_ms,
                ..
            } = original.operation
            else {
                unreachable!()
            };
            if args.registration_id.is_some_and(|id| id != command_id)
                || args
                    .registration_expires_at_ms
                    .is_some_and(|expiry| expiry != expires_at_ms)
            {
                return Err(CliError::Confirmation);
            }
            observe_only = original.machine != machine;
            original.clone()
        } else {
            let operation = ClientOperation::Register {
                command_id: args.registration_id.unwrap_or_else(Uuid::new_v4),
                expires_at_ms: args
                    .registration_expires_at_ms
                    .unwrap_or(clock()?.checked_add(300_000).ok_or(CliError::Arguments)?),
                address,
                source_revision: session.revision,
            };
            operation.validate().map_err(|_| CliError::Arguments)?;
            let origin = if let Some(client) = client.as_ref() {
                client.origin().to_owned()
            } else {
                previous.as_ref().expect("fresh previous").origin.clone()
            };
            let intent = super::control_intent::Intent {
                origin,
                owner_id,
                machine,
                operation,
            };
            intents
                .append(previous.as_ref(), &intent)
                .map_err(|_| CliError::Intent)?;
            intent
        };
        let metadata = serde_json::to_value(&intent).map_err(|_| CliError::Intent)?;
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&metadata).map_err(|_| CliError::Intent)?)
        );
        match args.confirm_registration {
            None => {
                if let Some(peer) = connection.take() {
                    peer.close().await;
                }
                return super::connect::write_notice(serde_json::json!({"status":"confirmation_required","execution_authority":"none","metadata":metadata,"confirmation":digest,"notice":"This private intent preserves the original identifiers/revision across retries. Only this metadata is published; canonical history and local session bytes stay unchanged. Repeat with the displayed registration ID, expiry and confirmation."}).to_string()).await;
            }
            Some(confirm) if confirm == digest => {
                source_owner = Some(owner);
                registration = Some(intent.operation);
            }
            Some(_) => return Err(CliError::Confirmation),
        }
    }
    let mut connection = match connection {
        Some(peer) => peer,
        None => connect(client.take().expect("client available"), cancel.clone()).await?,
    };
    let result=async{
        if let Some(operation)=registration {
            let send=if observe_only {
                let ClientOperation::Register{command_id,expires_at_ms,address,..}=operation else{unreachable!()};
                ClientOperation::ObserveRegistration{command_id,expires_at_ms,address}
            }else{operation.clone()};
            let reply=exchange(&mut connection,send,&cancel).await?;
            if !matches!(&reply,Reply::Registered{..}){
                let ClientOperation::Register{command_id,expires_at_ms,address,..}=operation else{unreachable!()};
                let observation=exchange(&mut connection,ClientOperation::ObserveRegistration{command_id,expires_at_ms,address},&cancel).await?;
                super::connect::write_notice(serde_json::json!({"event":"coordination_registration_observation","reply":observation,"execution_authority":"none"}).to_string()).await?;
                return Err(CliError::Control)
            }
            super::connect::write_notice(serde_json::json!({"event":"coordination_registered","reply":reply,"execution_authority":"none"}).to_string()).await?;
            // Registry publication is inert. It does not transfer the local owner.
            drop(source_owner.take());
        }
        let mut previous=None;
        let mut tick=tokio::time::interval(Duration::from_secs(2));
        loop{
            tokio::select!{biased;_=cancel.cancelled()=>return Err(CliError::Cancelled),_=tick.tick()=>{}}
            let reply=exchange(&mut connection,ClientOperation::Refresh{},&cancel).await?;
            let Reply::Snapshot{views}=&reply else{return Err(CliError::Control)};
            // Emit changes, not every lease renewal; no growing stdout queue.
            let state=serde_json::to_string(&views.iter().map(|v|(&v.record,v.lease.as_ref().map(|l|(l.lease_id,l.server_generation,l.coordinator,l.participant)))).collect::<Vec<_>>()).map_err(|_|CliError::Output)?;
            if previous.as_ref()!=Some(&state){
                super::connect::write_notice(serde_json::json!({"event":"coordination_control","status":"configured","execution_authority":"none","machine_id":connection.context().machine_id,"views":views}).to_string()).await?;previous=Some(state);
            }
        }
    }.await;
    // Best effort release; finite generation-bound leases remain conservative on loss.
    let cleanup = CancellationToken::new();
    let _ = tokio::time::timeout(
        Duration::from_millis(250),
        exchange(&mut connection, ClientOperation::Release {}, &cleanup),
    )
    .await;
    connection.close().await;
    drop(source_owner);
    result
}

async fn connect(
    client: EnrollmentClient,
    cancel: CancellationToken,
) -> Result<transport::Connection, CliError> {
    let features =
        Features::new(vec![Feature::CoordinationControl]).map_err(|_| CliError::Arguments)?;
    let connection = transport::connect(client, features, cancel).await?;
    if !connection
        .context()
        .features
        .contains(Feature::CoordinationControl)
    {
        connection.close().await;
        return Err(CliError::Control);
    }
    Ok(connection)
}
