use super::{Supervisor, store};
use crate::process::{registry, routing};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use uuid::Uuid;
use voyage_protocol::process::*;
use voyage_protocol::vessel::{VESSEL_API_VERSION, VoyageCommand};

impl Supervisor {
    pub(crate) async fn granted(
        &self,
        id: Uuid,
        token: String,
        expected_vessel_id: Option<Uuid>,
        command: VesselCommand,
    ) -> Result<Value> {
        let identity = crate::process::identity::public(&self.directory)?.vessel_id;
        if let Some(expected) = expected_vessel_id {
            ensure!(expected == identity, "Vessel identity changed");
        }
        if store::connection_path(&self.directory, id).exists() {
            ensure!(
                expected_vessel_id == Some(identity),
                "workspace access requires pinned Vessel identity"
            );
            return self.connected(id, &token, command).await;
        }
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
                json!({"protocol":VESSEL_API_VERSION,"version":env!("CARGO_PKG_VERSION"),"vessel_id":crate::process::identity::public(&self.directory)?.vessel_id,"principal_id":grant.principal_id,"scope":"session","session_id":grant.session_id,"grant_revision":grant.revision,"rights":grant.rights,"expires_at_ms":grant.expires_at_ms,"features":["scoped_catalogue","voyage_operations","sse_events","grant_revocation","start_resolution","provider_accounts","account_start","private_account_enrollment"]}),
            ),
            command @ (VesselCommand::Accounts { .. } | VesselCommand::AccountDefaults { .. } | VesselCommand::AccountModels { .. } | VesselCommand::StartAccount { .. } | VesselCommand::ResolveStartAccount { .. } | VesselCommand::EnrollAccount { .. } | VesselCommand::CancelAccountEnrollment { .. } | VesselCommand::PrivateAccountEnrollment { .. }) => self.host_accounts(command, crate::process::accounts::Scope::Session(grant.clone())).await,
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
            VesselCommand::Voyage(request) => {
                ensure!(
                    request.session_id == grant.session_id,
                    "session grant denied"
                );
                let right = super::super::api::required_right(&request.command)
                    .ok_or_else(|| anyhow::anyhow!("operation unavailable to scoped clients"))?;
                has(right)?;
                if matches!(
                    request.command,
                    VoyageCommand::AssignmentObserve { cancel: true, .. }
                ) {
                    has(ProcessRight::Cancel)?;
                }
                if matches!(request.command, VoyageCommand::Terminal { .. }) {
                    has(ProcessRight::Execute)?;
                }
                self.voyage(request, Some(binding)).await
            }
            VesselCommand::ResolveStart {
                command_id,
                session_id,
                workspace,
                config_path,
            } => {
                has(ProcessRight::Lifecycle)?;
                ensure!(
                    config_path.is_none()
                        && session_id == grant.session_id
                        && workspace == grant.workspace
                        && std::fs::canonicalize(&workspace)? == grant.workspace,
                    "session creation scope denied"
                );
                self.resolve_start(command_id, session_id, workspace, None)
                    .await
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
                super::super::api::reply(self.stop(session_id, incarnation, Some(binding)).await?)
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
