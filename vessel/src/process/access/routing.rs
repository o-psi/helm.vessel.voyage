use super::{Supervisor, store};
use crate::process::{registry, routing};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use uuid::Uuid;
use voyage_protocol::process::*;

impl Supervisor {
    pub(crate) async fn granted(
        &self,
        id: Uuid,
        token: String,
        command: VesselCommand,
    ) -> Result<Value> {
        let grant = store::authenticate(&self.directory, id, &token)?;
        if let Ok(registration) = self.registration(grant.session_id).await {
            ensure!(
                registration.workspace == grant.workspace,
                "session grant workspace mismatch"
            );
        }
        let binding = GrantBinding {
            grant_id: grant.grant_id,
            revision: grant.revision,
            principal_id: grant.principal_id,
        };
        let has = |right| ensure_right(&grant, right);
        match command {
            VesselCommand::FenceAssignment { request } => {
                self.fence_assignment(&grant, request).await
            }
            VesselCommand::Assign { request } => self.assign(&grant, request).await,
            VesselCommand::ObserveAssignment { assignment_id } => {
                self.observe_assignment(&grant, assignment_id, false).await
            }
            VesselCommand::CancelAssignment { assignment_id } => {
                self.observe_assignment(&grant, assignment_id, true).await
            }
            VesselCommand::Capabilities => Ok(
                json!({"protocol":PROCESS_PROTOCOL,"vessel_id":crate::process::identity::public(&self.directory)?.vessel_id,"principal_id":grant.principal_id,"session_id":grant.session_id,"grant_revision":grant.revision,"rights":grant.rights,"expires_at_ms":grant.expires_at_ms,"features":["scoped_catalogue","forward","sse_events","grant_revocation"]}),
            ),
            VesselCommand::Catalogue => {
                has(ProcessRight::Observe)?;
                match self.registration(grant.session_id).await {
                    Ok(registration) => Ok(json!([routing::inspect(
                        &registry::directory(&self.directory, grant.session_id),
                        &registration
                    )
                    .await])),
                    Err(_) => Ok(json!([])),
                }
            }
            VesselCommand::Inspect { session_id } => {
                has(ProcessRight::Observe)?;
                ensure!(session_id == grant.session_id, "session grant denied");
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
                ensure!(session_id == grant.session_id, "session grant denied");
                let right = required_process_right(&command)
                    .ok_or_else(|| anyhow::anyhow!("operation unavailable to scoped clients"))?;
                has(right)?;
                if matches!(
                    command,
                    RuntimeCommand::AssignmentObserve { cancel: true, .. }
                ) {
                    has(ProcessRight::Cancel)?;
                }
                if matches!(command, RuntimeCommand::Terminal { .. }) {
                    has(ProcessRight::Execute)?;
                }
                ensure!(
                    !matches!(command, RuntimeCommand::Stop),
                    "use exact lifecycle stop"
                );
                Ok(serde_json::to_value(
                    self.forward_resuming(session_id, incarnation, command, Some(binding))
                        .await?,
                )?)
            }
            VesselCommand::Start {
                command_id,
                session_id,
                workspace,
            } => {
                has(ProcessRight::Lifecycle)?;
                ensure!(
                    session_id == grant.session_id
                        && std::fs::canonicalize(&workspace)? == grant.workspace,
                    "session creation scope denied"
                );
                self.start(command_id, session_id, workspace, None).await
            }
            VesselCommand::Stop {
                session_id,
                incarnation,
            } => {
                has(ProcessRight::Lifecycle)?;
                ensure!(session_id == grant.session_id, "session grant denied");
                Ok(serde_json::to_value(
                    self.stop(session_id, incarnation, Some(binding)).await?,
                )?)
            }
            VesselCommand::Restart {
                command_id,
                session_id,
                incarnation,
            } => {
                has(ProcessRight::Lifecycle)?;
                ensure!(session_id == grant.session_id, "session grant denied");
                self.restart(command_id, session_id, incarnation).await
            }
            _ => anyhow::bail!("operation requires local account-owner authority"),
        }
    }
}
fn ensure_right(grant: &ProcessGrant, right: ProcessRight) -> Result<()> {
    ensure!(
        grant.rights.contains(&right),
        "session grant permission denied"
    );
    Ok(())
}
