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
    meter: Option<Arc<crate::provider::goal_meter::GoalMeter>>,
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
        if self.meter.is_some() {
            ensure!(
                response.result["features"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "execution_budget")),
                "participant does not support bounded execution; upgrade before delegating Goal work"
            );
        }
        Ok(credential)
    }
    async fn record_observation(&self, observation: AssignmentObservation) -> Result<()> {
        let base = observation.execution_usage.clone().or(observation
            .result
            .as_ref()
            .and_then(|r| r.get("execution_usage"))
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()?);
        if let Some(usage) = base {
            ensure!(
                usage.budget.command_id == observation.assignment_id
                    && usage.session_id == observation.child_session_id
                    && Some(usage.run_id) == observation.run_id,
                "participant usage observation identity mismatch"
            );
            let observed = observation.execution_usage_observed.clone().or(observation
                .result
                .as_ref()
                .and_then(|r| r.get("execution_usage_observed"))
                .filter(|v| !v.is_null())
                .map(|v| serde_json::from_value(v.clone()))
                .transpose()?);
            if let Some(meter) = &self.meter {
                meter
                    .settle_allocation(observation.participant_vessel_id, usage)
                    .await?;
            } else {
                self.owner
                    .reconcile_goal_allocation(observation.participant_vessel_id, usage, observed)
                    .await?;
            }
        }
        if observation.admission_closed {
            ensure!(
                observation.run_id.is_none()
                    && observation.cleanup_observed
                    && observation.state == "cancelled",
                "invalid participant non-admission proof"
            );
            let original = self
                .owner
                .assignment_request(observation.parent_run_id, observation.assignment_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("unknown fenced assignment"))?;
            if original.budget.is_some() {
                let proof = serde_json::json!({"status":"not_admitted","command_id":observation.assignment_id});
                if let Some(meter) = &self.meter {
                    meter
                        .close_allocation(
                            observation.participant_vessel_id,
                            observation.assignment_id,
                            proof,
                        )
                        .await?;
                } else {
                    self.owner
                        .close_goal_allocation(
                            observation.participant_vessel_id,
                            observation.assignment_id,
                            Some(proof),
                        )
                        .await?;
                }
            }
        }
        self.owner.update_assignment(observation).await?;
        Ok(())
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
        self.record_observation(observation.clone()).await?;
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
        meter: None,
    };
    let prior = parent
        .owner
        .assignment_result(run_id, assignment_id)
        .await?;
    if prior["cleanup_observed"] == true {
        parent
            .record_observation(serde_json::from_value(prior.clone())?)
            .await?;
        return Ok(prior);
    }
    Ok(serde_json::to_value(
        parent.observe(&endpoint, assignment_id, cancel).await?,
    )?)
}
