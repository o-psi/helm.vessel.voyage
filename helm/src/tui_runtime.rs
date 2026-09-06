//! Keeps each workspace's runtime and native terminal ownership in this Helm
//! while the operator navigates saved voyages. A session switch is not a child
//! process launch; fresh target ownership is acquired before activating it.
use super::*;
use helm::policy_profile::switching::SwitchContext;
use helm::tui::{UiBridge, UiEvent};
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
#[error("candidate runtime cleanup unconfirmed; automatic policy fallback refused")]
struct UnconfirmedRuntimeCleanup;

#[derive(Debug, Default)]
struct Admission(std::sync::atomic::AtomicBool);
impl helm::policy::ExecutionAuthority for Admission {
    fn check(&self) -> Result<()> {
        anyhow::ensure!(
            !self.0.load(std::sync::atomic::Ordering::Acquire),
            "policy handoff blocked this runtime; reopen the saved voyage to resolve its policy"
        );
        Ok(())
    }
}

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
    policy: SwitchContext,
    admission: Arc<Admission>,
    resources: Arc<ManagedResources>,
}
impl WorkspaceRuntime {
    async fn build(config: &Config, session: &Session) -> Result<Self> {
        Self::build_checked(config, session, None).await
    }
    async fn build_checked(
        config: &Config,
        session: &Session,
        expected: Option<&str>,
    ) -> Result<Self> {
        Self::build_with_resources(
            config,
            session,
            expected,
            Arc::new(ManagedResources::local()),
        )
        .await
    }
    async fn build_with_resources(
        config: &Config,
        session: &Session,
        expected: Option<&str>,
        resources: Arc<ManagedResources>,
    ) -> Result<Self> {
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
        anyhow::ensure!(
            expected.is_none_or(|digest| digest == resolved.policy().effective().digest()),
            "policy changed during handoff; review again"
        );
        let policy_context =
            SwitchContext::new(active_config.clone(), resolved.policy().effective().clone());
        let runtime_config = resolved.config();
        let admission = Arc::new(Admission::default());
        let policy = Arc::new(
            resolved
                .policy()
                .clone()
                .with_execution_authority(admission.clone()),
        );
        let context = ToolContext {
            github: helm::github::Credential::from_config(runtime_config),
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
        let accounting = helm::inference::runtime::Accounting::root(
            &session.workspace,
            &runtime_config.provider_profile(),
        )
        .await?;
        let provider = provider::from_config(runtime_config, session.workspace.clone())?;
        let subagents = build_subagents_managed(
            runtime_config,
            &session.workspace,
            context.policy.clone(),
            Some(resources.clone()),
        )
        .await?;
        let subagent_runtime = subagents.runtime;
        let todo = subagents.todos.clone();
        let mut tools = match build_tools(
            runtime_config,
            Some(subagents.tool),
            Some(todo.clone()),
            Some(subagents.completion_tool),
            Some(&resources),
            &context.policy,
        )
        .await
        {
            Ok(tools) => tools,
            Err(error) => {
                return Err(Self::construction_error(
                    error,
                    &subagent_runtime,
                    &resources,
                    expected.is_some(),
                )
                .await);
            }
        };
        if let Err(error) = resources.register(&mut tools) {
            return Err(Self::construction_error(
                error,
                &subagent_runtime,
                &resources,
                expected.is_some(),
            )
            .await);
        }
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
            .with_inference_accounting(accounting)
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
            policy: policy_context,
            admission,
            resources,
        })
    }
    async fn construction_error(
        error: anyhow::Error,
        children: &SubagentRuntime,
        resources: &ManagedResources,
        handoff: bool,
    ) -> anyhow::Error {
        let closed = resources.close();
        let children =
            tokio::time::timeout(std::time::Duration::from_secs(10), children.shutdown()).await;
        let observed = resources
            .shutdown_observed(handoff || cfg!(target_os = "linux"))
            .await;
        if closed.is_err() || children.is_err() || observed.is_err() {
            error.context(UnconfirmedRuntimeCleanup)
        } else {
            error
        }
    }
    async fn check_idle(&self) -> Result<()> {
        anyhow::ensure!(
            Arc::strong_count(&self.agent) == 1 && Arc::strong_count(&self.supervisor) == 1,
            "wait for outstanding model, title or supervisor requests"
        );
        anyhow::ensure!(
            !self
                .subagents
                .list()
                .await
                .iter()
                .any(|record| !record.status.is_terminal()),
            "finish or cancel child work first"
        );
        anyhow::ensure!(
            !self.resources.has_owned_work()?,
            "close terminals and finish background shell work first"
        );
        #[cfg(not(target_os = "linux"))]
        anyhow::ensure!(
            self.access == AccessMode::ReadOnly,
            "shell/MCP process-session cleanup cannot be observed on this platform"
        );
        Ok(())
    }
    async fn stop_observed(&self) -> Result<()> {
        self.admission
            .0
            .store(true, std::sync::atomic::Ordering::Release);
        self.resources.close()?;
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.subagents.shutdown(),
        )
        .await
        .context("child shutdown unconfirmed; runtime remains blocked")?;
        self.resources.shutdown_observed(true).await
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
                runtime.todos.clone(), runtime.provider_label.clone(), runtime.access, Some(runtime.policy.clone()), notice.take()).await?;
            let current_id = store.owned_session_id().context("active session ownership missing")?;
            session = store.load(current_id).await?;
            match exit {
                helm::tui::TuiExit::Quit => return Ok(None),
                helm::tui::TuiExit::Launch(request) => {
                    let mut launch_config = runtime.config.clone();
                    launch_config.model = session.model.clone();
                    return Ok(Some((launch_config, current_id, request)));
                }
                helm::tui::TuiExit::SwitchPolicy(request) => {
                    // Stage and validate without disturbing the old runtime. Source
                    // reads run off the UI executor and exact rules are checked again
                    // in build_checked before new provider/MCP construction.
                    let context = runtime.policy.clone();
                    let prepared = tokio::task::spawn_blocking(move || {
                        helm::policy_profile::switching::read_sources(|| context.prepare_with_digest(&request))
                    }).await;
                    let (candidate, digest) = match prepared {
                        Ok(Ok(value)) => value,
                        _ => { notice = Some("Policy changed or confirmation invalid; reopen policy review. Current runtime retained.".into()); continue; }
                    };
                    if let Err(error) = runtime.check_idle().await {
                        notice = Some(format!("Cannot switch policy: {error}. Current runtime retained."));
                        continue;
                    }
                    let old_config = runtime.config.clone();
                    let old_digest = runtime.policy.current().digest().to_owned();
                    if let Err(error) = runtime.stop_observed().await {
                        notice = Some(error.to_string());
                        continue;
                    }
                    // Drop every persistent writer only after observed cleanup; the
                    // SessionStore lease and canonical draft remain owned throughout.
                    drop(runtimes.remove(&workspace));
                    match WorkspaceRuntime::build_checked(&candidate, &session, Some(&digest)).await {
                        Ok(next) => {
                            runtimes.insert(workspace.clone(), next);
                            notice = Some("Policy applied to this workspace runtime. Launch defaults are unchanged.".into());
                        }
                        Err(error) => {
                            if error.downcast_ref::<UnconfirmedRuntimeCleanup>().is_some() {
                                return Err(anyhow::Error::new(UnconfirmedRuntimeCleanup).context("policy switch stopped; saved voyage retained, review unconfirmed cleanup before reopening"));
                            }
                            let fallback = WorkspaceRuntime::build_checked(&old_config, &session, Some(&old_digest)).await
                                .map_err(|_| anyhow::anyhow!("policy handoff failed and exact previous policy cannot be restored; saved voyage retained, reopen with freshly reviewed policy"))?;
                            runtimes.insert(workspace.clone(), fallback);
                            notice = Some("Policy handoff failed; exact previous policy restored after fresh validation.".into());
                        }
                    }
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
        runtime
            .admission
            .0
            .store(true, std::sync::atomic::Ordering::Release);
        if let Err(error) = runtime
            .resources
            .shutdown_observed(cfg!(target_os = "linux"))
            .await
        {
            cleanup = Err(error);
        }
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
