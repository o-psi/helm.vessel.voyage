//! Idle views read session inventories and policy; they do not instantiate an agent.
use super::*;
use sha2::{Digest, Sha256};
pub(super) async fn inspect(
    controls: &LiveControls,
    section: &str,
    config: &crate::Config,
    workspace: &std::path::Path,
) -> Result<Value> {
    let resolved = crate::runtime_policy::RuntimePolicy::resolve(config, workspace)?;
    let root = crate::build::resource_root();
    let workspace = workspace.canonicalize()?;
    let key = hex::encode(Sha256::digest(workspace.as_os_str().as_encoded_bytes()));
    let value = match section {
        "tools" => builtin_preflight(resolved.config(), resolved.policy())?,
        "policy" => serde_json::to_value(resolved.policy().effective())?,
        "models" => {
            let reservation =
                crate::host_resources::Reservation::acquire("executors", uuid::Uuid::new_v4(), 1)?;
            let discovery = crate::server::models::discover(config, &workspace).await;
            reservation.release_observed()?;
            serde_json::to_value(discovery?)?
        }
        "todos" => serde_json::to_value(
            crate::todo::TodoStore::new(
                root.join("todos").join(format!("{key}.json")),
                crate::todo::TodoScope::workspace(workspace),
            )
            .snapshot()
            .await?,
        )?,
        "subagents" => serde_json::to_value(
            crate::subagent::AgentTreeStore::new(
                root.join("subagents").join(format!("{key}.json")),
            )
            .list()
            .await?,
        )?,
        "terminals" => {
            use crate::terminal::InteractiveTerminals;
            let retained = controls.retained.read().await.clone();
            match retained {
                Some(entry) => {
                    let entry = controls.retained(entry.run).await?;
                    return Ok(
                        json!({"run_id":entry.run,"section":section,"value":entry.manager.list().await?,"execution":"idle"}),
                    );
                }
                None => json!([]),
            }
        }
        "workflows" => serde_json::to_value(crate::workflow::discover(&workspace, None)?)?,
        "host_resources" => {
            let mut summary = crate::host_resources::inspect()?;
            if let Some(object) = summary.as_object_mut() {
                object.remove("database");
                object.remove("reservations");
            }
            summary
        }
        _ => anyhow::bail!("unknown idle control section"),
    };
    ensure!(
        serde_json::to_vec(&value)?.len() <= 1024 * 1024,
        "idle control result exceeds bounded frame"
    );
    Ok(json!({"run_id":null,"section":section,"value":value,"execution":"idle"}))
}

/// This is an executing-Voyage contract preview, not cached runtime authority.
/// Never return historical MCP names as selectable tools after suspension.
fn builtin_preflight(config: &crate::Config, policy: &crate::policy::Policy) -> Result<Value> {
    use crate::tools::Tool;
    policy.check_current()?;
    let mut tools = crate::build::builtin_tools(config);
    // Match build_tools: a live access binding retains built-ins so per-dispatch
    // policy can handle an access upgrade. Presence does not grant any action.
    if config.access_mode() == crate::config::AccessMode::ReadOnly && config.live_access.is_none() {
        tools.retain_read_only();
    }
    let mut inventory = tools.definitions();
    // Root managed voyages always install these two tools. Their constructors
    // would require persistent ownership; only their shared pure contracts belong here.
    inventory.push(crate::tools::TodoTool::builtin_definition());
    inventory.push(crate::subagent::SubagentTool::builtin_definition());
    // Managed build adds the browser after build_tools, only for an explicit offer.
    // Cloning the broker does not claim sharing or contact the human browser.
    if let Some(browser) = &config.browser {
        inventory.push(crate::tools::BrowserTool(browser.clone()).definition());
    }
    inventory.sort_by(|a, b| a.name.cmp(&b.name));
    policy.check_current()?;
    Ok(json!({
        "inventory": inventory,
        "source": "builtin_preflight",
        "refresh": "execution",
        "validation": "Execution requires admission by the live registry and current Voyage policy; presence is not permission or readiness.",
        "mcp": {
            "configured_servers": config.mcp_servers.len(),
            "discovery": "unavailable_until_active",
            "note": "No MCP server is started or contacted by idle discovery; configured tools are not inferred or cached as runnable."
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{AccessMode, McpServerConfig},
        subagent::{
            AgentBudget, AgentPolicy, ApprovalPolicy, ExecutionContext, RuntimeLimits,
            SubagentExecutor, SubagentResult, SubagentRuntime, SubagentTool,
        },
        todo::{TodoScope, TodoStore},
        tools::TodoTool,
    };

    struct NeverExecute;
    #[async_trait::async_trait]
    impl SubagentExecutor for NeverExecute {
        async fn execute(&self, _: ExecutionContext) -> Result<SubagentResult, String> {
            panic!("inventory must not execute a subagent")
        }
    }

    // Compare complete serialized contracts, not just names or a copied schema.
    #[tokio::test]
    async fn preflight_matches_managed_builtin_contracts() -> Result<()> {
        let root = tempfile::tempdir()?;
        for mode in [
            AccessMode::ReadOnly,
            AccessMode::Approval,
            AccessMode::Unrestricted,
        ] {
            for live in [false, true] {
                for vessel in [false, true] {
                    let mut config = crate::Config {
                        access: Some(mode),
                        live_access: live.then(|| Arc::new(crate::policy::LiveAccess::new(mode))),
                        ..Default::default()
                    };
                    config.vessel.enabled = vessel;
                    let resolved =
                        crate::runtime_policy::RuntimePolicy::resolve(&config, root.path())?;
                    let budget = AgentBudget {
                        max_tokens: 0,
                        max_terminals: 1,
                    };
                    let child_policy = AgentPolicy {
                        access: mode,
                        readable_roots: vec![root.path().to_owned()],
                        writable_roots: vec![root.path().to_owned()],
                        allowed_tools: Default::default(),
                        approval: ApprovalPolicy::Deny,
                        budget: budget.clone(),
                    };
                    let runtime = Arc::new(SubagentRuntime::new(
                        Arc::new(NeverExecute),
                        RuntimeLimits::default(),
                        None,
                    )?);
                    let todos = TodoTool::new(Arc::new(TodoStore::new(
                        root.path().join("must-not-be-created.json"),
                        TodoScope::workspace(root.path().to_owned()),
                    )));
                    let tools = crate::build::build_tools(
                        resolved.config(),
                        Some(SubagentTool::new(runtime, child_policy, budget)),
                        Some(todos),
                        None,
                        resolved.policy(),
                    )
                    .await?;
                    let preflight = builtin_preflight(resolved.config(), resolved.policy())?;
                    assert_eq!(
                        preflight["inventory"],
                        serde_json::to_value(tools.definitions())?
                    );
                    let write_present = tools
                        .definitions()
                        .iter()
                        .any(|tool| tool.name == "write_file");
                    assert_eq!(
                        write_present,
                        resolved.config().access_mode() != AccessMode::ReadOnly || live
                    );
                    assert!(tools.terminals().unwrap().metadata()?.is_empty());
                    assert!(!root.path().join("must-not-be-created.json").exists());
                }
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn idle_discovery_ignores_stale_registry_and_never_starts_mcp() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut config = crate::Config {
            access: Some(AccessMode::Unrestricted),
            // Runtime provider construction cannot use this credential reference.
            api_key_env: "VOYAGE_IDLE_DISCOVERY_MISSING_CREDENTIAL".into(),
            github_enabled: false,
            ..Default::default()
        };
        config.vessel.enabled = false;
        config.mcp_servers.insert(
            "not-started".into(),
            McpServerConfig {
                command: root
                    .path()
                    .join("nonexistent-mcp-executable")
                    .to_string_lossy()
                    .into_owned(),
                url: None,
                bearer_token_env: None,
                args: vec![],
                env: Default::default(),
            },
        );
        config.mcp_servers.insert(
            "not-contacted".into(),
            McpServerConfig {
                command: String::new(),
                url: Some("http://127.0.0.1:1/mcp".into()),
                bearer_token_env: Some("VOYAGE_IDLE_DISCOVERY_MISSING_MCP_CREDENTIAL".into()),
                args: vec![],
                env: Default::default(),
            },
        );
        let controls = LiveControls::default();
        let first = inspect(&controls, "tools", &config, root.path()).await?;
        *controls.inventory.write().await = Some(json!([{"name":"stale_mcp_tool"}]));
        let after_suspend = inspect(&controls, "tools", &config, root.path()).await?;
        assert_eq!(first, after_suspend);
        assert_eq!(first["execution"], "idle");
        assert_eq!(first["value"]["source"], "builtin_preflight");
        assert_eq!(first["value"]["mcp"]["configured_servers"], 2);
        assert_eq!(
            first["value"]["mcp"]["discovery"],
            "unavailable_until_active"
        );
        let names: Vec<_> = first["value"]["inventory"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"todo") && names.contains(&"subagent"));
        for absent in ["stale_mcp_tool", "vessel", "github", "browser"] {
            assert!(!names.contains(&absent));
        }
        assert!(controls.active.read().await.is_none());
        assert!(controls.retained.read().await.is_none());
        assert_eq!(std::fs::read_dir(root.path())?.count(), 0);
        assert!(serde_json::to_vec(&first)?.len() < 1024 * 1024);
        Ok(())
    }

    #[test]
    fn github_preflight_uses_execution_credential_gate_without_exposing_credentials() -> Result<()>
    {
        let root = tempfile::tempdir()?;
        for enabled in [false, true] {
            let config = crate::Config {
                github_enabled: enabled,
                ..Default::default()
            };
            let resolved = crate::runtime_policy::RuntimePolicy::resolve(&config, root.path())?;
            let value = builtin_preflight(resolved.config(), resolved.policy())?;
            let inventory = value["inventory"].as_array().unwrap();
            assert_eq!(
                inventory.iter().any(|tool| tool["name"] == "github"),
                crate::github::Credential::from_config(resolved.config()).is_some()
            );
            // This test does not mutate the process environment or require a live token.
            assert!(
                inventory
                    .iter()
                    .all(|tool| tool.get("credential").is_none())
            );
        }
        Ok(())
    }
}
