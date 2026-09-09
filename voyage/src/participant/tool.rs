use super::*;
use crate::{
    config::AccessMode,
    model::ToolDefinition,
    tools::{Tool, ToolContext, ToolError},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

pub struct ParticipantTool {
    parent: Arc<Parent>,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Args {
    Submit {
        participant: String,
        task: String,
        #[serde(default)]
        context: Vec<DisclosedMessage>,
        #[serde(default)]
        assignment_id: Option<Uuid>,
    },
    Observe {
        participant: String,
        assignment_id: Uuid,
    },
    Cancel {
        participant: String,
        assignment_id: Uuid,
    },
    List,
}
impl ParticipantTool {
    pub async fn configured(
        owner: ManagedSessionOwner,
        run_id: Uuid,
        principal_id: Uuid,
        config: &crate::Config,
    ) -> Result<Option<Arc<dyn Tool>>> {
        if config.participants.is_empty() {
            return Ok(None);
        }
        ensure!(
            config.participants.len() <= 16,
            "participant endpoint limit exceeded"
        );
        for (i, endpoint) in config.participants.iter().enumerate() {
            ensure!(
                !endpoint.name.is_empty()
                    && endpoint.name.len() <= 64
                    && endpoint.credential_file.is_absolute()
                    && !endpoint.participant_vessel_id.is_nil()
                    && !endpoint.binding_id.is_nil()
                    && endpoint.binding_revision > 0
                    && !config.participants[..i]
                        .iter()
                        .any(|prior| prior.name == endpoint.name),
                "invalid participant configuration"
            );
        }
        let resources = crate::build::resource_root();
        let root = resources
            .parent()
            .and_then(std::path::Path::parent)
            .and_then(std::path::Path::parent)
            .ok_or_else(|| anyhow::anyhow!("participant owner requires supervised runtime"))?;
        let identity: VesselIdentity =
            serde_json::from_slice(&std::fs::read(root.join("identity/public.json"))?)?;
        Ok(Some(Arc::new(Self {
            parent: Arc::new(Parent {
                owner,
                run_id,
                principal_id,
                vessel_id: identity.vessel_id,
                endpoints: config.participants.clone(),
            }),
        })))
    }
    async fn submit(
        &self,
        participant: String,
        task: String,
        disclosed: Vec<DisclosedMessage>,
        id: Option<Uuid>,
        context: &ToolContext,
    ) -> Result<Value> {
        context.policy.check_current()?;
        ensure!(
            !context.cancellation.is_cancelled(),
            "parent run cancellation requested"
        );
        ensure!(
            context.policy.access_mode() != AccessMode::ReadOnly,
            "read-only parent policy does not delegate participant effects"
        );
        let endpoint = self.parent.endpoint(&participant)?.clone();
        if context.policy.access_mode() == AccessMode::Approval {
            let request = context.approval(
                "participant_assignment",
                participant.clone(),
                format!(
                    "Delegate one bounded task and {} selected context messages: {}",
                    disclosed.len(),
                    task
                ),
            );
            context
                .approver
                .approve(&request)
                .await
                .require_approved()?;
            context.policy.check_current()?;
        }
        let credential = self.parent.connected(&endpoint).await?;
        let now: u64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
            .try_into()?;
        let rules = context.policy.effective().rules();
        let mut request = AssignmentRequest {
            assignment_id: id.unwrap_or_else(Uuid::new_v4),
            binding_id: endpoint.binding_id,
            binding_revision: endpoint.binding_revision,
            parent_vessel_id: self.parent.vessel_id,
            parent_session_id: self.parent.owner.session_id(),
            parent_run_id: self.parent.run_id,
            expires_at_ms: now.saturating_add(300000),
            task,
            context: disclosed,
            policy: ParticipantPolicy {
                access: serde_json::to_value(context.policy.access_mode())?
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("invalid parent policy"))?
                    .into(),
                deny_commands: rules.deny_commands.clone(),
                inherit_env: rules.inherit_env.clone(),
                github_enabled: rules.github_enabled,
                timeout_secs: context.timeout.as_secs(),
                max_output_bytes: context.max_output_bytes,
                max_subagents: 1,
            },
        };
        if let Some(prior) = self
            .parent
            .owner
            .assignment_request(self.parent.run_id, request.assignment_id)
            .await?
        {
            request.expires_at_ms = prior.expires_at_ms;
            ensure!(
                serde_json::to_vec(&prior)? == serde_json::to_vec(&request)?,
                "assignment ID payload conflict"
            );
        }
        let recorded = self
            .parent
            .owner
            .record_assignment(
                self.parent.principal_id,
                endpoint.participant_vessel_id,
                request.clone(),
            )
            .await?;
        if recorded["duplicate"] == true {
            let prior = self
                .parent
                .owner
                .assignment_result(self.parent.run_id, request.assignment_id)
                .await?;
            if prior["cleanup_observed"] == true {
                return Ok(prior);
            }
        }
        // Start the cleanup observer before network admission so dropping this tool
        // future on parent cancellation cannot orphan the cancellation obligation.
        super::monitor::spawn(
            self.parent.clone(),
            endpoint.clone(),
            request.assignment_id,
            context.cancellation.clone(),
        );
        // The canonical parent obligation is durable before the participant can admit effects.
        let delivered = transport::request(
            &credential,
            VesselCommand::Assign {
                request: request.clone(),
            },
        )
        .await;
        let observation = match delivered {
            Ok(response) if response.error.is_none() => {
                serde_json::from_value::<AssignmentObservation>(response.result)?
            }
            Ok(response) if !response.outcome_unknown && recorded["duplicate"] != true => {
                AssignmentObservation {
                    assignment_id: request.assignment_id,
                    participant_vessel_id: endpoint.participant_vessel_id,
                    parent_session_id: request.parent_session_id,
                    parent_run_id: request.parent_run_id,
                    child_session_id: request.assignment_id,
                    child_incarnation: None,
                    run_id: None,
                    state: "rejected".into(),
                    cleanup_observed: true,
                    result: Some(json!({"reason":response.error.unwrap_or_default()})),
                }
            }
            _ => {
                return Ok(
                    json!({"assignment_id":request.assignment_id,"state":"acceptance_unknown","cleanup_observed":false,"next":"observe this exact assignment; do not allocate a replacement"}),
                );
            }
        };
        ensure!(
            observation.assignment_id == request.assignment_id
                && observation.participant_vessel_id == endpoint.participant_vessel_id
                && observation.parent_session_id == request.parent_session_id
                && observation.parent_run_id == request.parent_run_id
                && observation.child_session_id == request.assignment_id,
            "participant acceptance attribution mismatch"
        );
        self.parent
            .owner
            .update_assignment(observation.clone())
            .await?;
        Ok(serde_json::to_value(observation)?)
    }
}
#[async_trait]
impl Tool for ParticipantTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition{output_schema:None,annotations:None,name:"participant".into(),description:"Delegate bounded work only to locally configured participant Vessels. Canonical parent obligations survive unknown admission. Observe the exact assignment ID; never replace unknown work with a new ID. Context is explicitly selected public conversation text, never secrets or runtime instructions. Cancellation targets one assignment and is not proof of cleanup.".into(),input_schema:json!({"type":"object","properties":{"action":{"enum":["submit","observe","cancel","list"]},"participant":{"type":"string"},"task":{"type":"string"},"assignment_id":{"type":"string"},"context":{"type":"array","items":{"type":"object","properties":{"role":{"enum":["user","assistant","tool"]},"content":{"type":"string"}},"required":["role","content"],"additionalProperties":false}}},"required":["action"],"additionalProperties":false})}
    }
    async fn execute(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> std::result::Result<String, ToolError> {
        let args: Args = serde_json::from_value(arguments)
            .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
        context
            .policy
            .check_current()
            .map_err(|error| ToolError::Failed(error.to_string()))?;
        let result = match args {
            Args::Submit {
                participant,
                task,
                context: disclosed,
                assignment_id,
            } => {
                self.submit(participant, task, disclosed, assignment_id, context)
                    .await
            }
            Args::Observe {
                participant,
                assignment_id,
            } => match self.parent.endpoint(&participant) {
                Ok(endpoint) => self
                    .parent
                    .observe(endpoint, assignment_id, false)
                    .await
                    .and_then(|value| serde_json::to_value(value).map_err(Into::into)),
                Err(error) => Err(error),
            },
            Args::Cancel {
                participant,
                assignment_id,
            } => match self.parent.endpoint(&participant) {
                Ok(endpoint) => self
                    .parent
                    .observe(endpoint, assignment_id, true)
                    .await
                    .and_then(|value| serde_json::to_value(value).map_err(Into::into)),
                Err(error) => Err(error),
            },
            Args::List => {
                self.parent
                    .owner
                    .assignment_observations(self.parent.run_id)
                    .await
            }
        }
        .map_err(|error| ToolError::Failed(error.to_string()))?;
        Ok(crate::tools::truncate(
            serde_json::to_vec(&result).map_err(|error| ToolError::Failed(error.to_string()))?,
            context.max_output_bytes,
        ))
    }
}
