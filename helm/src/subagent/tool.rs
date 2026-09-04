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
        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| ToolError::InvalidArguments(e.to_string()))?;
        let value = match args {
            Args::Spawn {
                name,
                task,
                worktree,
            } => {
                let lease = if worktree {
                    let manager = self.worktrees.as_ref().ok_or_else(|| {
                        ToolError::Failed("workspace is not a supported Git repository".into())
                    })?;
                    Some(
                        manager
                            .create(&format!("{}-{}", name, Uuid::new_v4().simple()), "HEAD")
                            .map_err(failed)?,
                    )
                } else {
                    None
                };
                let spawned = self
                    .runtime
                    .spawn(SpawnRequest {
                        parent_id: self.parent_id,
                        name,
                        task,
                        policy: self.policy.clone(),
                        budget: self.budget.clone(),
                        worktree: lease.as_ref().map(|lease| lease.path.clone()),
                        branch: lease.as_ref().map(|lease| lease.branch.clone()),
                    })
                    .await;
                let id = match spawned {
                    Ok(id) => id,
                    Err(error) => {
                        if let (Some(manager), Some(lease)) = (&self.worktrees, &lease) {
                            let _ = manager.remove(lease);
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
                let outcome = tokio::select! {_ = context.cancellation.cancelled()=>return Err(ToolError::Cancelled), value=self.runtime.wait(AgentId(id))=>value.map_err(failed)?};
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
                let outcomes = tokio::select! {_ = context.cancellation.cancelled()=>return Err(ToolError::Cancelled), value=self.runtime.wait_many(&agent_ids)=>value.map_err(failed)?};
                json!({"ids":ids,"results":outcomes.into_iter().map(|outcome| match outcome { Ok(result) => json!({"status":"completed","result":result}), Err(error) => json!({"status":"failed","error":error}) }).collect::<Vec<_>>()})
            }
            Args::Cancel { id } => {
                self.runtime.cancel(AgentId(id)).await.map_err(failed)?;
                json!({"id":id,"cancel_requested":true})
            }
            Args::Message { id, message } => {
                self.runtime
                    .send_message(AgentId(id), message)
                    .await
                    .map_err(failed)?;
                json!({"id":id,"queued":true})
            }
            Args::FollowUp { id, message } => {
                let follow_up_id = self
                    .runtime
                    .follow_up(AgentId(id), message)
                    .await
                    .map_err(failed)?;
                json!({"id":follow_up_id,"queued":true})
            }
            Args::WorktreeStatus { id } => {
                let manager = self.manager()?;
                let lease = self.lease(AgentId(id)).await?;
                json!({"id":id,"path":lease.path,"branch":lease.branch,"clean":manager.is_clean(&lease).map_err(failed)?})
            }
            Args::WorktreeConflicts { id, other_id } => {
                let manager = self.manager()?;
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
                let manager = self.manager()?;
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
                let manager = self.manager()?;
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
                let manager = self.manager()?;
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
    fn manager(&self) -> Result<&WorktreeManager, ToolError> {
        self.worktrees
            .as_ref()
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
    match context.policy.command(&format!("git {action} {target}")) {
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
            max_runtime_secs: 30,
            max_children: 2,
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
