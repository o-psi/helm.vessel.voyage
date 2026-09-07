use super::*;
pub fn todo_tool(
    workspace: &std::path::Path,
    coordinator: crate::completion::runtime::Coordinator,
) -> TodoTool {
    let workspace = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    TodoTool::new(Arc::new(
        TodoStore::new(
            resource_root().join("todos").join(format!("{key}.json")),
            TodoScope::workspace(workspace),
        )
        .with_coordinator(coordinator),
    ))
}

pub async fn build_tools(
    config: &Config,
    subagents: Option<SubagentTool>,
    todos: Option<TodoTool>,
    completion: Option<crate::completion::tool::CompletionTool>,
    resources: Option<&ManagedResources>,
    policy: &Policy,
) -> Result<ToolRegistry> {
    policy.check_current()?;
    let mut tools = ToolRegistry::standard_with_terminal_limits(
        config.terminal_max_count,
        config.terminal_max_unread_bytes,
    );
    if let Some(tool) = subagents {
        tools.register_subagents(tool)?;
    }
    if let Some(tool) = todos {
        tools.register_todos(tool)?;
    }
    if let Some(tool) = completion {
        tools.register_arc(Arc::new(tool))?;
    }
    if config.github_enabled && crate::github::Credential::from_config(config).is_some() {
        tools.register(crate::github::tool::GithubTool);
    }
    if config.access_mode() == AccessMode::ReadOnly {
        // Do not even start external MCP servers in read-only mode: their
        // initialization and tool contracts are outside Helm's authority model.
        // Live process runtimes retain built-ins behind per-dispatch policy checks
        // so an access upgrade does not need to rebuild the running agent.
        if config.live_access.is_none() {
            tools.retain_read_only();
        }
        return Ok(tools);
    }
    // Keep transport ownership until assembly succeeds so a later discovery or
    // registration failure can reap every server already started by this build.
    let mut servers = Vec::new();
    let assembly: Result<()> = async {
        for (name, server) in &config.mcp_servers {
            let mut environment = tool_environment(config);
            environment.extend(server.env.clone());
            let start = || {
                policy.check_current()?;
                crate::tools::mcp::McpServer::start(
                    name,
                    &server.command,
                    &server.args,
                    &environment,
                )
                .map(Arc::new)
                .with_context(|| format!("failed to start MCP server `{name}`"))
            };
            // Admission and observer registration are atomic with spawn. A
            // cancelled child initialization cannot hide a server from cleanup.
            let mcp = match resources {
                Some(resources) => resources.start_mcp(start)?,
                None => start()?,
            };
            tools.own_mcp(mcp.clone());
            servers.push(mcp);
            let mcp = servers.last().expect("new MCP server");
            tokio::time::timeout(config.timeout(), mcp.initialize())
                .await
                .with_context(|| format!("MCP server `{name}` initialization timed out"))?
                .with_context(|| format!("failed to initialize MCP server `{name}`"))?;
            let discovered = tokio::time::timeout(config.timeout(), mcp.discover())
                .await
                .with_context(|| format!("MCP server `{name}` discovery timed out"))?
                .with_context(|| format!("failed to discover tools from MCP server `{name}`"))?;
            for tool in discovered {
                tools
                    .register_arc(tool)
                    .with_context(|| format!("MCP server `{name}` exposed a duplicate tool"))?;
            }
        }
        policy.check_current()?;
        Ok(())
    }
    .await;
    if let Err(error) = assembly {
        let mut cleanup_failed = false;
        for server in &servers {
            cleanup_failed |= !matches!(
                tokio::time::timeout(std::time::Duration::from_secs(5), server.shutdown()).await,
                Ok(Ok(()))
            );
        }
        return Err(if cleanup_failed {
            error.context("failed runtime MCP cleanup unconfirmed")
        } else {
            error
        });
    }
    Ok(tools)
}
