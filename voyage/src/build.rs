use crate::{
    Agent, AgentEvent, Config, EventSink,
    agent::RetryPolicy,
    config::{AccessMode, UnattendedApprovalMode},
    policy::Policy,
    provider,
    subagent::{
        AgentBudget, AgentPolicy, ApprovalPolicy, ExecutionContext, RuntimeLimits,
        SubagentExecutor, SubagentResult, SubagentRuntime, SubagentTool, WorktreeManager,
    },
    todo::{TodoScope, TodoStore},
    tools::{
        Approver, InteractionMode, Redactor, TodoTool, ToolContext, ToolRegistry,
        UnattendedApprover,
    },
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, RwLock, Weak},
};
mod resources;
mod subagents;
pub use resources::{ManagedResourceState, ManagedResources};
pub use subagents::{SubagentBundle, build_subagents_managed};
pub struct ManagedAgent {
    pub agent: Arc<Agent>,
    pub subagents: Arc<SubagentRuntime>,
    pub resources: Option<Arc<ManagedResources>>,
}
#[derive(Default)]
pub struct BuildResources {
    pub extra_tool: Option<Arc<dyn crate::tools::Tool>>,
    pub terminal_manager: Option<crate::tools::ProcessTool>,
}
pub struct BuildFailure {
    pub stage: &'static str,
    pub cleanup_observed: bool,
}

pub async fn build_authorized_agent_bundle(
    config: &Config,
    workspace: PathBuf,
    attended: bool,
    sink: Option<Arc<dyn EventSink>>,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    decision_approver: Option<Arc<dyn Approver>>,
    retained: BuildResources,
) -> std::result::Result<ManagedAgent, BuildFailure> {
    // Keep observers outside the fallible assembly so partially built resources
    // remain available for positive cleanup observation.
    let managed_resources = Arc::new(ManagedResources::default());
    let mut runtime: Option<Arc<SubagentRuntime>> = None;
    let mut stage = "runtime policy";
    let result: Result<ManagedAgent> = async {
        let BuildResources {
            extra_tool,
            terminal_manager,
        } = retained;
        if let Some(authority) = &authority {
            authority.check()?;
        }
        let resolved = crate::runtime_policy::RuntimePolicy::resolve(config, &workspace)?;
        let config = resolved.config();
        let mut policy = resolved
            .policy()
            .clone()
            .with_live_access(config.live_access.clone());
        if let Some(authority) = authority {
            policy = policy.with_execution_authority(authority);
        }
        policy.check_execution_authority()?;
        let policy = Arc::new(policy);

        let interactive = decision_approver.is_some();
        let approver: Arc<dyn Approver> = if let Some(approver) = decision_approver {
            approver
        } else if attended {
            Arc::new(UnattendedApprover { allow: false })
        } else {
            Arc::new(UnattendedApprover {
                allow: config.unattended_approval == UnattendedApprovalMode::Allow,
            })
        };
        let context = ToolContext {
            github: crate::github::Credential::from_config(config),
            completion: None,
            policy,
            approver,
            timeout: config.timeout(),
            max_output_bytes: config.max_output_bytes,
            environment: tool_environment(config),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: if attended || interactive {
                InteractionMode::Attended
            } else {
                InteractionMode::Unattended
            },
            redactor: redactor(config),
        };
        stage = "inference accounting";
        let accounting =
            crate::inference::runtime::Accounting::root(&workspace, &config.provider_profile())
                .await?;
        stage = "subagent initialization";
        let subagents = build_subagents_managed(
            config,
            &workspace,
            context.policy.clone(),
            Some(managed_resources.clone()),
        )
        .await?;
        runtime = Some(subagents.runtime.clone());
        let gate_runtime = subagents.runtime.clone();
        let gate_todos = subagents.todos.store();
        let gate_agents = gate_runtime.store().expect("persistent runtime");
        context.policy.check_execution_authority()?;
        stage = "tool initialization";
        let mut tools = build_tools(
            config,
            Some(subagents.tool),
            Some(subagents.todos),
            Some(managed_resources.as_ref()),
            &context.policy,
        )
        .await?;
        if let Some(tool) = extra_tool {
            tools.register_arc(tool)?;
        }
        if let Some(manager) = terminal_manager {
            tools.reuse_terminals(manager);
        }
        managed_resources.register(&mut tools)?;
        let retained_runtime = gate_runtime.clone();
        context.policy.check_execution_authority()?;
        stage = "provider configuration";
        let agent = Agent::new(
            provider::from_config(config, context.policy.workspace().to_owned())?,
            tools,
            context,
            sink.unwrap_or_else(|| Arc::new(SilentEvents)),
            config.model.clone(),
            config.system_prompt.clone(),
            config.max_tokens,
            config.temperature,
        )
        .with_inference_settings(config.reasoning_effort.clone(), config.service_tier.clone())
        .with_inference_accounting(accounting)
        .with_completion_coordinator(subagents.coordinator)
        .with_completion_gate(gate_todos, gate_agents, gate_runtime)
        .with_context_window(config.context_window)
        .with_model_mirror(subagents.model)
        .with_retry_policy(RetryPolicy {
            max_attempts: config.provider_retry_attempts,
            initial_delay: std::time::Duration::from_millis(config.provider_retry_initial_ms),
            max_delay: std::time::Duration::from_millis(config.provider_retry_max_ms),
        });
        Ok(ManagedAgent {
            agent: Arc::new(agent),
            subagents: retained_runtime,
            resources: Some(managed_resources.clone()),
        })
    }
    .await;
    match result {
        Ok(agent) => Ok(agent),
        Err(_) => {
            let children = match runtime {
                Some(runtime) => {
                    tokio::time::timeout(std::time::Duration::from_secs(15), runtime.shutdown())
                        .await
                        .is_ok()
                }
                None => true,
            };
            let resources = managed_resources.shutdown_observed(true).await.is_ok();
            let compatibility = provider::shutdown_compatibility().await.is_ok();
            Err(BuildFailure {
                stage,
                cleanup_observed: children && resources && compatibility,
            })
        }
    }
}

pub fn tool_environment(config: &Config) -> std::collections::BTreeMap<String, String> {
    let mut environment = config
        .inherit_env
        .iter()
        .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
        .collect::<std::collections::BTreeMap<_, _>>();
    environment.extend(config.env.clone());
    environment
}

pub fn redactor(config: &Config) -> Arc<Redactor> {
    let secrets = config
        .redact_values
        .iter()
        .cloned()
        .chain(config.env.values().cloned())
        .chain(
            config
                .mcp_servers
                .values()
                .flat_map(|server| server.env.values().cloned()),
        )
        .chain(config.api_key_for_redaction())
        .chain(crate::github::credential_redactions(config));
    Arc::new(Redactor::new(secrets))
}

mod tools;
pub use tools::{build_tools, todo_tool};

struct SilentEvents;
#[async_trait]
impl EventSink for SilentEvents {
    async fn emit(&self, _: AgentEvent) {}
}

static RESOURCE_ROOT: OnceLock<PathBuf> = OnceLock::new();
pub(crate) fn set_resource_root(path: PathBuf) -> Result<()> {
    RESOURCE_ROOT
        .set(path)
        .map_err(|_| anyhow::anyhow!("runtime resource scope already set"))
}
pub(crate) fn resource_root() -> PathBuf {
    RESOURCE_ROOT
        .get()
        .cloned()
        .unwrap_or_else(crate::config::default_data_dir)
}
