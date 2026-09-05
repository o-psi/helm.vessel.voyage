use super::*;
use crate::config::{AccessMode, McpServerConfig, UnattendedApprovalMode};
fn fixture() -> (tempfile::TempDir, Config) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("workspace")).unwrap();
    std::fs::create_dir(root.path().join("helm")).unwrap();
    let config = Config {
        access: Some(AccessMode::Unrestricted),
        unattended_approval: UnattendedApprovalMode::Allow,
        env: std::collections::BTreeMap::from([
            ("EXPLICIT".into(), "global-value".into()),
            ("REMOVE".into(), "secret-canary".into()),
        ]),
        mcp_servers: std::collections::BTreeMap::from([(
            "fixture".into(),
            McpServerConfig {
                command: "unstarted".into(),
                args: vec![],
                env: std::collections::BTreeMap::from([
                    ("EXPLICIT".into(), "server-value".into()),
                    ("REMOVE".into(), "server-secret-canary".into()),
                ]),
            },
        )]),
        ..Config::default()
    };
    (root, config)
}
fn write_ceiling(root: &tempfile::TempDir, mode: AccessMode) {
    let mut rules = crate::policy_profile::Builtin::Balanced.document().rules;
    rules.access = mode;
    rules.inherit_env = vec!["PATH".into(), "EXPLICIT".into()];
    std::fs::write(
        root.path().join("helm/policy-ceiling.toml"),
        toml::to_string(&crate::policy_profile::CeilingDocument { schema: 1, rules }).unwrap(),
    )
    .unwrap();
}
fn resolve(root: &tempfile::TempDir, config: &Config) -> anyhow::Result<RuntimePolicy> {
    RuntimePolicy::resolve_with_source(
        config,
        &root.path().join("workspace"),
        Source::Test(root.path().into()),
    )
}
#[test]
fn no_ceiling_preserves_explicit_environment_precedence_without_mutating_config() {
    let (root, config) = fixture();
    let runtime = resolve(&root, &config).unwrap();
    assert_eq!(runtime.config().env, config.env);
    assert_eq!(
        runtime.config().mcp_servers["fixture"].env,
        config.mcp_servers["fixture"].env
    );
    assert_eq!(runtime.config().access_mode(), AccessMode::Unrestricted);
    assert!(!runtime.ceiling_present());
    assert!(runtime.policy().check_current().is_ok());
}
#[test]
fn actual_ceiling_filters_explicit_and_server_values_but_not_by_inherit_list() {
    let (root, config) = fixture();
    write_ceiling(&root, AccessMode::ReadOnly);
    let runtime = resolve(&root, &config).unwrap();
    assert!(!runtime.config().inherit_env.contains(&"EXPLICIT".into()));
    assert_eq!(runtime.config().env["EXPLICIT"], "global-value");
    assert_eq!(
        runtime.config().mcp_servers["fixture"].env["EXPLICIT"],
        "server-value"
    );
    assert!(!runtime.config().env.contains_key("REMOVE"));
    assert!(
        !runtime.config().mcp_servers["fixture"]
            .env
            .contains_key("REMOVE")
    );
    assert_eq!(runtime.config().access_mode(), AccessMode::ReadOnly);
    assert_eq!(
        runtime.config().unattended_approval,
        UnattendedApprovalMode::Deny
    );
    assert!(config.env.contains_key("REMOVE"));
    assert!(runtime.ceiling_present());
}
#[test]
fn changed_deleted_or_malformed_ceiling_requires_restart_before_next_turn() {
    for mode in 0..3 {
        let (root, config) = fixture();
        write_ceiling(&root, AccessMode::Approval);
        let runtime = resolve(&root, &config).unwrap();
        match mode {
            0 => write_ceiling(&root, AccessMode::ReadOnly),
            1 => std::fs::remove_file(root.path().join("helm/policy-ceiling.toml")).unwrap(),
            _ => std::fs::write(root.path().join("helm/policy-ceiling.toml"), "bad").unwrap(),
        };
        assert!(runtime.policy().check_current().is_err());
    }
}
#[test]
fn nonexistent_worktree_requires_explicit_parent_root_before_effects() {
    let (root, config) = fixture();
    let runtime = resolve(&root, &config).unwrap();
    assert!(
        runtime
            .policy()
            .check_delegated_workspace(&root.path().join("outside/new"))
            .is_err()
    );
    assert!(
        runtime
            .policy()
            .check_delegated_workspace(&root.path().join("workspace/new"))
            .is_ok()
    );
    assert!(!root.path().join("outside").exists());
}

#[test]
fn existing_config_names_and_redundant_roots_do_not_become_profile_import_errors() {
    let (root, mut config) = fixture();
    config.inherit_env = (0..150).map(|n| format!("existing-name-{n}")).collect();
    config.allow_read = vec![root.path().join("workspace/."); 150];
    config.allow_write = config.allow_read.clone();
    let runtime = resolve(&root, &config).unwrap();
    assert_eq!(runtime.config().inherit_env.len(), 150);
    assert_eq!(runtime.config().allow_read.len(), 1);
    write_ceiling(&root, AccessMode::Approval);
    let capped = resolve(&root, &config).unwrap();
    assert!(capped.config().inherit_env.is_empty());
}
#[test]
fn child_cannot_restore_parent_mode_denies_environment_or_external_workspace() {
    let (root, config) = fixture();
    write_ceiling(&root, AccessMode::ReadOnly);
    let parent = resolve(&root, &config).unwrap();
    let mut child = config.clone();
    child.deny_commands.clear();
    child.inherit_env.push("REMOVE".into());
    parent
        .policy()
        .limit_child_config(&mut child, &root.path().join("workspace"))
        .unwrap();
    assert_eq!(child.access_mode(), AccessMode::ReadOnly);
    assert_eq!(child.unattended_approval, UnattendedApprovalMode::Deny);
    assert!(child.deny_commands.contains(&"shutdown".into()));
    assert!(!child.env.contains_key("REMOVE"));
    assert!(!child.inherit_env.contains(&"REMOVE".into()));
    // A newly relaxed ceiling cannot grant more than the captured parent delegated.
    std::fs::remove_file(root.path().join("helm/policy-ceiling.toml")).unwrap();
    let fresh = resolve(&root, &child).unwrap();
    assert_eq!(fresh.config().access_mode(), AccessMode::ReadOnly);
    assert!(!fresh.config().env.contains_key("REMOVE"));
    assert!(
        parent
            .policy()
            .limit_child_config(&mut child, root.path())
            .is_err()
    );
}
#[test]
fn replaced_workspace_and_symlink_destination_fail_closed() {
    let (root, config) = fixture();
    let runtime = resolve(&root, &config).unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("workspace/escape")).unwrap();
    assert!(
        runtime
            .policy()
            .check_delegated_workspace(&root.path().join("workspace/escape/new"))
            .is_err()
    );
    std::fs::rename(root.path().join("workspace"), root.path().join("old")).unwrap();
    std::fs::create_dir(root.path().join("workspace")).unwrap();
    assert!(runtime.policy().check_current().is_err());
    assert!(
        runtime
            .policy()
            .check_delegated_workspace(&root.path().join("workspace/new"))
            .is_err()
    );
}

#[tokio::test]
async fn changed_ceiling_stops_reused_agent_before_provider_or_run_events() {
    use crate::{
        agent::{Agent, AgentError, SilentSink},
        model::{Message, ModelRequest, ModelResponse, Role, Usage},
        provider::{Provider, ProviderError},
        tools::{InteractionMode, Redactor, ToolContext, ToolRegistry, UnattendedApprover},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Count(Arc<AtomicUsize>);
    #[async_trait::async_trait]
    impl Provider for Count {
        async fn complete(
            &self,
            _: ModelRequest,
        ) -> std::result::Result<ModelResponse, ProviderError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(ModelResponse {
                message: Message::new(Role::Assistant, "first"),
                usage: Usage::default(),
            })
        }
    }
    let (root, config) = fixture();
    write_ceiling(&root, AccessMode::Approval);
    let runtime = resolve(&root, &config).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = Agent::new(
        Box::new(Count(calls.clone())),
        ToolRegistry::default(),
        ToolContext {
            completion: None,
            policy: Arc::new(runtime.policy().clone()),
            approver: Arc::new(UnattendedApprover { allow: false }),
            timeout: std::time::Duration::from_secs(1),
            max_output_bytes: 4096,
            environment: Default::default(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: uuid::Uuid::new_v4(),
            interaction: InteractionMode::Unattended,
            redactor: Arc::new(Redactor::default()),
        },
        Arc::new(SilentSink),
        "fixture".into(),
        "system".into(),
        100,
        None,
    );
    let first = agent.run(Vec::new(), "first input".into()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    write_ceiling(&root, AccessMode::ReadOnly);
    let second = agent
        .run(first.messages, "must not reach provider".into())
        .await;
    assert!(matches!(second, Err(AgentError::Policy(_))));
    let session = crate::session::Session::new(root.path().join("workspace"), "fixture".into());
    assert!(matches!(
        agent.prepare_run(&session).await,
        Err(AgentError::Policy(_))
    ));
    assert!(session.messages.is_empty() && session.completion_runs.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(matches!(
        agent.models(true).await,
        Err(AgentError::Policy(_))
    ));
}

#[test]
fn actual_child_entrypoint_rechecks_parent_and_fresh_source() {
    let (root, config) = fixture();
    write_ceiling(&root, AccessMode::ReadOnly);
    let parent = resolve(&root, &config).unwrap();
    let make_child = || {
        RuntimePolicy::resolve_child_with_source(
            &config,
            &root.path().join("workspace"),
            parent.policy(),
            Source::Test(root.path().into()),
        )
    };
    let child = make_child().unwrap();
    assert_eq!(child.config().access_mode(), AccessMode::ReadOnly);
    assert!(!child.config().env.contains_key("REMOVE"));
    write_ceiling(&root, AccessMode::Unrestricted);
    assert!(
        make_child().is_err(),
        "parent must rebuild after changed ceiling"
    );
}

#[test]
fn excluded_environment_values_keep_original_redaction_coverage() {
    let (root, config) = fixture();
    write_ceiling(&root, AccessMode::ReadOnly);
    let runtime = resolve(&root, &config).unwrap();
    let redact = crate::tools::Redactor::new(runtime.config().redact_values.clone());
    assert_eq!(redact.redact("secret-canary"), "[REDACTED]");
    assert!(
        !redact
            .redact("server-secret-canary")
            .contains("secret-canary")
    );
    assert!(config.redact_values.is_empty());
    assert!(!format!("{:?}", runtime.policy()).contains("secret-canary"));
}

#[test]
fn existing_config_exact_file_root_preserves_access_without_sibling_grant() {
    let (root, mut config) = fixture();
    let allowed = root.path().join("one-file");
    let sibling = root.path().join("sibling");
    std::fs::write(&allowed, "allowed").unwrap();
    std::fs::write(&sibling, "denied").unwrap();
    config.allow_read = vec![allowed.clone()];
    config.allow_write = vec![allowed.clone()];
    let runtime = resolve(&root, &config).unwrap();
    assert!(runtime.policy().resolve_read(&allowed).is_ok());
    assert!(runtime.policy().resolve_write(&allowed).is_ok());
    assert!(runtime.policy().resolve_read(&sibling).is_err());
    assert!(runtime.policy().resolve_write(&sibling).is_err());
}

#[test]
fn managed_git_inspection_rechecks_current_ceiling_before_spawning() {
    let (root, config) = fixture();
    write_ceiling(&root, AccessMode::ReadOnly);
    let runtime = resolve(&root, &config).unwrap();
    let workspace = root.path().join("workspace");
    std::fs::create_dir(workspace.join(".git")).unwrap();
    let manager =
        crate::subagent::WorktreeManager::new(workspace.clone(), workspace.join("worktrees"))
            .unwrap()
            .with_policy(std::sync::Arc::new(runtime.policy().clone()));
    write_ceiling(&root, AccessMode::Unrestricted);
    let error = manager
        .is_clean(&crate::subagent::WorktreeLease {
            path: workspace,
            branch: "child".into(),
        })
        .unwrap_err();
    assert!(error.to_string().contains("restart or rebuild"), "{error}");
}
