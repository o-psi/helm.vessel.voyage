use super::*;
fn inherited_child_workspace(
    records: Vec<(
        crate::subagent::AgentId,
        Option<crate::subagent::AgentId>,
        Option<PathBuf>,
    )>,
    id: crate::subagent::AgentId,
    fallback: &std::path::Path,
) -> Result<PathBuf> {
    let count = records.len();
    let records = records
        .into_iter()
        .map(|(id, parent, path)| (id, (parent, path)))
        .collect::<std::collections::BTreeMap<_, _>>();
    anyhow::ensure!(
        records.len() == count,
        "duplicate subagent workspace ancestry"
    );
    let mut visited = std::collections::BTreeSet::new();
    let mut cursor = Some(id);
    let mut nearest = None;
    while let Some(id) = cursor {
        anyhow::ensure!(visited.insert(id), "cyclic subagent workspace ancestry");
        let (parent, path) = records
            .get(&id)
            .context("subagent workspace ancestry unavailable")?;
        if nearest.is_none() {
            nearest = path.clone();
        }
        cursor = *parent;
    }
    // Every visited node must be a distinct member of the captured finite map.
    Ok(nearest.unwrap_or_else(|| fallback.to_path_buf()))
}
fn check_requested_workspace(
    workspace: &std::path::Path,
    read: &[PathBuf],
    write: &[PathBuf],
) -> Result<()> {
    let workspace = workspace.canonicalize()?;
    let contains = |roots: &[PathBuf]| -> Result<bool> {
        Ok(roots
            .iter()
            .map(|root| root.canonicalize())
            .collect::<std::io::Result<Vec<_>>>()?
            .iter()
            .any(|root| workspace.starts_with(root)))
    };
    anyhow::ensure!(
        contains(read)? && contains(write)?,
        "child workspace exceeds requested read/write delegation before implicit roots"
    );
    Ok(())
}

/// Attribution is added outside the model-facing tool arguments. Delegated
/// policy and DispatchApprover still enforce authority before and after consent.
struct WorkerApprover {
    inner: Arc<dyn Approver>,
    source: crate::tools::ApprovalSource,
}
#[async_trait]
impl Approver for WorkerApprover {
    async fn approve(
        &self,
        request: &crate::tools::ApprovalRequest,
    ) -> crate::tools::ApprovalOutcome {
        let mut request = request.clone();
        request.source = Some(self.source.clone());
        request.mode = InteractionMode::Attended;
        self.inner.approve(&request).await
    }
}

struct CliSubagentExecutor {
    approver: Option<Arc<dyn Approver>>,
    managed_resources: Option<Arc<ManagedResources>>,
    pub todos: TodoTool,
    config: Config,
    parent_policy: Arc<Policy>,
    workspace: PathBuf,
    runtime: OnceLock<Weak<SubagentRuntime>>,
    worktrees: Option<WorktreeManager>,
    worktree_error: Option<String>,
    pub model: Arc<RwLock<String>>,
}
#[async_trait]
impl SubagentExecutor for CliSubagentExecutor {
    async fn execute(
        &self,
        mut context: ExecutionContext,
    ) -> std::result::Result<SubagentResult, String> {
        context.progress("initializing provider and tools").await;
        let mut config = self.config.clone();
        config.model = self
            .model
            .read()
            .expect("subagent model lock poisoned")
            .clone();
        let workspace = if let Some(worktree) = &context.worktree {
            worktree.clone()
        } else {
            let runtime = self
                .runtime
                .get()
                .and_then(Weak::upgrade)
                .context("subagent runtime unavailable")
                .map_err(|error| error.to_string())?;
            let records = runtime
                .list()
                .await
                .into_iter()
                .map(|record| (record.id, record.parent_id, record.worktree))
                .collect();
            inherited_child_workspace(records, context.id, &self.workspace)
                .map_err(|error| error.to_string())?
        };
        check_requested_workspace(
            &workspace,
            &context.policy.readable_roots,
            &context.policy.writable_roots,
        )
        .map_err(|error| error.to_string())?;
        config.workspace = Some(workspace);
        // Resolve this request beneath its immediate parent, not merely the root
        // executor snapshot. The captured cap also survives later root upgrades.
        config.access = Some(context.policy.access);
        config.allow_read = context.policy.readable_roots.clone();
        config.allow_write = context.policy.writable_roots.clone();
        config.max_tokens = context
            .policy
            .budget
            .response_limit(context.budget.response_limit(config.max_tokens));
        let workspace = config.resolve_workspace(None).map_err(|e| e.to_string())?;
        let resolved = crate::runtime_policy::RuntimePolicy::resolve_child(
            &config,
            &workspace,
            &self.parent_policy,
        )
        .map_err(|e| e.to_string())?;
        let config = resolved.config().clone();
        let policy = Arc::new(resolved.policy().clone());
        let approver = if context.policy.approval != ApprovalPolicy::Deny {
            self.approver.clone()
        } else {
            None
        };
        let interaction = if approver.is_some() {
            InteractionMode::Attended
        } else {
            InteractionMode::Unattended
        };
        let approver: Arc<dyn Approver> = if let Some(inner) = approver {
            let runtime = self
                .runtime
                .get()
                .and_then(Weak::upgrade)
                .ok_or("subagent runtime unavailable")?;
            let record = runtime.get(context.id).await.map_err(|e| e.to_string())?;
            Arc::new(WorkerApprover {
                inner,
                source: crate::tools::ApprovalSource {
                    agent_id: context.id.0,
                    name: redactor(&config).redact(record.name),
                },
            })
        } else {
            Arc::new(UnattendedApprover { allow: false })
        };
        let tool_context = ToolContext {
            tool_call_id: None,
            github: crate::github::Credential::from_config(&config),
            artifact_scope: config.artifact_scope.clone(),
            completion: context.completion.clone(),
            policy,
            approver,
            timeout: config.timeout(),
            max_output_bytes: config.max_output_bytes,
            environment: tool_environment(&config),
            cancellation: context.cancellation.clone(),
            execution_id: uuid::Uuid::new_v4(),
            interaction,
            redactor: redactor(&config),
        };
        let child_worktrees = self.worktrees.clone().map(|manager| {
            if resolved.ceiling_present() {
                manager.with_environment(tool_environment(&config))
            } else {
                manager
            }
        });
        let child_budget = context.budget.clone();
        let mut child_policy = context.policy.clone();
        child_policy.limit_access(tool_context.policy.access_mode());
        child_policy.budget = child_budget.clone();
        child_policy.readable_roots = config.allow_read.clone();
        child_policy.writable_roots = config.allow_write.clone();
        let child_tool = self.runtime.get().and_then(Weak::upgrade).map(|runtime| {
            SubagentTool::new(runtime, child_policy, child_budget)
                .with_parent(context.id)
                .with_worktrees(child_worktrees)
                .with_worktree_error(self.worktree_error.clone())
        });
        // Worktree-isolated children still coordinate through the parent's workspace plan.
        // Keying todos by the temporary worktree would silently fork task state.
        tool_context
            .policy
            .check_execution_authority()
            .map_err(|error| error.to_string())?;
        let accounting =
            crate::inference::runtime::Accounting::child(&config.provider_profile(), context.id.0)
                .await
                .map_err(|error| error.to_string())?;
        let mut tools = build_tools(
            &config,
            child_tool,
            Some(self.todos.clone()),
            self.managed_resources.as_deref(),
            &tool_context.policy,
        )
        .await
        .map_err(|e| e.to_string())?;
        tools.retain_allowed(&context.policy.allowed_tools);
        if let Some(resources) = &self.managed_resources {
            resources
                .register(&mut tools)
                .map_err(|error| error.to_string())?;
        }
        tool_context
            .policy
            .check_execution_authority()
            .map_err(|error| error.to_string())?;
        let agent = Agent::new(
            provider::from_config(&config).map_err(|e| e.to_string())?,
            tools,
            tool_context,
            context.inference_warning_sink(),
            config.model.clone(),
            config.system_prompt.clone(),
            config.max_tokens,
            config.temperature,
        )
        .with_inference_provider(config.provider.clone())
        .with_inference_settings(config.reasoning_effort.clone(), config.service_tier.clone())
        .with_inference_accounting(accounting)
        .with_context_window(config.context_window)
        .with_retry_policy(RetryPolicy {
            max_attempts: config.provider_retry_attempts,
            initial_delay: std::time::Duration::from_millis(config.provider_retry_initial_ms),
            max_delay: std::time::Duration::from_millis(config.provider_retry_max_ms),
        });
        let mut inbox = context.take_inbox();
        let (input_tx, input_rx) = crate::agent::steering_channel(64);
        let input_cancel = context.cancellation.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {_=input_cancel.cancelled()=>break,message=inbox.recv()=>match message{Some(crate::subagent::InboxMessage::Message(text))=>{if input_tx.send(format!("Message from supervisor: {text}")).await.is_err(){break}},Some(crate::subagent::InboxMessage::FollowUp(text))=>{if input_tx.send(format!("Follow-up instruction: {text}")).await.is_err(){break}},None=>break}}
            }
        });
        let outcome = agent
            .run_with_cancel_and_input(
                Vec::new(),
                context.task,
                context.cancellation,
                Some(input_rx),
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok(SubagentResult {
            summary: outcome.answer,
        })
    }
}

pub struct SubagentBundle {
    pub coordinator: crate::completion::runtime::Coordinator,
    pub todos: TodoTool,
    pub runtime: Arc<SubagentRuntime>,
    pub tool: SubagentTool,
    pub model: Arc<RwLock<String>>,
}
pub async fn build_subagents_managed(
    config: &Config,
    workspace: &std::path::Path,
    parent_policy: Arc<Policy>,
    managed_resources: Option<Arc<ManagedResources>>,
    approver: Option<Arc<dyn Approver>>,
) -> Result<SubagentBundle> {
    parent_policy.check_execution_authority()?;
    let standard = ToolRegistry::standard();
    let mut allowed_tools: std::collections::BTreeSet<String> = standard
        .definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    allowed_tools.insert("subagent".to_string());
    allowed_tools.insert("todo".to_string());
    if config.vessel.enabled {
        allowed_tools.insert("vessel".into());
    }
    if config.github_enabled && parent_policy.effective().rules().github_enabled {
        allowed_tools.insert("github".into());
    }
    let budget = AgentBudget {
        max_tokens: config.max_tokens as u64,

        max_terminals: config.terminal_max_count.min(u32::MAX as usize) as u32,
    };
    let policy = AgentPolicy {
        // The live root policy clamps each spawn; do not pin root delegation to
        // its initial access mode when the operator may update it mid-run.
        access: AccessMode::Unrestricted,
        readable_roots: std::iter::once(workspace.to_path_buf())
            .chain(config.allow_read.clone())
            .collect(),
        writable_roots: std::iter::once(workspace.to_path_buf())
            .chain(config.allow_write.clone())
            .collect(),
        allowed_tools,
        approval: if approver.is_some() {
            ApprovalPolicy::Inherit
        } else {
            ApprovalPolicy::Deny
        },
        budget: budget.clone(),
    };
    let workspace_key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    let completion_root = resource_root().join("completion");
    let mut directories = std::fs::DirBuilder::new();
    directories.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directories.mode(0o700);
    }
    directories.create(&completion_root)?;
    // The agent tree and its completion ledger belong to this session's
    // resource root. Their writer lease must have the same scope: another
    // voyage using the workspace owns a different tree and may run concurrently.
    let coordinator = crate::completion::runtime::Coordinator::open(
        completion_root.join(&workspace_key),
        workspace,
    )?;
    let todos = todo_tool(workspace, coordinator.clone());
    let (worktrees, worktree_error) = match worktree_manager(workspace) {
        Ok(manager) => (manager, None),
        Err(error) => (
            None,
            Some(format!("Git worktree initialization failed: {error:#}")),
        ),
    };
    let worktrees = worktrees.map(|manager| {
        if parent_policy.ceiling_present() {
            manager.with_environment(tool_environment(config))
        } else {
            manager
        }
    });
    let model = Arc::new(RwLock::new(config.model.clone()));
    let executor = Arc::new(CliSubagentExecutor {
        approver,
        managed_resources,
        todos: todos.clone(),
        config: config.clone(),
        parent_policy,
        workspace: workspace.to_path_buf(),
        runtime: OnceLock::new(),
        worktrees: worktrees.clone(),
        worktree_error: worktree_error.clone(),
        model: model.clone(),
    });
    let store = crate::subagent::AgentTreeStore::new(
        resource_root()
            .join("subagents")
            .join(format!("{workspace_key}.json")),
    )
    .with_coordinator(coordinator.clone());
    let runtime = Arc::new(
        SubagentRuntime::new_persistent(
            executor.clone(),
            RuntimeLimits {
                max_concurrency: config.subagent_max_concurrency,
                event_history: config.subagent_event_history,
            },
            store,
        )
        .await
        .map_err(anyhow::Error::msg)?,
    );
    executor
        .runtime
        .set(Arc::downgrade(&runtime))
        .map_err(|_| anyhow::anyhow!("subagent runtime already initialized"))?;
    let tool = SubagentTool::new(runtime.clone(), policy, budget)
        .with_worktrees(worktrees)
        .with_worktree_error(worktree_error);
    Ok(SubagentBundle {
        coordinator,
        todos,
        runtime,
        tool,
        model,
    })
}

fn worktree_manager(workspace: &std::path::Path) -> Result<Option<WorktreeManager>> {
    WorktreeManager::discover_managed(workspace, &resource_root())
}
