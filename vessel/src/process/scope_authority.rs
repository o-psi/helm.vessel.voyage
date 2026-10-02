//! Read-only local authority checks for independent bound Voyages. No executor,
//! provider credentials, control key or supervisor token crosses this boundary.
use super::{
    access::{execution_epoch, store},
    accounts::Scope,
    database,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    os::linux::net::SocketAddrExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
};
use uuid::Uuid;
use voyage_protocol::{execution_scope::*, process::*};
use voyage_storage::protected_linux::RootDirectory;

const MAX_FRAME: usize = 16_384;
const DEADLINE: Duration = Duration::from_secs(2);
#[cfg(test)]
#[path = "scope_authority_tests.rs"]
mod tests;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Lease {
    schema: u32,
    session: Uuid,
    grant: GrantBinding,
    epoch: String,
    handle: ExecutionScopeHandle,
}

pub(super) fn socket_name(root: &Path) -> String {
    let mut hash = Sha256::new();
    hash.update(b"voyage/scope-authority-socket/v1\0");
    hash.update(root.as_os_str().as_encoded_bytes());
    format!("voyage-scope-{:x}", hash.finalize())
}

fn guardian_socket_name(root: &Path, session: Uuid, incarnation: Uuid) -> String {
    let mut hash = Sha256::new();
    hash.update(b"voyage/guardian-scope-socket/v1\0");
    hash.update(root.as_os_str().as_encoded_bytes());
    hash.update(session.as_bytes());
    hash.update(incarnation.as_bytes());
    format!("voyage-scope-{:x}", hash.finalize())
}

fn current_grant(
    root: &Path,
    registration: &ProcessRegistration,
    binding: &GrantBinding,
) -> Result<ProcessGrant> {
    let grant: ProcessGrant = store::load(&store::grant_path(root, binding.grant_id))?;
    ensure!(
        grant.grant_id == binding.grant_id
            && grant.principal_id == binding.principal_id
            && grant.revision == binding.revision
            && grant.session_id == registration.session_id
            && grant.workspace == registration.workspace,
        "runtime scope identity refused"
    );
    let right = *grant
        .rights
        .first()
        .context("runtime scope has no rights")?;
    Scope::Session(grant.clone()).check(root, &registration.workspace, right)?;
    Ok(grant)
}

pub(super) async fn mint(
    root: &Path,
    registration: &ProcessRegistration,
    binding: &GrantBinding,
) -> Result<ExecutionScopeHandle> {
    mint_for(root, registration, binding, false).await
}

/// One-shot suspended observation has no live guardian and remains tied to the
/// current supervisor. It cannot create or resume an execution owner.
pub(super) async fn mint_observer(
    root: &Path,
    registration: &ProcessRegistration,
    binding: &GrantBinding,
) -> Result<ExecutionScopeHandle> {
    mint_for(root, registration, binding, true).await
}

fn lease_identity(
    registration: &ProcessRegistration,
    binding: &GrantBinding,
    epoch: &str,
    socket: &str,
    observer: bool,
) -> Result<Uuid> {
    let mut hash = Sha256::new();
    hash.update(b"voyage/runtime-grant-lease/v2\0");
    hash.update(serde_json::to_vec(&(
        registration.session_id,
        registration.incarnation,
        binding.grant_id,
        binding.principal_id,
        binding.revision,
        epoch,
        socket,
        observer,
    ))?);
    let bytes: [u8; 32] = hash.finalize().into();
    Ok(Uuid::from_bytes(bytes[..16].try_into()?))
}

async fn mint_for(
    root: &Path,
    registration: &ProcessRegistration,
    binding: &GrantBinding,
    observer: bool,
) -> Result<ExecutionScopeHandle> {
    ensure!(
        registration
            .peer_uids
            .as_ref()
            .is_some_and(|peer| peer.supervisor == 0),
        "scope lease requires protected runtime"
    );
    let identity = database::bound_observer_identity(root, registration).await?;
    super::launch::validate_identity(&identity)?;
    current_grant(root, registration, binding)?;
    let epoch = execution_epoch::current(root, registration.session_id)?
        .context("protected execution epoch unavailable")?;
    let socket = if observer {
        socket_name(root)
    } else {
        guardian_socket_name(root, registration.session_id, registration.incarnation)
    };
    let id = lease_identity(registration, binding, &epoch, &socket, observer)?;
    let directory = RootDirectory::open(root)?
        .create_child("access".as_ref())?
        .create_child("runtime-scope-leases".as_ref())?;
    let name = format!("{id}.json");
    let lease = match directory.read(name.as_ref(), 4096) {
        Ok(bytes) => serde_json::from_slice::<Lease>(&bytes)?,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            ensure!(
                std::fs::read_dir(root.join("access/runtime-scope-leases"))?
                    .take(16_385)
                    .count()
                    < 16_384,
                "runtime scope lease capacity reached"
            );
            let lease = Lease {
                schema: 1,
                session: registration.session_id,
                grant: binding.clone(),
                epoch: epoch.clone(),
                handle: ExecutionScopeHandle {
                    socket_name: socket.clone(),
                    lease_id: id,
                    secret: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
                },
            };
            directory.publish_new(name.as_ref(), &serde_json::to_vec(&lease)?, 4096)?;
            lease
        }
        Err(error) => return Err(error),
    };
    ensure!(
        lease.schema == 1
            && lease.session == registration.session_id
            && lease.grant.grant_id == binding.grant_id
            && lease.grant.principal_id == binding.principal_id
            && lease.grant.revision == binding.revision
            && lease.epoch == epoch
            && lease.handle.lease_id == id
            && lease.handle.socket_name == socket
            && lease.handle.secret.len() == 64,
        "retained runtime scope lease changed"
    );
    current_grant(root, registration, binding)?;
    Ok(lease.handle)
}

async fn scope(root: &Path, peer: u32, request: ScopeCheck) -> Result<RuntimeScope> {
    use subtle::ConstantTimeEq;
    ensure!(
        request.schema == 1
            && !request.session_id.is_nil()
            && !request.incarnation.is_nil()
            && request.token.len() == 64
            && request.handle.secret.len() == 64
            && (request.handle.socket_name == socket_name(root)
                || request.handle.socket_name
                    == guardian_socket_name(root, request.session_id, request.incarnation)),
        "runtime scope request refused"
    );
    let registration = database::registration(root, request.session_id).await?;
    ensure!(
        registration.incarnation == request.incarnation
            && registration
                .peer_uids
                .as_ref()
                .is_some_and(|p| p.supervisor == 0 && p.runtime == peer)
            && bool::from(
                registration
                    .token
                    .as_bytes()
                    .ct_eq(request.token.as_bytes())
            ),
        "runtime scope peer refused"
    );
    let identity = database::bound_observer_identity(root, &registration).await?;
    super::launch::validate_identity(&identity)?;
    let directory = RootDirectory::open(root)?
        .child("access".as_ref())?
        .child("runtime-scope-leases".as_ref())?;
    let lease: Lease = serde_json::from_slice(
        &directory.read(format!("{}.json", request.handle.lease_id).as_ref(), 4096)?,
    )?;
    ensure!(
        lease.schema == 1
            && lease.session == request.session_id
            && lease.handle.lease_id == request.handle.lease_id
            && lease.handle.socket_name == request.handle.socket_name
            && bool::from(
                lease
                    .handle
                    .secret
                    .as_bytes()
                    .ct_eq(request.handle.secret.as_bytes())
            ),
        "runtime scope lease refused"
    );
    let epoch = execution_epoch::current(root, request.session_id)?
        .context("execution epoch unavailable")?;
    ensure!(lease.epoch == epoch, "runtime execution authority changed");
    let grant = current_grant(root, &registration, &lease.grant)?;
    Ok(RuntimeScope {
        schema: 1,
        session_id: registration.session_id,
        incarnation: registration.incarnation,
        binding: lease.grant,
        full_access: grant.full_access,
        rights: grant.rights,
        accounts: grant.accounts,
        enrollment_connections: grant.enrollment_connections,
        expires_at_ms: grant.expires_at_ms,
        workspace: grant.workspace,
        connection_binding: grant.connection_binding,
    })
}

#[cfg(test)]
async fn connection(root: PathBuf, stream: UnixStream, peer: u32) -> Result<()> {
    connection_for(root, stream, peer, None).await
}

async fn connection_for(
    root: PathBuf,
    mut stream: UnixStream,
    peer: u32,
    owner: Option<(Uuid, Uuid)>,
) -> Result<()> {
    let response = tokio::time::timeout(DEADLINE, async {
        let length = stream.read_u32().await? as usize;
        ensure!((1..=MAX_FRAME).contains(&length), "scope frame refused");
        let mut bytes = vec![0; length];
        stream.read_exact(&mut bytes).await?;
        let request: ScopeCheck = serde_json::from_slice(&bytes)?;
        ensure!(
            owner.is_none_or(|(session, incarnation)| request.session_id == session
                && request.incarnation == incarnation),
            "guardian scope owner refused"
        );
        scope(&root, peer, request).await
    })
    .await;
    // Failure reasons, credential material and parser diagnostics never enter
    // the reply or logs. The executing runtime receives only current metadata.
    let response = ScopeReply {
        scope: match response {
            Ok(Ok(scope)) => Some(scope),
            _ => None,
        },
    };
    let bytes = serde_json::to_vec(&response)?;
    ensure!(bytes.len() <= MAX_FRAME, "scope reply bound exceeded");
    tokio::time::timeout(DEADLINE, async {
        stream.write_u32(bytes.len() as u32).await?;
        stream.write_all(&bytes).await?;
        stream.shutdown().await
    })
    .await??;
    Ok(())
}

pub(super) struct Service {
    task: JoinHandle<Result<()>>,
}
impl Drop for Service {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Service {
    pub(super) async fn wait(&mut self) -> Result<()> {
        match (&mut self.task).await {
            Ok(Ok(())) => anyhow::bail!("runtime scope authority service stopped"),
            Ok(Err(error)) => Err(error),
            Err(_) => anyhow::bail!("runtime scope authority service unavailable"),
        }
    }
}

pub(super) async fn wait(service: &mut Option<Service>) -> Result<()> {
    if let Some(service) = service {
        service.wait().await
    } else {
        std::future::pending::<Result<()>>().await
    }
}

pub(super) fn start(root: &Path) -> Result<Option<Service>> {
    if !super::runtime_storage::has_bound_layout(root) {
        return Ok(None);
    }
    RootDirectory::open(root)?;
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "root authority service required"
    );
    start_listener(root, socket_name(root), None).map(Some)
}

fn start_listener(root: &Path, name: String, owner: Option<(Uuid, Uuid)>) -> Result<Service> {
    RootDirectory::open(root)?;
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "root authority service required"
    );
    let address = std::os::unix::net::SocketAddr::from_abstract_name(name.as_bytes())?;
    let listener = UnixListener::bind_addr(&address.into())?;
    let root = root.to_owned();
    Ok(Service {
        task: tokio::spawn(async move {
            let capacity = Arc::new(Semaphore::new(32));
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                accepted=listener.accept()=>{
                    let (stream,_)=accepted?;let peer=stream.peer_cred()?.uid();
                    while tasks.try_join_next().is_some() {}
                    if tasks.len() >= 32 {continue;}
                        let Ok(permit)=capacity.clone().try_acquire_owned() else{continue;};
                        let root=root.clone();tasks.spawn(async move {let _permit=permit;let _=connection_for(root,stream,peer,owner).await;});
                    },
                    _=tasks.join_next(),if !tasks.is_empty()=>{},
                }
            }
        }),
    })
}

/// A guardian owns this metadata-only listener until its actual child tree has
/// retired. Supervisor replacement never owns or cancels this authority service.
pub(super) struct GuardianService {
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for GuardianService {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub(super) fn start_guardian(
    root: &Path,
    registration: &ProcessRegistration,
) -> Result<GuardianService> {
    RootDirectory::open(root)?;
    ensure!(
        unsafe { libc::geteuid() } == 0
            && registration
                .peer_uids
                .as_ref()
                .is_some_and(|p| p.supervisor == 0),
        "protected guardian authority required"
    );
    let root = root.to_owned();
    let owner = (registration.session_id, registration.incarnation);
    let name = guardian_socket_name(&root, owner.0, owner.1);
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let thread = std::thread::Builder::new()
        .name("guardian-scope".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(_) => {
                    let _ = ready_tx.send(false);
                    return;
                }
            };
            runtime.block_on(async move {
                let mut service = match start_listener(&root, name, Some(owner)) {
                    Ok(service) => service,
                    Err(_) => {
                        let _ = ready_tx.send(false);
                        return;
                    }
                };
                if ready_tx.send(true).is_err() {
                    return;
                }
                tokio::select! { _ = stopped => {}, _ = service.wait() => {} }
            });
            runtime.shutdown_timeout(DEADLINE);
        })?;
    let service = GuardianService {
        stop: Some(stop),
        thread: Some(thread),
    };
    ensure!(
        ready_rx.recv_timeout(DEADLINE).unwrap_or(false),
        "guardian scope listener unavailable"
    );
    Ok(service)
}

#[cfg(test)]
#[path = "scope_authority_ordinary_tests.rs"]
mod ordinary_family_tests;
