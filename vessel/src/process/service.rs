use super::{registry, routing};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{
    net::UnixListener,
    sync::{Mutex, Semaphore},
};
use uuid::Uuid;
use voyage_protocol::process::*;

pub(super) struct Supervisor {
    pub(super) directory: PathBuf,
    pub(super) binary: PathBuf,
    pub(super) assignment_locks: Mutex<HashMap<Uuid, Arc<Mutex<()>>>>,
    pub(super) lifecycle_locks: Mutex<HashMap<Uuid, Arc<Mutex<()>>>>,
    pub(super) registrations: Mutex<HashMap<Uuid, ProcessRegistration>>,
}

pub async fn serve(directory: PathBuf, binary: PathBuf) -> Result<()> {
    registry::private_directory(&directory)?;
    let _lock = registry::lock(&directory)?;
    super::identity::public(&directory)?;
    let sessions = directory.join("sessions");
    registry::private_directory(&sessions)?;
    let mut registrations = HashMap::new();
    for entry in std::fs::read_dir(&sessions)? {
        ensure!(
            registrations.len() < 4096,
            "supervisor registration retention limit reached"
        );
        let path = entry?.path();
        if path.join("registration.json").exists() {
            registry::private_directory(&path)?;
            let registration = registry::load(&path.join("registration.json"))?;
            ensure!(
                registration.protocol == PROCESS_PROTOCOL
                    && path == registry::directory(&directory, registration.session_id),
                "invalid runtime registration identity"
            );
            registrations.insert(registration.session_id, registration);
        }
    }
    let endpoint = directory.join("vessel.sock");
    if endpoint.exists() {
        std::fs::remove_file(&endpoint)?;
    }
    let listener = UnixListener::bind(&endpoint)?;
    registry::secure_socket(&endpoint)?;
    let supervisor = Arc::new(Supervisor {
        directory,
        binary,
        registrations: Mutex::new(registrations),
        assignment_locks: Mutex::new(HashMap::new()),
        lifecycle_locks: Mutex::new(HashMap::new()),
    });
    let connections = Arc::new(Semaphore::new(64));
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        let (mut stream, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = terminate.recv() => break,
            _ = tokio::signal::ctrl_c() => break,
        };
        if stream.peer_cred()?.uid() != unsafe { libc::geteuid() } {
            continue;
        }
        let Ok(permit) = connections.clone().try_acquire_owned() else {
            continue;
        };
        let supervisor = supervisor.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let result = tokio::time::timeout(
                Duration::from_secs(5),
                read_frame::<VesselRequest>(&mut stream),
            )
            .await;
            let response = match result {
                Ok(Ok(request)) if request.protocol == PROCESS_PROTOCOL => tokio::time::timeout(
                    Duration::from_secs(25),
                    supervisor.handle(request.command),
                )
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow::anyhow!("supervisor request deadline exceeded")
                        .context(routing::OutcomeUnknown))
                }),
                _ => Err(anyhow::anyhow!(
                    "invalid frame or unsupported process protocol"
                )),
            };
            let response = match response {
                Ok(result) => VesselResponse {
                    protocol: PROCESS_PROTOCOL,
                    result,
                    error: None,
                    outcome_unknown: false,
                },
                Err(error) => VesselResponse {
                    protocol: PROCESS_PROTOCOL,
                    result: Value::Null,
                    outcome_unknown: error.downcast_ref::<routing::OutcomeUnknown>().is_some(),
                    error: Some(error.to_string()),
                },
            };
            let _ =
                tokio::time::timeout(Duration::from_secs(5), write_frame(&mut stream, &response))
                    .await;
        });
    }
    // Runtime processes own their lifetimes; service shutdown only detaches routing.
    std::fs::remove_file(endpoint)?;
    Ok(())
}

impl Supervisor {
    pub(super) async fn handle(&self, command: VesselCommand) -> Result<Value> {
        match command {
            VesselCommand::Wake { session_id } => self.wake(session_id).await,
            command @ (VesselCommand::AcceptParticipant { .. }
            | VesselCommand::RemoveParticipant { .. }) => self.participant_admin(command).await,
            VesselCommand::Assign { .. }
            | VesselCommand::FenceAssignment { .. }
            | VesselCommand::ObserveAssignment { .. }
            | VesselCommand::CancelAssignment { .. } => {
                anyhow::bail!("participant operations require an explicit scoped execution grant")
            }
            VesselCommand::Identity => Ok(serde_json::to_value(super::identity::public(
                &self.directory,
            )?)?),
            VesselCommand::TrustVessel { identity } => {
                let _serial = self.registrations.lock().await;
                super::identity::pin(&self.directory, &identity)?;
                Ok(serde_json::json!({"trusted":identity.vessel_id}))
            }
            command @ (VesselCommand::PrepareTransfer { .. }
            | VesselCommand::ExportTransfer { .. }
            | VesselCommand::AcceptTransfer { .. }
            | VesselCommand::TransferChunk { .. }
            | VesselCommand::UploadTransferChunk { .. }
            | VesselCommand::ActivateTransfer { .. }) => self.transfer(command).await,
            command @ VesselCommand::Import { .. } => self.initialize_import(command).await,
            command @ VesselCommand::Branch { .. } => self.branch(command).await,
            command @ VesselCommand::Grant { .. } => self.grant(command).await,
            command @ VesselCommand::RevokeGrant { .. } => self.revoke_grant(command).await,
            VesselCommand::Granted {
                grant_id,
                token,
                command,
            } => self.granted(grant_id, token, *command).await,
            command @ VesselCommand::StartOutbound { .. } => self.start_outbound(command).await,
            command @ VesselCommand::ManagedImport { .. } => self.initialize_managed(command).await,
            VesselCommand::Capabilities => Ok(
                json!({"protocol":PROCESS_PROTOCOL,"platform":std::env::consts::OS,"features":["catalogue","start","start_configured","inspect","forward","stop","restart","explicit_recovery","durable_receipts","history_paging","events","decisions","lifecycle","branch","ordinary_import","managed_import","outbound_adapter","scoped_grants","revocation","participant_bindings","participant_assignments","signed_owner_transfer"],"max_frame_bytes":MAX_PROCESS_FRAME,"capacity":null,"max_connections":64}),
            ),
            VesselCommand::Catalogue => {
                let registrations: Vec<_> = self
                    .registrations
                    .lock()
                    .await
                    .values()
                    .filter(|registration| {
                        !matches!(
                            registration.initialize,
                            Some(RuntimeInitialization::Participant { .. })
                        )
                    })
                    .cloned()
                    .collect();
                let mut tasks = tokio::task::JoinSet::new();
                for registration in registrations {
                    let directory = registry::directory(&self.directory, registration.session_id);
                    tasks.spawn(async move { routing::inspect(&directory, &registration).await });
                }
                let mut entries = Vec::new();
                while let Some(result) = tasks.join_next().await {
                    entries.push(result?);
                }
                entries.sort_by_key(|entry| entry.session_id);
                Ok(serde_json::to_value(entries)?)
            }
            VesselCommand::Start {
                command_id,
                session_id,
                workspace,
            } => self.start(command_id, session_id, workspace, None).await,
            VesselCommand::StartConfigured {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                self.start(command_id, session_id, workspace, Some(config_path))
                    .await
            }
            command @ VesselCommand::Recover { .. } => self.recover(command).await,
            VesselCommand::Restart {
                command_id,
                session_id,
                incarnation,
            } => self.restart(command_id, session_id, incarnation).await,
            VesselCommand::Inspect { session_id } => {
                let registration = self.registration(session_id).await?;
                Ok(serde_json::to_value(
                    routing::inspect(
                        &registry::directory(&self.directory, session_id),
                        &registration,
                    )
                    .await,
                )?)
            }
            VesselCommand::Forward {
                session_id,
                incarnation,
                command,
            } => {
                ensure!(
                    !matches!(command, RuntimeCommand::Stop),
                    "use Vessel stop for lifecycle tracking"
                );
                Ok(serde_json::to_value(
                    self.forward_resuming(session_id, incarnation, command, None)
                        .await?,
                )?)
            }
            VesselCommand::Stop {
                session_id,
                incarnation,
            } => Ok(serde_json::to_value(
                self.stop(session_id, incarnation, None).await?,
            )?),
        }
    }

    pub(super) async fn registration(&self, session_id: Uuid) -> Result<ProcessRegistration> {
        self.registrations
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown session"))
    }
}
