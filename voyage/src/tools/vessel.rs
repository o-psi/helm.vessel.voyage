//! Native model-facing coordination over the public Vessel protocol.
//! The trusted host supplies routes; the model never supplies credentials or URLs.
mod journal;
mod transport;

use super::{Tool, ToolContext, ToolError};
use crate::{config::AccessMode, model::ToolDefinition};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;
use voyage_protocol::vessel::{VesselCommand, VoyageCommand, VoyageReply, VoyageRequest};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VesselSettings {
    pub enabled: bool,
    /// Used outside a supervised voyage. No service is started implicitly.
    pub local_directory: Option<PathBuf>,
    /// Operator-configured aliases to private AccessCredential files.
    pub remotes: BTreeMap<String, PathBuf>,
}
impl Default for VesselSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            local_directory: None,
            remotes: BTreeMap::new(),
        }
    }
}
impl VesselSettings {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.remotes.len() <= 32,
            "at most 32 Vessel remote routes are supported"
        );
        if let Some(path) = &self.local_directory {
            anyhow::ensure!(
                path.is_absolute(),
                "Vessel local_directory must be absolute"
            );
        }
        for (name, path) in &self.remotes {
            anyhow::ensure!(
                !name.is_empty()
                    && name.len() <= 64
                    && name != "local"
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
                "Vessel route names must be 1..64 ASCII alphanumeric, hyphen or underscore; local is reserved"
            );
            anyhow::ensure!(
                path.is_absolute(),
                "Vessel access file paths must be absolute"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct VesselContext {
    pub session_id: Uuid,
    pub directory: PathBuf,
}
#[derive(Clone)]
pub struct VesselTool {
    settings: VesselSettings,
    context: Option<VesselContext>,
}
impl VesselTool {
    pub fn new(settings: VesselSettings, context: Option<VesselContext>) -> Self {
        Self { settings, context }
    }
    fn local(&self) -> Result<&Path, ToolError> {
        self.context
            .as_ref()
            .map(|c| c.directory.as_path())
            .or(self.settings.local_directory.as_deref())
            .ok_or_else(|| failed("local Vessel discovery is not configured"))
    }
}
fn invalid(message: &str) -> ToolError {
    ToolError::InvalidArguments(message.into())
}
fn failed(message: &str) -> ToolError {
    ToolError::Failed(message.into())
}
fn limit() -> u32 {
    50
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ControlSection {
    Models,
    Policy,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Action {
    Routes,
    Capabilities,
    Controls {
        session_id: Uuid,
        run_id: Option<Uuid>,
        section: ControlSection,
    },
    Operations {
        #[serde(default)]
        offset: usize,
        #[serde(default = "limit")]
        limit: u32,
    },
    List {
        #[serde(default)]
        offset: usize,
        #[serde(default = "limit")]
        limit: u32,
    },
    Search {
        query: String,
        #[serde(default)]
        offset: usize,
        #[serde(default = "limit")]
        limit: u32,
    },
    Inspect {
        session_id: Uuid,
    },
    History {
        session_id: Uuid,
        #[serde(default)]
        offset: u64,
        #[serde(default = "limit")]
        limit: u32,
        expected_revision: Option<u64>,
    },
    Follow {
        session_id: Uuid,
        #[serde(default)]
        after: u64,
        #[serde(default = "limit")]
        limit: u32,
        #[serde(default)]
        wait_ms: u32,
    },
    Wait {
        session_id: Uuid,
        #[serde(default)]
        after: u64,
        #[serde(default = "limit")]
        limit: u32,
        #[serde(default)]
        wait_ms: u32,
    },
    Receipt {
        session_id: Option<Uuid>,
        command_id: Uuid,
    },
    Create {
        command_id: Uuid,
        session_id: Uuid,
        workspace: PathBuf,
        config_path: Option<PathBuf>,
        task: String,
    },
    Submit {
        session_id: Uuid,
        command_id: Uuid,
        expected_revision: u64,
        prompt: String,
    },
    Steer {
        session_id: Uuid,
        incarnation: Uuid,
        run_id: Uuid,
        command_id: Uuid,
        expected_revision: u64,
        prompt: String,
    },
    Cancel {
        session_id: Uuid,
        incarnation: Uuid,
        run_id: Uuid,
        command_id: Uuid,
        expected_revision: u64,
    },
    Rename {
        session_id: Uuid,
        command_id: Uuid,
        expected_revision: u64,
        name: String,
    },
    Archive {
        session_id: Uuid,
        command_id: Uuid,
        expected_revision: u64,
    },
    Restore {
        session_id: Uuid,
        command_id: Uuid,
        expected_revision: u64,
    },
}
impl Action {
    fn mutation_id(&self) -> Option<Uuid> {
        match self {
            Self::Create { command_id, .. }
            | Self::Submit { command_id, .. }
            | Self::Steer { command_id, .. }
            | Self::Cancel { command_id, .. }
            | Self::Rename { command_id, .. }
            | Self::Archive { command_id, .. }
            | Self::Restore { command_id, .. } => Some(*command_id),
            _ => None,
        }
    }
    fn executes(&self) -> bool {
        matches!(
            self,
            Self::Create { .. } | Self::Submit { .. } | Self::Steer { .. }
        )
    }
}

fn action_schemas() -> Vec<Value> {
    let variants: &[(&str, &[&str])] = &[
        ("routes", &[]),
        ("operations", &[]),
        ("capabilities", &[]),
        ("controls", &["session_id", "section"]),
        ("list", &[]),
        ("search", &["query"]),
        ("inspect", &["session_id"]),
        ("history", &["session_id"]),
        ("follow", &["session_id"]),
        ("wait", &["session_id"]),
        ("receipt", &["command_id"]),
        ("create", &["command_id", "session_id", "workspace", "task"]),
        (
            "submit",
            &["session_id", "command_id", "expected_revision", "prompt"],
        ),
        (
            "steer",
            &[
                "session_id",
                "incarnation",
                "run_id",
                "command_id",
                "expected_revision",
                "prompt",
            ],
        ),
        (
            "cancel",
            &[
                "session_id",
                "incarnation",
                "run_id",
                "command_id",
                "expected_revision",
            ],
        ),
        (
            "rename",
            &["session_id", "command_id", "expected_revision", "name"],
        ),
        (
            "archive",
            &["session_id", "command_id", "expected_revision"],
        ),
        (
            "restore",
            &["session_id", "command_id", "expected_revision"],
        ),
    ];
    variants
        .iter()
        .map(|(action, fields)| {
            let required: Vec<_> = std::iter::once("action")
                .chain(fields.iter().copied())
                .collect();
            json!({"properties":{"action":{"const":action}},"required":required})
        })
        .collect()
}

#[async_trait]
impl Tool for VesselTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "vessel".into(),
            description: "Coordinate any voyage authorized by configured Vessel routes, not just related voyages. Public HTTP only; no credentials, terminal access, approval responses, shell, or provider configuration exposed. Ordinary tool policy approvals still apply. routes identifies the owning voyage and configured target aliases. inspect returns registration and current snapshot; list/search page catalogue metadata (search is not full-text history). history pages canonical conversation. follow/wait read bounded events after a cursor; a timeout is not completion. Mutations require a stable caller-generated command_id; reuse it only for the identical request. Durable intents precede effects; unknown outcomes are never replayed. operations pages durable local intent IDs; receipt with session_id queries the server; without it reads the local journal. Create starts a session then submits required initial task; its start command ID is session_id. Use a fresh session ID. Target defaults to local; remote grants enforce their actual rights. No implicit startup, recovery, deletion, or authority broadening.".into(),
            input_schema: json!({"type":"object","additionalProperties":false,"required":["action"],"properties":{
                "action":{"type":"string","enum":["routes","operations","controls","capabilities","list","search","inspect","history","follow","wait","receipt","create","submit","steer","cancel","rename","archive","restore"]},
                "target":{"type":"string","description":"Trusted configured route alias; default local"},
                "session_id":{"type":"string","format":"uuid"},"command_id":{"type":"string","format":"uuid"},"incarnation":{"type":"string","format":"uuid"},"run_id":{"type":"string","format":"uuid"},
                "expected_revision":{"type":"integer","minimum":0},"offset":{"type":"integer","minimum":0},"after":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":128,"default":50},"wait_ms":{"type":"integer","minimum":0,"maximum":30000,"default":0},
                "section":{"type":"string","enum":["models","policy"]},"query":{"type":"string"},"workspace":{"type":"string"},"config_path":{"type":"string"},"task":{"type":"string","description":"Required initial task for create"},"prompt":{"type":"string"},"name":{"type":"string","minLength":1,"maxLength":256}
            },"oneOf": action_schemas()})
        }
    }
    async fn execute(
        &self,
        mut arguments: Value,
        context: &ToolContext,
    ) -> Result<String, ToolError> {
        if !self.settings.enabled {
            return Err(ToolError::Denied("Vessel coordination is disabled".into()));
        }
        let object = arguments
            .as_object_mut()
            .ok_or_else(|| invalid("expected an object"))?;
        let target = match object.remove("target") {
            None => "local".to_owned(),
            Some(Value::String(s)) => s,
            _ => return Err(invalid("target must be a configured alias")),
        };
        let action: Action = serde_json::from_value(arguments.clone())
            .map_err(|e| invalid(&format!("invalid Vessel action arguments: {e}")))?;
        self.settings
            .validate()
            .map_err(|_| failed("invalid trusted Vessel route settings"))?;
        check(context)?;
        let mutation = action.mutation_id();
        if mutation.is_some() {
            if context.policy.access_mode() == AccessMode::ReadOnly {
                return Err(ToolError::Denied(
                    "Vessel mutations are disabled in read-only mode".into(),
                ));
            }
            if action.executes() && context.policy.ceiling_present() {
                return Err(ToolError::Denied("public Vessel execution cannot propagate the current policy ceiling; use policy-bounded subagents instead".into()));
            }
            if context.policy.access_mode() == AccessMode::Approval {
                context.approver.approve(&context.approval("vessel.mutate", &target, "Coordinate an independent voyage through Vessel; target execution retains its own policy".into())).await.require_approved()?;
                check(context)?;
            }
        }
        if matches!(action, Action::Routes) {
            return output(
                json!({"self_session_id":self.context.as_ref().map(|c|c.session_id),"local_available":self.local().is_ok(),"targets":std::iter::once("local".to_owned()).chain(self.settings.remotes.keys().filter(|k| k.as_str() != "local").cloned()).collect::<Vec<_>>(),"transport":"public_http","remote_scope":"configured grant rights","automatic_start":false}),
                context,
            );
        }
        let access = if target == "local" {
            None
        } else {
            Some(
                self.settings
                    .remotes
                    .get(&target)
                    .ok_or_else(|| invalid("unknown configured Vessel target"))?
                    .as_path(),
            )
        };
        // Remote-only reads do not require a local service directory.
        let local = self.local().unwrap_or(Path::new(""));
        if matches!(
            action,
            Action::Receipt {
                session_id: None,
                ..
            } | Action::Operations { .. }
        ) {
            let owner = self
                .context
                .as_ref()
                .ok_or_else(|| failed("journal reads require an owning voyage context"))?;
            let root = journal::existing_root(local, owner.session_id)?;
            let result = match &action {
                Action::Receipt { command_id, .. } => journal::receipt(&root, *command_id)?,
                Action::Operations { offset, limit } => {
                    journal::operations(&root, *offset, page(*limit)?)?
                }
                _ => unreachable!(),
            };
            return output(result, context);
        }
        let mut transport = transport::Transport::open(local, access)?;
        // Validate delegation before persisting intent or starting a process.
        if let Action::Create {
            workspace,
            config_path,
            task,
            command_id,
            session_id,
        } = &action
        {
            if !workspace.is_absolute() || config_path.as_ref().is_some_and(|p| !p.is_absolute()) {
                return Err(invalid(
                    "create workspace and config_path must be absolute target-host paths",
                ));
            }
            if command_id == session_id {
                return Err(invalid("create command_id and session_id must differ"));
            }
            text(task, 64 * 1024)?;
            if target == "local" {
                context
                    .policy
                    .check_delegated_workspace(workspace)
                    .map_err(|_| {
                        ToolError::Denied(
                            "creation workspace is outside delegated read/write roots".into(),
                        )
                    })?;
                if let Some(path) = config_path {
                    context.policy.resolve_read(path).map_err(|_| {
                        ToolError::Denied("creation config is outside readable roots".into())
                    })?;
                }
            }
        }
        match &action {
            Action::Submit { prompt, .. } | Action::Steer { prompt, .. } => {
                text(prompt, 64 * 1024)?
            }
            Action::Rename { name, .. } => text(name, 256)?,
            _ => (),
        }
        let journal = if let Some(id) = mutation {
            let owner = self.context.as_ref().ok_or_else(|| {
                failed("mutations require a stable owning voyage context for durable journaling")
            })?;
            let root = journal::root(local, owner.session_id)?;
            let intent = json!({"version":1,"target":target,"request":arguments});
            match journal::admit(&root, id, &intent)? {
                journal::Admission::Existing(result) => return output(result, context),
                journal::Admission::Fresh => (),
            }
            Some((root, id))
        } else {
            None
        };
        if let Some((root, _)) = &journal {
            transport.journal(root.clone());
        }
        let timeout = context.timeout.min(Duration::from_secs(35));
        let result = tokio::select! {
            _ = context.cancellation.cancelled() => Err(ToolError::Cancelled),
            result = tokio::time::timeout(timeout, perform(action, &transport, context)) => result.unwrap_or(Err(ToolError::Timeout(timeout))),
        };
        if let Some((root, id)) = journal {
            let value = match result {
                Ok(value) => json!({"command_id":id,"result":value,"replayed":false}),
                Err(_) => {
                    json!({"command_id":id,"status":"outcome_unknown","replayed":false,"detail":"No confirmed result. Query receipt or inspect; never retry effects. Cancellation or timeout does not cancel the remote run."})
                }
            };
            // Apply secret redaction before durable result storage as well as model output.
            let value = redact(value, context)?;
            journal::finish(&root, id, &value)?;
            output(value, context)
        } else {
            output(result?, context)
        }
    }
}
fn check(context: &ToolContext) -> Result<(), ToolError> {
    if context.cancellation.is_cancelled() {
        return Err(ToolError::Cancelled);
    }
    context
        .policy
        .check_execution_authority()
        .map_err(|_| ToolError::Denied("execution authority unavailable".into()))
}
fn text(value: &str, max: usize) -> Result<(), ToolError> {
    if value.trim().is_empty() || value.len() > max {
        Err(invalid("text must be nonblank and within its byte limit"))
    } else {
        Ok(())
    }
}
fn page(limit: u32) -> Result<u32, ToolError> {
    if (1..=128).contains(&limit) {
        Ok(limit)
    } else {
        Err(invalid("limit must be 1..128"))
    }
}
fn redact(value: Value, context: &ToolContext) -> Result<Value, ToolError> {
    // Redact leaf strings rather than serialized JSON (secrets can contain JSON syntax).
    Ok(match value {
        Value::String(s) => Value::String(context.redactor.redact(s)),
        Value::Array(a) => Value::Array(
            a.into_iter()
                .map(|v| redact(v, context))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| Ok((k, redact(v, context)?)))
                .collect::<Result<_, ToolError>>()?,
        ),
        v => v,
    })
}
fn output(value: Value, context: &ToolContext) -> Result<String, ToolError> {
    let encoded = serde_json::to_string(&redact(value, context)?)
        .map_err(|_| failed("Vessel result encoding failed"))?;
    if encoded.len() > context.max_output_bytes {
        return Ok(json!({"status":"output_limit","detail":"Response exceeds output budget; request a smaller page. Mutation results remain in the journal; do not replay."}).to_string());
    }
    Ok(encoded)
}
async fn voyage(
    t: &transport::Transport,
    session_id: Uuid,
    incarnation: Option<Uuid>,
    command: VoyageCommand,
) -> Result<Value, ToolError> {
    let result = t
        .exchange(VesselCommand::Voyage(VoyageRequest {
            session_id,
            incarnation,
            command,
        }))
        .await?;
    if result.get("status").is_some() && result.get("session_id").is_none() {
        return Ok(result);
    }
    let reply: VoyageReply = serde_json::from_value(result)
        .map_err(|_| failed("invalid voyage response; outcome unknown"))?;
    if reply.session_id != session_id || incarnation.is_some_and(|i| i != reply.incarnation) {
        return Err(failed("Vessel response identity mismatch; outcome unknown"));
    }
    Ok(reply.result)
}
async fn perform(
    action: Action,
    t: &transport::Transport,
    context: &ToolContext,
) -> Result<Value, ToolError> {
    check(context)?;
    let expires_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| failed("system clock unavailable"))?
        .as_millis()
        .min(u64::MAX as u128) as u64
        + 60_000;
    match action {
        Action::Routes | Action::Operations { .. } => unreachable!(),
        Action::Controls {
            session_id,
            run_id,
            section,
        } => {
            voyage(
                t,
                session_id,
                None,
                VoyageCommand::Controls {
                    run_id,
                    section: match section {
                        ControlSection::Models => "models",
                        ControlSection::Policy => "policy",
                    }
                    .into(),
                },
            )
            .await
        }
        Action::Capabilities => t.exchange(VesselCommand::Capabilities).await,
        Action::List { offset, limit }
        | Action::Search {
            query: _,
            offset,
            limit,
        } => {
            let limit = page(limit)? as usize;
            let query = match action {
                Action::Search { query, .. } => {
                    text(&query, 4096)?;
                    Some(query.to_lowercase())
                }
                _ => None,
            };
            let value = t.exchange(VesselCommand::Catalogue).await?;
            let Some(entries) = value.as_array() else {
                return Ok(value);
            };
            let matched: Vec<_> = entries
                .iter()
                .filter(|v| {
                    query
                        .as_ref()
                        .is_none_or(|q| v.to_string().to_lowercase().contains(q))
                })
                .cloned()
                .collect();
            let end = offset.saturating_add(limit).min(matched.len());
            Ok(
                json!({"entries":matched.get(offset..end).unwrap_or(&[]),"total":matched.len(),"next_offset":(end < matched.len()).then_some(end)}),
            )
        }
        Action::Inspect { session_id } => {
            let registration = t.exchange(VesselCommand::Inspect { session_id }).await?;
            if registration.get("status").is_some() {
                return Ok(registration);
            }
            let snapshot = voyage(t, session_id, None, VoyageCommand::Snapshot).await?;
            if snapshot.get("status").is_some() {
                return Ok(snapshot);
            }
            Ok(json!({"registration":registration,"snapshot":snapshot}))
        }
        Action::History {
            session_id,
            offset,
            limit,
            expected_revision,
        } => {
            voyage(
                t,
                session_id,
                None,
                VoyageCommand::History {
                    offset,
                    limit: page(limit)?,
                    expected_revision,
                },
            )
            .await
        }
        Action::Follow {
            session_id,
            after,
            limit,
            wait_ms,
        }
        | Action::Wait {
            session_id,
            after,
            limit,
            wait_ms,
        } => {
            if wait_ms > 30_000 {
                return Err(invalid("wait_ms must be at most 30000"));
            }
            voyage(
                t,
                session_id,
                None,
                VoyageCommand::Events {
                    after,
                    limit: page(limit)?,
                    wait_ms,
                },
            )
            .await
        }
        Action::Receipt {
            session_id,
            command_id,
        } => {
            let session =
                session_id.ok_or_else(|| invalid("session_id required for server receipt"))?;
            let info = t
                .exchange(VesselCommand::Inspect {
                    session_id: session,
                })
                .await?;
            if let Some(receipt) = info
                .get("archive")
                .and_then(|a| a.get("receipt"))
                .or_else(|| info.get("deletion"))
            {
                if receipt.get("command_id").and_then(Value::as_str)
                    == Some(command_id.to_string().as_str())
                {
                    return Ok(receipt.clone());
                }
            }
            voyage(
                t,
                session_id.ok_or_else(|| invalid("session_id required for server receipt"))?,
                None,
                VoyageCommand::Receipt { command_id },
            )
            .await
        }
        Action::Create {
            command_id,
            session_id,
            workspace,
            config_path,
            task,
        } => {
            let command = match config_path {
                Some(config_path) => VesselCommand::StartConfigured {
                    command_id: session_id,
                    session_id,
                    workspace,
                    config_path,
                },
                None => VesselCommand::Start {
                    command_id: session_id,
                    session_id,
                    workspace,
                },
            };
            let started = t.exchange(command).await?;
            if started.get("status").is_some() {
                return Ok(
                    json!({"start_command_id":session_id,"start":started,"initial_task_submitted":false}),
                );
            }
            check(context)?;
            let snapshot = voyage(t, session_id, None, VoyageCommand::Snapshot).await?;
            let revision = snapshot
                .get("revision")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    failed("created voyage revision unavailable; initial task not submitted")
                })?;
            check(context)?;
            let submitted = voyage(
                t,
                session_id,
                None,
                VoyageCommand::Submit {
                    command_id,
                    expected_revision: revision,
                    expires_at_ms,
                    prompt: task,
                },
            )
            .await?;
            Ok(
                json!({"session_id":session_id,"start_command_id":session_id,"start":started,"submit":submitted}),
            )
        }
        Action::Submit {
            session_id,
            command_id,
            expected_revision,
            prompt,
        } => {
            text(&prompt, 64 * 1024)?;
            voyage(
                t,
                session_id,
                None,
                VoyageCommand::Submit {
                    command_id,
                    expected_revision,
                    expires_at_ms,
                    prompt,
                },
            )
            .await
        }
        Action::Steer {
            session_id,
            incarnation,
            run_id,
            command_id,
            expected_revision,
            prompt,
        } => {
            text(&prompt, 64 * 1024)?;
            voyage(
                t,
                session_id,
                Some(incarnation),
                VoyageCommand::Steer {
                    command_id,
                    expected_revision,
                    expires_at_ms,
                    run_id,
                    prompt,
                },
            )
            .await
        }
        Action::Cancel {
            session_id,
            incarnation,
            run_id,
            command_id,
            expected_revision,
        } => {
            voyage(
                t,
                session_id,
                Some(incarnation),
                VoyageCommand::Cancel {
                    command_id,
                    expected_revision,
                    expires_at_ms,
                    run_id,
                },
            )
            .await
        }
        Action::Rename {
            session_id,
            command_id,
            expected_revision,
            name,
        } => {
            text(&name, 256)?;
            voyage(
                t,
                session_id,
                None,
                VoyageCommand::Rename {
                    command_id,
                    expected_revision,
                    expires_at_ms,
                    name,
                },
            )
            .await
        }
        Action::Archive {
            session_id,
            command_id,
            expected_revision,
        }
        | Action::Restore {
            session_id,
            command_id,
            expected_revision,
        } => {
            if matches!(action, Action::Restore { .. }) {
                let info = t.exchange(VesselCommand::Inspect { session_id }).await?;
                if info.get("status").is_some() {
                    return Ok(info);
                }
                let info: voyage_protocol::vessel::ProcessInfo = serde_json::from_value(info)
                    .map_err(|_| failed("invalid restore inspection"))?;
                if info.session_id != session_id {
                    return Err(failed("restore session identity mismatch"));
                }
                if info.state == voyage_protocol::vessel::ProcessState::Stopped
                    && info.archive.is_some()
                {
                    check(context)?;
                    let restarted = t
                        .exchange(VesselCommand::Restart {
                            command_id,
                            session_id,
                            incarnation: info.incarnation,
                        })
                        .await?;
                    if restarted.get("status").is_some() {
                        return Ok(restarted);
                    }
                }
                check(context)?;
            }
            voyage(
                t,
                session_id,
                None,
                VoyageCommand::Archive {
                    command_id,
                    expected_revision,
                    expires_at_ms,
                    archived: matches!(action, Action::Archive { .. }),
                },
            )
            .await
        }
    }
}
