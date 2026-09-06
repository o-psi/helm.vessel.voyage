//! Parent-authoritative participant assignments, with durable unknown-outcome obligations.
mod monitor;
mod tool;
mod transport;
use crate::attachment::runtime::ManagedSessionOwner;
use anyhow::{Result, ensure};
use std::sync::Arc;
pub use tool::ParticipantTool;
use uuid::Uuid;
use voyage_protocol::process::*;

#[derive(Clone)]
struct Parent {
    owner: ManagedSessionOwner,
    run_id: Uuid,
    principal_id: Uuid,
    vessel_id: Uuid,
    endpoints: Vec<ParticipantEndpoint>,
}
impl Parent {
    fn endpoint(&self, name: &str) -> Result<&ParticipantEndpoint> {
        self.endpoints
            .iter()
            .find(|entry| entry.name == name)
            .ok_or_else(|| anyhow::anyhow!("participant is not locally configured"))
    }
    async fn connected(&self, endpoint: &ParticipantEndpoint) -> Result<AccessCredential> {
        let credential = transport::credential(&endpoint.credential_file)?;
        ensure!(
            credential.session_id == self.owner.session_id(),
            "participant credential parent scope mismatch"
        );
        let response = transport::request(&credential, VesselCommand::Capabilities).await?;
        ensure!(
            response.error.is_none()
                && response.result["vessel_id"].as_str()
                    == Some(endpoint.participant_vessel_id.to_string().as_str()),
            "participant identity mismatch"
        );
        Ok(credential)
    }
    async fn observe(
        &self,
        endpoint: &ParticipantEndpoint,
        id: Uuid,
        cancel: bool,
    ) -> Result<AssignmentObservation> {
        let credential = self.connected(endpoint).await?;
        let response = transport::request(
            &credential,
            if cancel {
                VesselCommand::FenceAssignment {
                    request: self
                        .owner
                        .assignment_request(self.run_id, id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("unknown canonical assignment"))?,
                }
            } else {
                VesselCommand::ObserveAssignment { assignment_id: id }
            },
        )
        .await?;
        ensure!(
            response.error.is_none(),
            "participant observation denied or unavailable"
        );
        let observation: AssignmentObservation = serde_json::from_value(response.result)?;
        ensure!(
            observation.assignment_id == id
                && observation.parent_session_id == self.owner.session_id()
                && observation.parent_run_id == self.run_id
                && observation.participant_vessel_id == endpoint.participant_vessel_id,
            "participant result attribution mismatch"
        );
        self.owner.update_assignment(observation.clone()).await?;
        Ok(observation)
    }
}

/// Reconcile retained canonical obligations even when no new run can be admitted.
pub async fn reconcile(
    owner: ManagedSessionOwner,
    run_id: Uuid,
    assignment_id: Uuid,
    participant: &str,
    cancel: bool,
    config: &crate::Config,
) -> Result<serde_json::Value> {
    let request = owner
        .assignment_request(run_id, assignment_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("unknown parent assignment"))?;
    let endpoint = config
        .participants
        .iter()
        .find(|entry| entry.name == participant)
        .ok_or_else(|| anyhow::anyhow!("participant is not locally configured"))?
        .clone();
    ensure!(
        endpoint.binding_id == request.binding_id,
        "assignment participant binding mismatch"
    );
    let parent = Parent {
        owner,
        run_id,
        principal_id: Uuid::nil(),
        vessel_id: request.parent_vessel_id,
        endpoints: vec![endpoint.clone()],
    };
    let prior = parent
        .owner
        .assignment_result(run_id, assignment_id)
        .await?;
    if prior["cleanup_observed"] == true {
        return Ok(prior);
    }
    Ok(serde_json::to_value(
        parent.observe(&endpoint, assignment_id, cancel).await?,
    )?)
}
