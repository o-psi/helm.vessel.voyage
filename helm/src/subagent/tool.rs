use super::{AgentBudget, AgentId, AgentPolicy, SpawnRequest, SubagentRuntime, WorktreeManager};
use crate::{
    model::ToolDefinition,
    policy::Decision,
    tools::{Tool, ToolContext, ToolError},
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

/// Model-facing control surface. Policy and budgets are supplied by Helm, never by the model.
#[derive(Clone)]
pub struct SubagentTool {
    runtime: Arc<SubagentRuntime>,
    policy: AgentPolicy,
    budget: AgentBudget,
    parent_id: Option<AgentId>,
    worktrees: Option<WorktreeManager>,
}
impl SubagentTool {
    pub fn new(runtime: Arc<SubagentRuntime>, policy: AgentPolicy, budget: AgentBudget) -> Self {
        Self {
            runtime,
            policy,
            budget,
            parent_id: None,
            worktrees: None,
        }
    }
    pub fn with_parent(mut self, parent_id: AgentId) -> Self {
        self.parent_id = Some(parent_id);
        self
    }
    pub fn with_worktrees(mut self, worktrees: Option<WorktreeManager>) -> Self {
        self.worktrees = worktrees;
        self
    }
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum Args {
    Spawn {
        name: String,
        task: String,
        #[serde(default)]
        worktree: bool,
    },
    Status {
        id: Uuid,
    },
    List {
        #[serde(default)]
        parent_id: Option<Uuid>,
    },
    Archive {
        #[serde(default)]
        after: Option<Uuid>,
        #[serde(default = "archive_limit")]
        limit: usize,
    },
    Wait {
        id: Uuid,
    },
    WaitMany {
        ids: Vec<Uuid>,
    },
    Cancel {
        id: Uuid,
    },
    Message {
        id: Uuid,
        message: String,
    },
    FollowUp {
        id: Uuid,
        message: String,
    },
    WorktreeStatus {
        id: Uuid,
    },
    WorktreeConflicts {
        id: Uuid,
        other_id: Uuid,
    },
    Integrate {
        id: Uuid,
        target: String,
    },
    Commit {
        id: Uuid,
        message: String,
    },
    Cleanup {
        id: Uuid,
    },
}

fn archive_limit() -> usize {
    20
}

#[async_trait]
impl Tool for SubagentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
        name:"subagent".into(), description:"Spawn and supervise bounded background Helm agents. Spawn independent agents before waiting. Set worktree=true for isolated Git coding work. Worktree commit, integration, and cleanup are guarded and refuse dirty or conflicting changes. Finished agents archive automatically. Use archive (after/limit pagination) to find old IDs, then status or wait to read their results. Archived records cannot be restarted; use spawn for new work. Actions: spawn, status, list, archive, wait, wait_many, cancel, message, follow_up, worktree_status, worktree_conflicts, commit, integrate, cleanup.".into(),
        input_schema:json!({"type":"object","required":["action"],"properties":{"action":{"enum":["spawn","status","list","archive","wait","wait_many","cancel","message","follow_up","worktree_status","worktree_conflicts","commit","integrate","cleanup"]},"id":{"type":"string","format":"uuid"},"ids":{"type":"array","items":{"type":"string","format":"uuid"},"minItems":1},"other_id":{"type":"string","format":"uuid"},"parent_id":{"type":"string","format":"uuid"},"after":{"type":"string","format":"uuid"},"limit":{"type":"integer","minimum":1,"maximum":100},"name":{"type":"string"},"task":{"type":"string"},"message":{"type":"string"},"worktree":{"type":"boolean"},"target":{"type":"string"}},"additionalProperties":false}),
    }
    }
    async fn execute(&self, arguments: Value, context: &ToolContext) -> Result<String, ToolError> {
        context.policy.check_current().map_err(failed)?;
        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let value = match args {
            Args::Spawn {
                name,
                task,
                worktree,
            } => {
                let lease = if worktree {
                    let manager = self.manager(context)?;
                    let worktree_name = format!("{}-{}", name, Uuid::new_v4().simple());
                    let destination = manager.planned_path(&worktree_name).map_err(failed)?;
                    context
                        .policy
                        .check_delegated_workspace(&destination)
                        .map_err(failed)?;
                    let command = manager
                        .create_command(&worktree_name, "HEAD")
                        .map_err(failed)?;
                    approve_git_command(context, "subagent.create", &worktree_name, &command)
                        .await?;
                    context.policy.check_current().map_err(failed)?;
                    Some(manager.create(&worktree_name, "HEAD").map_err(failed)?)
                } else {
                    None
                };
                let spawned = self
                    .runtime
                    .spawn_for_run(
                        SpawnRequest {
                            parent_id: self.parent_id,
                            name,
                            task,
                            policy: self.policy.clone(),
                            budget: self.budget.clone(),
                            worktree: lease.as_ref().map(|lease| lease.path.clone()),
                            branch: lease.as_ref().map(|lease| lease.branch.clone()),
                        },
                        context.completion.clone(),
                    )
                    .await;
                let id = match spawned {
                    Ok(id) => id,
                    Err(error) => {
                        if let (Some(manager), Some(lease)) = (&self.worktrees, &lease) {
                            let _ = manager
                                .clone()
                                .with_policy(context.policy.clone())
                                .remove(lease);
                        }
                        return Err(failed(error));
                    }
                };
                json!({"id":id,"status":"queued"})
            }
            Args::Status { id } => {
                let mut value =
                    serde_json::to_value(self.runtime.get(AgentId(id)).await.map_err(failed)?)
                        .map_err(json_error)?;
                value["archived"] = json!(
                    self.runtime
                        .is_archived(AgentId(id))
                        .await
                        .map_err(failed)?
                );
                value
            }
            Args::List { parent_id } => serde_json::to_value(match parent_id {
                Some(id) => self.runtime.tree(Some(AgentId(id))).await,
                None => self.runtime.list().await,
            })
            .map_err(json_error)?,
            Args::Archive { after, limit } => serde_json::to_value(
                self.runtime
                    .list_archived(after.map(AgentId), limit)
                    .await
                    .map_err(failed)?,
            )
            .map_err(json_error)?,
            Args::Wait { id } => {
                let ids = [AgentId(id)];
                let outcome = tokio::select! {_ = context.cancellation.cancelled()=>return Err(ToolError::Cancelled), value=self.runtime.wait_many_as(self.parent_id, &ids)=>value.map_err(failed)?.remove(0)};
                match outcome {
                    Ok(result) => json!({"status":"completed","result":result}),
                    Err(error) => json!({"status":"failed","error":error}),
                }
            }
            Args::WaitMany { ids } => {
                if ids.is_empty() {
                    return Err(ToolError::InvalidArguments(
                        "ids must contain at least one subagent".into(),
                    ));
                }
                let agent_ids: Vec<_> = ids.iter().copied().map(AgentId).collect();
                let outcomes = tokio::select! {_ = context.cancellation.cancelled()=>return Err(ToolError::Cancelled), value=self.runtime.wait_many_as(self.parent_id, &agent_ids)=>value.map_err(failed)?};
                json!({"ids":ids,"results":outcomes.into_iter().map(|outcome| match outcome { Ok(result) => json!({"status":"completed","result":result}), Err(error) => json!({"status":"failed","error":error}) }).collect::<Vec<_>>()})
            }
            Args::Cancel { id } => {
                self.runtime.cancel(AgentId(id)).await.map_err(failed)?;
                json!({"id":id,"cancel_requested":true})
            }
            Args::Message { id, message } => {
                self.runtime
                    .send_message_as(self.parent_id, AgentId(id), message)
                    .await
                    .map_err(failed)?;
                json!({"id":id,"queued":true})
            }
            Args::FollowUp { id, message } => {
                let follow_up_id = self
                    .runtime
                    .follow_up_in_run(
                        self.parent_id,
                        AgentId(id),
                        message,
                        context.completion.clone(),
                    )
                    .await
                    .map_err(failed)?;
                json!({"id":follow_up_id,"queued":true})
            }
            Args::WorktreeStatus { id } => {
                let manager = self.manager(context)?;
                let lease = self.lease(AgentId(id)).await?;
                json!({"id":id,"path":lease.path,"branch":lease.branch,"clean":manager.is_clean(&lease).map_err(failed)?})
            }
            Args::WorktreeConflicts { id, other_id } => {
                let manager = self.manager(context)?;
                let left = self.lease(AgentId(id)).await?;
                let right = self.lease(AgentId(other_id)).await?;
                let report = manager.conflicts(&left, &right).map_err(failed)?;
                json!({"id":id,"other_id":other_id,"files":report.files})
            }
            Args::Integrate { id, target } => {
                approve_git(context, "subagent.integrate", &target).await?;
                let _mutation = self.runtime.mutation_guard().await;
                self.runtime
                    .worktree_record(AgentId(id))
                    .await
                    .map_err(failed)?;
                let manager = self.manager(context)?;
                let lease = self.lease(AgentId(id)).await?;
                let plan = manager.plan_integration(&lease, &target).map_err(failed)?;
                manager.integrate(&lease, &target).map_err(failed)?;
                json!({"id":id,"source":plan.source,"target":plan.target,"merge_base":plan.merge_base,"integrated":true})
            }
            Args::Commit { id, message } => {
                approve_git(context, "subagent.commit", &id.to_string()).await?;
                let _mutation = self.runtime.mutation_guard().await;
                self.runtime
                    .worktree_record(AgentId(id))
                    .await
                    .map_err(failed)?;
                let manager = self.manager(context)?;
                let lease = self.lease(AgentId(id)).await?;
                let commit = manager.commit(&lease, &message).map_err(failed)?;
                json!({"id":id,"branch":lease.branch,"commit":commit})
            }
            Args::Cleanup { id } => {
                approve_git(context, "subagent.cleanup", &id.to_string()).await?;
                let _mutation = self.runtime.mutation_guard().await;
                self.runtime
                    .worktree_record(AgentId(id))
                    .await
                    .map_err(failed)?;
                let manager = self.manager(context)?;
                let lease = self.lease(AgentId(id)).await?;
                manager.remove(&lease).map_err(failed)?;
                self.runtime
                    .clear_worktree_locked(AgentId(id))
                    .await
                    .map_err(failed)?;
                json!({"id":id,"removed":true,"branch_preserved":lease.branch})
            }
        };
        serde_json::to_string(&value).map_err(json_error)
    }
}
impl SubagentTool {
    fn manager(&self, context: &ToolContext) -> Result<WorktreeManager, ToolError> {
        self.worktrees
            .as_ref()
            .map(|manager| manager.clone().with_policy(context.policy.clone()))
            .ok_or_else(|| ToolError::Failed("workspace is not a supported Git repository".into()))
    }

    async fn lease(&self, id: AgentId) -> Result<super::WorktreeLease, ToolError> {
        let record = self.runtime.get(id).await.map_err(failed)?;
        match (record.worktree, record.branch) {
            (Some(path), Some(branch)) => Ok(super::WorktreeLease { path, branch }),
            _ => Err(ToolError::Failed(format!(
                "subagent {id} does not own a worktree"
            ))),
        }
    }
}

async fn approve_git(context: &ToolContext, action: &str, target: &str) -> Result<(), ToolError> {
    approve_git_command(context, action, target, &format!("git {action} {target}")).await
}
async fn approve_git_command(
    context: &ToolContext,
    action: &str,
    target: &str,
    command: &str,
) -> Result<(), ToolError> {
    match context.policy.command(command) {
        Decision::Deny(reason) => Err(ToolError::Denied(reason)),
        Decision::Ask(reason)
            if !context
                .approver
                .approve(&context.approval(action, target, reason.clone()))
                .await
                .approved() =>
        {
            Err(ToolError::Denied("user declined approval".into()))
        }
        _ => Ok(()),
    }
}
fn failed(error: impl std::fmt::Display) -> ToolError {
    ToolError::Failed(error.to_string())
}
fn json_error(error: serde_json::Error) -> ToolError {
    ToolError::Failed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definition_exposes_complete_worktree_lifecycle() {
        assert!(matches!(
            serde_json::from_value::<Args>(json!({
                "action": "spawn",
                "name": "worker",
                "task": "change one file",
                "worktree": true
            }))
            .unwrap(),
            Args::Spawn { worktree: true, .. }
        ));
        assert!(matches!(
            serde_json::from_value::<Args>(json!({
                "action": "commit",
                "id": Uuid::nil(),
                "message": "child change"
            }))
            .unwrap(),
            Args::Commit { .. }
        ));
        assert!(matches!(
            serde_json::from_value::<Args>(json!({
                "action": "integrate",
                "id": Uuid::nil(),
                "target": "main"
            }))
            .unwrap(),
            Args::Integrate { .. }
        ));
        assert!(matches!(
            serde_json::from_value::<Args>(json!({
                "action": "cleanup",
                "id": Uuid::nil()
            }))
            .unwrap(),
            Args::Cleanup { .. }
        ));
    }
    #[test]
    fn archive_arguments_require_uuid_cursor_and_unsigned_limit() {
        assert!(matches!(
            serde_json::from_value::<Args>(json!({"action":"archive"})).unwrap(),
            Args::Archive {
                after: None,
                limit: 20
            }
        ));
        for args in [
            json!({"action":"archive","after":"../../elsewhere"}),
            json!({"action":"archive","limit":-1}),
            json!({"action":"archive","limit":"20"}),
            json!({"action":"status","id":"../escape"}),
        ] {
            assert!(serde_json::from_value::<Args>(args).is_err());
        }
    }

    struct GatedExecutor(Arc<tokio::sync::Notify>);
    #[async_trait]
    impl super::super::SubagentExecutor for GatedExecutor {
        async fn execute(
            &self,
            _: super::super::ExecutionContext,
        ) -> Result<super::super::SubagentResult, String> {
            self.0.notified().await;
            Ok(super::super::SubagentResult {
                summary: "evidence".into(),
            })
        }
    }
    #[tokio::test]
    async fn denied_external_worktree_has_no_git_or_runtime_effects() {
        use std::{
            collections::{BTreeMap, BTreeSet},
            time::Duration,
        };
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).unwrap();
        // An invalid repository makes any attempted Git call fail differently.
        std::fs::create_dir(workspace.join(".git")).unwrap();
        let destination = root.path().join("external/worktrees");
        let manager = WorktreeManager::new(workspace.clone(), destination.clone()).unwrap();
        let runtime = Arc::new(
            SubagentRuntime::new(
                Arc::new(GatedExecutor(Arc::new(tokio::sync::Notify::new()))),
                super::super::RuntimeLimits::default(),
                None,
            )
            .unwrap(),
        );
        let budget = AgentBudget {
            max_tokens: 100,
            max_terminals: 1,
        };
        let policy = AgentPolicy {
            readable_roots: vec![workspace.clone()],
            writable_roots: vec![workspace.clone()],
            allowed_tools: BTreeSet::new(),
            approval: super::super::ApprovalPolicy::Deny,
            budget: budget.clone(),
        };
        let config = crate::Config {
            access: Some(crate::config::AccessMode::Unrestricted),
            ..crate::Config::default()
        };
        let context = ToolContext {
            github: None,
            completion: None,
            policy: Arc::new(crate::policy::Policy::new(&config, workspace).unwrap()),
            approver: Arc::new(crate::tools::UnattendedApprover { allow: true }),
            timeout: Duration::from_secs(2),
            max_output_bytes: 1024,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Unattended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        };
        let tool = SubagentTool::new(runtime.clone(), policy, budget).with_worktrees(Some(manager));
        let error = tool
            .execute(
                json!({"action":"spawn","name":"child","task":"nothing","worktree":true}),
                &context,
            )
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("explicit parent read and write root delegation"),
            "{error}"
        );
        assert!(!destination.exists());
        assert!(runtime.list().await.is_empty());
        let permitted_root = root.path().join("permitted");
        std::fs::create_dir(&permitted_root).unwrap();
        let manager = WorktreeManager::new(
            context.policy.workspace().into(),
            permitted_root.join("new-root"),
        )
        .unwrap();
        let mut config = config;
        config.allow_read = vec![permitted_root.clone()];
        config.allow_write = vec![permitted_root.clone()];
        for denied in ["worktree", "add"] {
            config.deny_commands = vec![denied.into()];
            let mut denied_context = context.clone();
            denied_context.policy = Arc::new(
                crate::policy::Policy::new(&config, context.policy.workspace().into()).unwrap(),
            );
            let tool = tool.clone().with_worktrees(Some(manager.clone()));
            let error = tool
                .execute(
                    json!({"action":"spawn","name":"child","task":"nothing","worktree":true}),
                    &denied_context,
                )
                .await
                .unwrap_err();
            assert!(error.to_string().contains("denied by policy"), "{error}");
            assert!(!permitted_root.join("new-root").exists());
            assert!(runtime.list().await.is_empty());
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn restarted_archived_lease_denies_status_and_preserves_cleanup_evidence() {
        use std::{
            collections::{BTreeMap, BTreeSet},
            os::unix::fs::PermissionsExt,
            time::Duration,
        };
        for redirected in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let workspace = root.path().join("workspace");
            let external = root.path().join("external");
            std::fs::create_dir(&workspace).unwrap();
            std::fs::create_dir(&external).unwrap();
            std::fs::create_dir(workspace.join(".git")).unwrap();
            let lease_path = if redirected {
                let path = workspace.join("old-link");
                std::os::unix::fs::symlink(&external, &path).unwrap();
                path
            } else {
                external.clone()
            };
            let gate = Arc::new(tokio::sync::Notify::new());
            let executor = Arc::new(GatedExecutor(gate.clone()));
            let store = super::super::AgentTreeStore::new(root.path().join("tree.json"));
            let runtime = Arc::new(
                SubagentRuntime::new_persistent(
                    executor.clone(),
                    super::super::RuntimeLimits::default(),
                    store.clone(),
                )
                .await
                .unwrap(),
            );
            let budget = AgentBudget {
                max_tokens: 100,
                max_terminals: 1,
            };
            let policy = AgentPolicy {
                readable_roots: vec![root.path().into()],
                writable_roots: vec![root.path().into()],
                allowed_tools: BTreeSet::new(),
                approval: super::super::ApprovalPolicy::Deny,
                budget: budget.clone(),
            };
            let id = runtime
                .spawn(SpawnRequest {
                    parent_id: None,
                    name: "old".into(),
                    task: "old task".into(),
                    policy: policy.clone(),
                    budget: budget.clone(),
                    worktree: Some(lease_path.clone()),
                    branch: Some("agents/old".into()),
                })
                .await
                .unwrap();
            gate.notify_one();
            tokio::time::timeout(Duration::from_secs(5), runtime.wait(id))
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            drop(runtime);
            let runtime = Arc::new(
                SubagentRuntime::new_persistent(
                    executor,
                    super::super::RuntimeLimits::default(),
                    store.clone(),
                )
                .await
                .unwrap(),
            );
            assert_eq!(
                runtime.get(id).await.unwrap().status,
                super::super::AgentStatus::Completed
            );
            let before = std::fs::read(root.path().join("tree.json")).unwrap();
            let config = crate::Config {
                access: Some(crate::config::AccessMode::Unrestricted),
                ..crate::Config::default()
            };
            let context = ToolContext {
                github: None,
                completion: None,
                policy: Arc::new(crate::policy::Policy::new(&config, workspace.clone()).unwrap()),
                approver: Arc::new(crate::tools::UnattendedApprover { allow: true }),
                timeout: Duration::from_secs(2),
                max_output_bytes: 1024,
                environment: BTreeMap::new(),
                cancellation: tokio_util::sync::CancellationToken::new(),
                execution_id: Uuid::new_v4(),
                interaction: crate::tools::InteractionMode::Unattended,
                redactor: Arc::new(crate::tools::Redactor::default()),
            };
            let bin = root.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            let marker = root.path().join("git-started");
            let git = bin.join("git");
            std::fs::write(
                &git,
                format!(
                    "#!/bin/sh\nprintf started > '{}'\nexit 0\n",
                    marker.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700)).unwrap();
            let manager = WorktreeManager::new(workspace, root.path().into())
                .unwrap()
                .with_environment(BTreeMap::from([(
                    "PATH".into(),
                    bin.to_str().unwrap().into(),
                )]));
            let tool =
                SubagentTool::new(runtime.clone(), policy, budget).with_worktrees(Some(manager));
            for action in ["worktree_status", "cleanup"] {
                let error = tool
                    .execute(json!({"action":action,"id":id}), &context)
                    .await
                    .unwrap_err();
                if action == "worktree_status" {
                    assert!(
                        error.to_string().contains("outside allowed roots"),
                        "{redirected}/{action}: {error}"
                    );
                } else {
                    // Restart archived this terminal record. Its earlier immutable-evidence
                    // guard must still refuse mutation rather than pretending cleanup ran.
                    assert!(error.to_string().contains("unknown subagent"), "{error}");
                }
                assert!(!marker.exists(), "denied old lease must never start Git");
                assert_eq!(
                    runtime.get(id).await.unwrap().worktree,
                    Some(lease_path.clone())
                );
                assert_eq!(
                    std::fs::read(root.path().join("tree.json")).unwrap(),
                    before
                );
                assert!(external.exists());
            }
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn retained_child_cleanup_requires_current_write_roots_and_keeps_store() {
        use std::{
            collections::{BTreeMap, BTreeSet},
            os::unix::fs::PermissionsExt,
            time::Duration,
        };
        struct ChildFinishes;
        #[async_trait]
        impl super::super::SubagentExecutor for ChildFinishes {
            async fn execute(
                &self,
                context: super::super::ExecutionContext,
            ) -> Result<super::super::SubagentResult, String> {
                if context.task == "parent" {
                    std::future::pending::<()>().await;
                }
                Ok(super::super::SubagentResult {
                    summary: "done".into(),
                })
            }
        }
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let external = root.path().join("external");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::create_dir(&external).unwrap();
        std::fs::create_dir(workspace.join(".git")).unwrap();
        let store_path = root.path().join("tree.json");
        let runtime = Arc::new(
            SubagentRuntime::new_persistent(
                Arc::new(ChildFinishes),
                super::super::RuntimeLimits {
                    // This scenario keeps its parent live while the child finishes.
                    // Machine defaults may grant only one slot on small runners.
                    max_concurrency: 2,
                    ..super::super::RuntimeLimits::default()
                },
                super::super::AgentTreeStore::new(store_path.clone()),
            )
            .await
            .unwrap(),
        );
        let budget = AgentBudget {
            max_tokens: 100,
            max_terminals: 1,
        };
        let policy = AgentPolicy {
            readable_roots: vec![root.path().into()],
            writable_roots: vec![root.path().into()],
            allowed_tools: BTreeSet::new(),
            approval: super::super::ApprovalPolicy::Deny,
            budget: budget.clone(),
        };
        let parent = runtime
            .spawn(SpawnRequest {
                parent_id: None,
                name: "parent".into(),
                task: "parent".into(),
                policy: policy.clone(),
                budget: budget.clone(),
                worktree: None,
                branch: None,
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while runtime.get(parent).await.unwrap().status != super::super::AgentStatus::Running {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let child = runtime
            .spawn(SpawnRequest {
                parent_id: Some(parent),
                name: "child".into(),
                task: "child".into(),
                policy: policy.clone(),
                budget: budget.clone(),
                worktree: Some(external.clone()),
                branch: Some("agents/child".into()),
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), runtime.wait(child))
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!runtime.is_archived(child).await.unwrap());
        let before = std::fs::read(&store_path).unwrap();
        let config = crate::Config {
            access: Some(crate::config::AccessMode::Unrestricted),
            allow_read: vec![external.clone()],
            ..crate::Config::default()
        };
        let context = ToolContext {
            github: None,
            completion: None,
            policy: Arc::new(crate::policy::Policy::new(&config, workspace.clone()).unwrap()),
            approver: Arc::new(crate::tools::UnattendedApprover { allow: true }),
            timeout: Duration::from_secs(2),
            max_output_bytes: 1024,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Unattended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        };
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let marker = root.path().join("git-started");
        let git = bin.join("git");
        std::fs::write(
            &git,
            format!(
                "#!/bin/sh\nprintf started > '{}'\nexit 0\n",
                marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o700)).unwrap();
        let manager = WorktreeManager::new(workspace, root.path().into())
            .unwrap()
            .with_environment(BTreeMap::from([(
                "PATH".into(),
                bin.to_str().unwrap().into(),
            )]));
        let tool = SubagentTool::new(runtime.clone(), policy, budget).with_worktrees(Some(manager));
        // Explicit read-only delegation permits inspection, but does not permit removal.
        tool.execute(json!({"action":"worktree_status","id":child}), &context)
            .await
            .unwrap();
        assert!(marker.exists());
        std::fs::remove_file(&marker).unwrap();
        let error = tool
            .execute(json!({"action":"cleanup","id":child}), &context)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("write outside allowed roots"),
            "{error}"
        );
        assert!(!marker.exists());
        assert_eq!(
            runtime.get(child).await.unwrap().worktree,
            Some(external.clone())
        );
        assert_eq!(std::fs::read(&store_path).unwrap(), before);
        assert!(external.exists());
        runtime.cancel(parent).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), runtime.wait(parent))
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
    }
    struct PendingApproval {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait]
    impl crate::tools::Approver for PendingApproval {
        async fn approve(
            &self,
            _: &crate::tools::ApprovalRequest,
        ) -> crate::tools::ApprovalOutcome {
            self.entered.notify_one();
            self.release.notified().await;
            crate::tools::ApprovalOutcome::Approved
        }
    }

    #[tokio::test]
    async fn pending_git_approval_does_not_block_completion_and_rechecks_archive() {
        use std::{
            collections::{BTreeMap, BTreeSet},
            time::Duration,
        };
        let directory = tempfile::tempdir().unwrap();
        let gate = Arc::new(tokio::sync::Notify::new());
        let runtime = Arc::new(
            SubagentRuntime::new(
                Arc::new(GatedExecutor(gate.clone())),
                super::super::RuntimeLimits::default(),
                None,
            )
            .unwrap(),
        );
        let budget = AgentBudget {
            max_tokens: 100,

            max_terminals: 1,
        };
        let policy = AgentPolicy {
            readable_roots: vec![directory.path().into()],
            writable_roots: vec![],
            allowed_tools: BTreeSet::new(),
            approval: super::super::ApprovalPolicy::Deny,
            budget: budget.clone(),
        };
        let id = runtime
            .spawn(SpawnRequest {
                parent_id: None,
                name: "worker".into(),
                task: "work".into(),
                policy: policy.clone(),
                budget: budget.clone(),
                worktree: None,
                branch: None,
            })
            .await
            .unwrap();
        let approver = Arc::new(PendingApproval {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let config = crate::config::Config {
            access: Some(crate::config::AccessMode::Approval),
            ..crate::config::Config::default()
        };
        let context = ToolContext {
            github: None,
            completion: None,
            policy: Arc::new(crate::policy::Policy::new(&config, directory.path().into()).unwrap()),
            approver: approver.clone(),
            timeout: Duration::from_secs(2),
            max_output_bytes: 1024,
            environment: BTreeMap::new(),
            cancellation: tokio_util::sync::CancellationToken::new(),
            execution_id: Uuid::new_v4(),
            interaction: crate::tools::InteractionMode::Attended,
            redactor: Arc::new(crate::tools::Redactor::default()),
        };
        let tool = SubagentTool::new(runtime.clone(), policy, budget);
        let operation = tokio::spawn(async move {
            tool.execute(json!({"action":"cleanup","id":id}), &context)
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), approver.entered.notified())
            .await
            .unwrap();
        gate.notify_one();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), runtime.wait(id))
                .await
                .expect("approval must not hold runtime mutation lock")
                .unwrap()
                .unwrap()
                .summary,
            "evidence"
        );
        assert!(runtime.is_archived(id).await.unwrap());
        approver.release.notify_one();
        let error = operation.await.unwrap().unwrap_err().to_string();
        assert!(
            error.contains("unknown subagent"),
            "must reject archived record before worktree effects: {error}"
        );
    }
}
