//! Keeps each workspace's runtime and native terminal ownership in this Helm
//! while the operator navigates saved voyages. A session switch is not a child
//! process launch; fresh target ownership is acquired before activating it.
use super::*;
use helm::tui::{UiBridge, UiEvent};
use std::collections::BTreeMap;

struct WorkspaceRuntime {
    agent: Arc<Agent>,
    bridge: Arc<UiBridge>,
    receiver: tokio::sync::mpsc::UnboundedReceiver<UiEvent>,
    subagents: Arc<SubagentRuntime>,
    supervisor: Arc<helm::supervision::RuntimeAgentSupervisor>,
    terminals: helm::tools::ProcessTool,
    todos: Arc<helm::todo::TodoStore>,
    provider_label: String,
    access: AccessMode,
    config: Config,
}
impl WorkspaceRuntime {
    async fn build(config: &Config, session: &Session) -> Result<Self> {
        let (bridge, receiver) = helm::tui::bridge();
        let mut active_config = config.clone();
        active_config.model = session.model.clone();
        let profile = active_config.provider_profile();
        let provider_label = format!(
            "{} ({})",
            profile.id,
            if profile.compatibility_bridge {
                "external bridge"
            } else {
                "native"
            }
        );
        let resolved =
            helm::runtime_policy::RuntimePolicy::resolve(&active_config, &session.workspace)?;
        let runtime_config = resolved.config();
        let policy = Arc::new(resolved.policy().clone());
        let context = ToolContext {
            completion: None,
            policy,
            approver: bridge.clone(),
            timeout: runtime_config.timeout(),
            max_output_bytes: runtime_config.max_output_bytes,
            environment: tool_environment(runtime_config),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Attended,
            redactor: redactor(runtime_config),
        };
        // Validate the provider before acquiring persistent runtime ownership or
        // starting external MCP servers. Navigation failures are recoverable.
        let provider = provider::from_config(runtime_config, session.workspace.clone())?;
        let subagents =
            build_subagents(runtime_config, &session.workspace, context.policy.clone()).await?;
        let subagent_runtime = subagents.runtime;
        let todo = subagents.todos.clone();
        let tools = match build_tools(
            runtime_config,
            Some(subagents.tool),
            Some(todo.clone()),
            Some(subagents.completion_tool),
        )
        .await
        {
            Ok(tools) => tools,
            Err(error) => {
                // No child work has been admitted yet. Explicitly release the
                // persistent runtime before returning to the active workspace.
                subagent_runtime.shutdown().await;
                return Err(error);
            }
        };
        let terminals = tools.terminals().unwrap_or_default();
        let agent = Arc::new(
            Agent::new(
                provider,
                tools,
                context,
                bridge.clone(),
                session.model.clone(),
                runtime_config.system_prompt.clone(),
                runtime_config.max_tokens,
                runtime_config.temperature,
            )
            .with_completion_coordinator(subagents.coordinator)
            .with_completion_gate(
                todo.store(),
                subagent_runtime.store().expect("persistent runtime"),
                subagent_runtime.clone(),
            )
            .with_context_window(runtime_config.context_window)
            .with_model_mirror(subagents.model)
            .with_retry_policy(RetryPolicy {
                max_attempts: runtime_config.provider_retry_attempts,
                initial_delay: std::time::Duration::from_millis(
                    runtime_config.provider_retry_initial_ms,
                ),
                max_delay: std::time::Duration::from_millis(runtime_config.provider_retry_max_ms),
            }),
        );

        Ok(Self {
            agent,
            bridge,
            receiver,
            supervisor: Arc::new(helm::supervision::RuntimeAgentSupervisor::new(
                subagent_runtime.clone(),
            )),
            subagents: subagent_runtime,
            terminals,
            todos: todo.store(),
            provider_label,
            access: runtime_config.access_mode(),
            config: active_config.clone(),
        })
    }
}

pub(super) async fn chat(
    config: Config,
    workspace_arg: Option<PathBuf>,
    resume: Option<String>,
    model_overridden: bool,
    verbose: bool,
    log_format: LogFormat,
) -> Result<()> {
    let mut runtimes = BTreeMap::<PathBuf, WorkspaceRuntime>::new();
    let store = SessionStore::default();
    let (mut store, mut session) = if let Some(reference) = resume {
        store.load_owned(&reference).await?
    } else {
        let session = Session::new(
            config.resolve_workspace(workspace_arg)?,
            config.model.clone(),
        );
        (store.with_execution(session.id).await?, session)
    };
    if model_overridden && session.switch_model(config.model.clone())? {
        store.save(&mut session).await?;
    }
    let outcome: Result<Option<(Config, uuid::Uuid, helm::tui::CliRequest)>> = async {
        let mut workspace = session.workspace.canonicalize()?;
        runtimes.insert(workspace.clone(), WorkspaceRuntime::build(&config, &session).await?);
        let mut notice = None;
        loop {
            let runtime = runtimes.get_mut(&workspace).expect("active workspace runtime");
            runtime.agent.set_model(session.model.clone())?;
            // Responses started by a previous idle view are not input for this
            // voyage. Dropping stale approval/question senders resolves them as
            // unavailable; no root run may be active during navigation.
            while runtime.receiver.try_recv().is_ok() {}
            let exit = helm::tui::run(runtime.agent.clone(), &mut store, session,
                &mut runtime.receiver, runtime.bridge.sender(), Arc::new(runtime.terminals.clone()),
                runtime.supervisor.clone(),
                runtime.todos.clone(), runtime.provider_label.clone(), runtime.access, notice.take()).await?;
            let current_id = store.owned_session_id().context("active session ownership missing")?;
            session = store.load(current_id).await?;
            match exit {
                helm::tui::TuiExit::Quit => return Ok(None),
                helm::tui::TuiExit::Launch(request) => {
                    let mut launch_config = runtime.config.clone();
                    launch_config.model = session.model.clone();
                    return Ok(Some((launch_config, current_id, request)));
                }
                helm::tui::TuiExit::Navigate(reference) => {
                    let candidate = async {
                        let (next_store, next_session) = store.load_owned(&reference).await?;
                        let next_workspace = next_session.workspace.canonicalize()?;
                        if !runtimes.contains_key(&next_workspace) {
                            let next_runtime = WorkspaceRuntime::build(&config, &next_session).await?;
                            runtimes.insert(next_workspace.clone(), next_runtime);
                        }
                        Ok::<_, anyhow::Error>((next_store, next_session, next_workspace))
                    }.await;
                    match candidate {
                        Ok((next_store, next_session, next_workspace)) => {
                            store = next_store;
                            session = next_session;
                            workspace = next_workspace;
                        }
                        Err(error) => notice = Some(format!("Cannot switch voyages: {error}; current voyage and terminals retained")),
                    }
                }
            }
        }
    }.await;
    // Real Helm exit/configuration handoff owns shutdown for every retained
    // workspace, including inactive ones. Navigation never reaches this path.
    let mut cleanup = Ok(());
    for runtime in runtimes.values() {
        if tokio::time::timeout(
            std::time::Duration::from_secs(10),
            runtime.subagents.shutdown(),
        )
        .await
        .is_err()
        {
            cleanup = Err(anyhow::anyhow!(
                "subagent shutdown timed out; refusing frontend handoff"
            ));
        }
        let terminals = runtime
            .terminals
            .shutdown(std::time::Duration::from_secs(5))
            .await;
        if !terminals.observation_complete {
            cleanup = Err(anyhow::anyhow!(
                "terminal cleanup unconfirmed; refusing frontend handoff"
            ));
        }
    }
    drop(runtimes);
    drop(store);
    cleanup?;
    match outcome? {
        None => Ok(()),
        Some((config, id, request)) => {
            launch_from_tui(&config, id, request, verbose, log_format).await
        }
    }
}

#[cfg(test)]
mod tests;
