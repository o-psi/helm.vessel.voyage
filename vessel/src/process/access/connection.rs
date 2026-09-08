//! Workspace routing derives session authority without changing local policy.
use super::{Supervisor, store};
use crate::process::{registry, routing};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use voyage_protocol::process::*;
use voyage_protocol::vessel::{VESSEL_API_VERSION, VoyageCommand};

impl Supervisor {
    pub(super) async fn connected(
        &self,
        id: Uuid,
        token: &str,
        command: VesselCommand,
    ) -> Result<Value> {
        let grant = store::authenticate_connection(&self.directory, id, token)?;
        let has = |right| -> Result<()> {
            ensure!(grant.rights.contains(&right), "workspace permission denied");
            Ok(())
        };
        let operation = command.clone();
        match command {
            VesselCommand::Capabilities => Ok(json!({
                "protocol": VESSEL_API_VERSION, "version": env!("CARGO_PKG_VERSION"),
                "vessel_id": grant.vessel_id, "principal_id": grant.principal_id,
                "scope": "workspaces", "grant_revision": grant.revision,
                "rights": grant.rights, "expires_at_ms": grant.expires_at_ms,
                "workspaces": grant.workspaces,
                "features": ["workspace_pairing", "sse_events", "scoped_catalogue", "voyage_operations", "grant_revocation","start_resolution"]
            })),
            VesselCommand::Catalogue => {
                has(ProcessRight::Catalogue)?;
                let registrations: Vec<_> = self
                    .registrations
                    .lock()
                    .await
                    .values()
                    .filter(|r| {
                        grant.workspaces.iter().any(|w| w.path == r.workspace)
                            && !matches!(
                                r.initialize,
                                Some(RuntimeInitialization::Participant { .. })
                            )
                    })
                    .cloned()
                    .collect();
                let mut entries = Vec::with_capacity(registrations.len());
                let mut tasks = tokio::task::JoinSet::new();
                for registration in registrations {
                    ordinary(&registration)?;
                    approved(&grant, &registration.workspace)?;
                    let directory = registry::directory(&self.directory, registration.session_id);
                    tasks.spawn(async move { routing::inspect(&directory, &registration).await });
                    if tasks.len() >= 16 {
                        if let Some(result) = tasks.join_next().await {
                            entries.push(result?);
                        }
                    }
                }
                while let Some(result) = tasks.join_next().await {
                    entries.push(result?);
                }
                entries.sort_by_key(|entry| entry.session_id);
                store::current_connection(
                    &self.directory,
                    &store::load(&store::connection_path(&self.directory, id))?,
                )?;
                Ok(serde_json::to_value(entries)?)
            }
            VesselCommand::ResolveStart {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                has(ProcessRight::Create)?;
                ensure!(config_path.is_none(), "host configuration denied");
                approved(&grant, &workspace)?;
                if let Ok(existing) = self.registration(session_id).await {
                    ordinary(&existing)?;
                    ensure!(
                        existing.workspace == workspace,
                        "session workspace conflict"
                    );
                }
                // Resolve binds the original Start, never a second lifecycle payload.
                let original = VesselCommand::Start {
                    command_id,
                    session_id,
                    workspace: workspace.clone(),
                };
                self.bind_connection_operation(&grant, command_id, &original)
                    .await?;
                self.resolve_start(command_id, session_id, workspace, None)
                    .await
            }
            VesselCommand::Start {
                command_id,
                session_id,
                workspace,
            } => {
                has(ProcessRight::Create)?;
                ensure!(
                    !session_id.is_nil() && !command_id.is_nil(),
                    "nil creation identity"
                );
                let canonical = std::fs::canonicalize(&workspace)?;
                ensure!(
                    workspace == canonical,
                    "use the exact approved workspace path"
                );
                approved(&grant, &canonical)?;
                if let Ok(existing) = self.registration(session_id).await {
                    ordinary(&existing)?;
                    ensure!(
                        existing.workspace == canonical,
                        "session workspace conflict"
                    );
                }
                self.bind_connection_operation(&grant, command_id, &operation)
                    .await?;
                self.connection_session(&grant, session_id, &canonical)?;
                // Dispatch the path that was authorized, not a caller-supplied
                // symlink that could resolve differently after the scope check.
                // Retain the exact original request for command deduplication.
                let original = VesselCommand::Start {
                    command_id,
                    session_id,
                    workspace,
                };
                self.start_initialized(command_id, session_id, canonical, None, None, original)
                    .await
            }
            VesselCommand::Inspect { session_id } => {
                has(ProcessRight::Observe)?;
                let registration = self.registration(session_id).await?;
                ordinary(&registration)?;
                approved(&grant, &registration.workspace)?;
                let info = routing::inspect(
                    &registry::directory(&self.directory, session_id),
                    &registration,
                )
                .await;
                store::current_connection(&self.directory, &grant)?;
                Ok(serde_json::to_value(info)?)
            }
            VesselCommand::Voyage(request) => {
                let right = crate::process::api::required_right(&request.command)
                    .ok_or_else(|| anyhow::anyhow!("operation unavailable to workspace clients"))?;
                has(right)?;
                if matches!(request.command, VoyageCommand::Terminal { .. }) {
                    has(ProcessRight::Execute)?;
                }
                if matches!(
                    request.command,
                    VoyageCommand::AssignmentObserve { cancel: true, .. }
                ) {
                    has(ProcessRight::Cancel)?;
                }
                let registration = self.registration(request.session_id).await?;
                ordinary(&registration)?;
                let binding =
                    self.connection_session(&grant, request.session_id, &registration.workspace)?;
                self.voyage(request, Some(binding)).await
            }
            VesselCommand::Stop {
                session_id,
                incarnation,
            } => {
                has(ProcessRight::Lifecycle)?;
                let registration = self.registration(session_id).await?;
                ordinary(&registration)?;
                let binding =
                    self.connection_session(&grant, session_id, &registration.workspace)?;
                crate::process::api::reply(self.stop(session_id, incarnation, Some(binding)).await?)
            }
            VesselCommand::Restart {
                command_id,
                session_id,
                incarnation,
            } => {
                has(ProcessRight::Lifecycle)?;
                let registration = self.registration(session_id).await?;
                ordinary(&registration)?;
                self.connection_session(&grant, session_id, &registration.workspace)?;
                self.bind_connection_operation(&grant, command_id, &operation)
                    .await?;
                self.restart(command_id, session_id, incarnation).await
            }
            _ => anyhow::bail!("operation requires local account-owner authority"),
        }
    }

    // The public start payload has no bearer. Retain its exact principal/grant
    // binding separately, before effects, so replacement credentials cannot replay it.
    async fn bind_connection_operation(
        &self,
        grant: &ConnectionGrant,
        id: Uuid,
        command: &VesselCommand,
    ) -> Result<()> {
        let _serial = self.registrations.lock().await;
        store::current_connection(&self.directory, grant)?;
        let directory = self.directory.join("access/connection-commands");
        registry::private_directory(&directory)?;
        let path = directory.join(format!("{id}.json"));
        let intent = json!({"schema_version": 1, "command_id": id,
            "vessel_id": grant.vessel_id, "grant_id": grant.grant_id,
            "principal_id": grant.principal_id, "grant_revision": grant.revision,
            "command": command});
        if path.try_exists()? {
            let prior: Value = store::load(&path)?;
            ensure!(
                prior == intent,
                "connection command identity or payload conflict"
            );
        } else {
            ensure!(
                std::fs::read_dir(&directory)?.take(65536).count() < 65536,
                "connection command capacity exhausted"
            );
            store::save(&path, &intent)?;
        }
        Ok(())
    }

    fn connection_session(
        &self,
        grant: &ConnectionGrant,
        session_id: Uuid,
        workspace: &std::path::Path,
    ) -> Result<GrantBinding> {
        approved(grant, workspace)?;
        store::current_connection(&self.directory, grant)?;
        let mut digest = Sha256::new();
        digest.update(b"voyage/connection-session/v1\0");
        digest.update(grant.grant_id.as_bytes());
        digest.update(session_id.as_bytes());
        let bytes: [u8; 32] = digest.finalize().into();
        let id = Uuid::from_bytes(bytes[..16].try_into()?);
        let binding = GrantBinding {
            grant_id: id,
            revision: grant.revision,
            principal_id: grant.principal_id,
        };
        let path = store::grant_path(&self.directory, id);
        store::initialize(&self.directory)?;
        let rights: Vec<_> = grant
            .rights
            .iter()
            .copied()
            .filter(|r| !matches!(r, ProcessRight::Catalogue | ProcessRight::Create))
            .collect();
        if path.exists() {
            let previous: ProcessGrant = store::load(&path)?;
            ensure!(
                previous.grant_id == id
                    && previous.principal_id == grant.principal_id
                    && previous.session_id == session_id
                    && previous.workspace == workspace
                    && previous.revision == grant.revision
                    && !previous.revoked
                    && previous.expires_at_ms == grant.expires_at_ms
                    && previous.rights == rights
                    && previous.parent_grant.is_none()
                    && previous.participant_binding.is_none()
                    && previous
                        .connection_binding
                        .as_ref()
                        .is_some_and(|b| b.grant_id == grant.grant_id
                            && b.revision == grant.revision
                            && b.principal_id == grant.principal_id),
                "derived session authority conflict"
            );
        } else {
            ensure!(
                std::fs::read_dir(path.parent().expect("grant directory"))?
                    .take(16384)
                    .count()
                    < 16384,
                "session authority capacity exhausted"
            );
            store::save(
                &path,
                &ProcessGrant {
                    grant_id: id,
                    principal_id: grant.principal_id,
                    session_id,
                    workspace: workspace.to_owned(),
                    revision: grant.revision,
                    rights,
                    expires_at_ms: grant.expires_at_ms,
                    revoked: false,
                    enrollment: None,
                    token_hash: String::new(),
                    parent_grant: None,
                    participant_binding: None,
                    connection_binding: Some(GrantBinding {
                        grant_id: grant.grant_id,
                        revision: grant.revision,
                        principal_id: grant.principal_id,
                    }),
                },
            )?;
        }
        Ok(binding)
    }
}

fn approved(grant: &ConnectionGrant, workspace: &std::path::Path) -> Result<()> {
    ensure!(
        workspace.is_absolute()
            && workspace.is_dir()
            && std::fs::canonicalize(workspace)? == workspace
            && grant.workspaces.iter().any(|w| w.path == workspace),
        "workspace scope denied"
    );
    Ok(())
}

fn ordinary(registration: &ProcessRegistration) -> Result<()> {
    ensure!(
        !matches!(
            registration.initialize,
            Some(RuntimeInitialization::Participant { .. })
        ),
        "participant sessions require their accepted participant authority"
    );
    Ok(())
}
