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
    pub(super) capacity: usize,
    pub(super) registrations: Mutex<HashMap<Uuid, ProcessRegistration>>,
}

pub async fn serve(directory: PathBuf, binary: PathBuf, capacity: usize) -> Result<()> {
    ensure!(
        (1..=256).contains(&capacity),
        "capacity must be between 1 and 256"
    );
    registry::private_directory(&directory)?;
    let _lock = registry::lock(&directory)?;
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
        capacity,
        registrations: Mutex::new(registrations),
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
    async fn handle(&self, command: VesselCommand) -> Result<Value> {
        match command {
            VesselCommand::Capabilities => Ok(
                json!({"protocol":PROCESS_PROTOCOL,"platform":std::env::consts::OS,"features":["catalogue","start","inspect","forward","stop","restart"],"max_frame_bytes":MAX_PROCESS_FRAME,"capacity":self.capacity,"max_connections":64}),
            ),
            VesselCommand::Catalogue => {
                let registrations: Vec<_> =
                    self.registrations.lock().await.values().cloned().collect();
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
            } => self.start(command_id, session_id, workspace).await,
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
                let registration = self.registration(session_id).await?;
                ensure!(
                    registration.incarnation == incarnation,
                    "stale runtime incarnation"
                );
                Ok(serde_json::to_value(
                    routing::forward(
                        &registry::directory(&self.directory, session_id),
                        &registration,
                        command,
                    )
                    .await?,
                )?)
            }
            VesselCommand::Stop {
                session_id,
                incarnation,
            } => {
                let mut registrations = self.registrations.lock().await;
                let mut registration = registrations
                    .get(&session_id)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("unknown session"))?;
                ensure!(
                    registration.incarnation == incarnation,
                    "stale runtime incarnation"
                );
                let directory = registry::directory(&self.directory, session_id);
                registration.state = ProcessState::CleanupUnconfirmed;
                registry::save(&directory, &registration)?;
                registrations.insert(session_id, registration.clone());
                drop(registrations);
                let response =
                    routing::forward(&directory, &registration, RuntimeCommand::Stop).await?;
                // A stop receipt alone is not proof of descendant cleanup.
                Ok(serde_json::to_value(response)?)
            }
        }
    }

    async fn registration(&self, session_id: Uuid) -> Result<ProcessRegistration> {
        self.registrations
            .lock()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("unknown session"))
    }

    async fn start(&self, command_id: Uuid, session_id: Uuid, workspace: PathBuf) -> Result<Value> {
        ensure!(
            !command_id.is_nil() && !session_id.is_nil(),
            "session and command IDs must be nonnil"
        );
        let command = VesselCommand::Start {
            command_id,
            session_id,
            workspace: workspace.clone(),
        };
        let workspace = std::fs::canonicalize(workspace)?;
        ensure!(workspace.is_dir(), "workspace must be a directory");
        let mut registrations = self.registrations.lock().await;
        if registry::command_record(&self.directory, command_id, &command, false)? {
            let previous = registrations.get(&session_id).ok_or_else(|| {
                anyhow::anyhow!("start outcome unconfirmed; retained admission prevents replay")
                    .context(routing::OutcomeUnknown)
            })?;
            return Ok(serde_json::to_value(
                routing::inspect(&registry::directory(&self.directory, session_id), previous).await,
            )?);
        }
        if let Some(previous) = registrations
            .values()
            .find(|item| item.command_id == command_id)
        {
            ensure!(
                previous.session_id == session_id && previous.workspace == workspace,
                "start command id payload conflict"
            );
        }
        if let Some(previous) = registrations.get(&session_id) {
            ensure!(
                previous.workspace == workspace,
                "session workspace conflict"
            );
            registry::command_record(&self.directory, command_id, &command, true)
                .map_err(|error| error.context(routing::OutcomeUnknown))?;
            return Ok(serde_json::to_value(
                routing::inspect(&registry::directory(&self.directory, session_id), previous).await,
            )?);
        }
        ensure!(
            registrations.len() < 4096,
            "supervisor registration retention limit reached"
        );
        for registration in registrations.values_mut() {
            let directory = registry::directory(&self.directory, registration.session_id);
            if !directory.join("runtime.sock").exists()
                && super::recovery::clean_stop(&directory, registration)
            {
                registration.state = ProcessState::Stopped;
            }
        }
        // Unavailable owners retain capacity: neither a timeout nor a PID proves cleanup.
        ensure!(
            registrations
                .values()
                .filter(|item| item.state != ProcessState::Stopped)
                .count()
                < self.capacity,
            "Vessel process capacity exhausted"
        );
        let directory = registry::directory(&self.directory, session_id);
        registry::private_directory(&directory)?;
        let registration = ProcessRegistration {
            protocol: PROCESS_PROTOCOL,
            session_id,
            incarnation: Uuid::new_v4(),
            command_id,
            restart_from: None,
            token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            workspace,
            state: ProcessState::Starting,
        };
        registry::command_record(&self.directory, command_id, &command, true)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registry::save(&directory, &registration)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        registrations.insert(session_id, registration.clone());
        drop(registrations);
        super::launch::launch(&self.binary, &directory, &registration)
            .map_err(|error| error.context(routing::OutcomeUnknown))?;
        let observed = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let info = routing::inspect(&directory, &registration).await;
                if info.state == ProcessState::Live {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await;
        if let Ok(info) = observed {
            return Ok(serde_json::to_value(info)?);
        }
        Err(anyhow::anyhow!(
            "runtime startup unconfirmed; retained registration prevents duplicate launch"
        )
        .context(routing::OutcomeUnknown))
    }
}
