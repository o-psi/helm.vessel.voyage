//! Durable coordination primitives for isolated child agents.
mod archive;
mod persistence;
mod runtime;
mod tool;
mod worktree;

pub use archive::{ArchivePage, ArchivedAgent};
pub use persistence::{AgentTree, AgentTreeStore};
pub use runtime::{
    ExecutionContext, InboxMessage, RuntimeError, RuntimeLimits, SpawnRequest, SubagentEvent,
    SubagentEventKind, SubagentExecutor, SubagentResult, SubagentRuntime,
};
pub use tool::SubagentTool;
pub use worktree::{ConflictReport, IntegrationPlan, WorktreeLease, WorktreeManager};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentId(pub Uuid);
impl AgentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Queued,
    Running,
    Waiting,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    TimedOut,
}
impl AgentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted | Self::TimedOut
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    Inherit,
    Ask,
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentBudget {
    pub max_tokens: u64,

    pub max_terminals: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPolicy {
    pub readable_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    pub allowed_tools: BTreeSet<String>,
    pub approval: ApprovalPolicy,
    pub budget: AgentBudget,
}
impl AgentPolicy {
    /// A child can only reduce its parent's authority and resources.
    pub fn validate_child(&self, child: &Self) -> anyhow::Result<()> {
        anyhow::ensure!(
            child.allowed_tools.is_subset(&self.allowed_tools),
            "child tool set exceeds parent policy"
        );
        anyhow::ensure!(
            child.readable_roots.iter().all(|root| self
                .readable_roots
                .iter()
                .any(|parent| root.starts_with(parent))),
            "child readable root exceeds parent policy"
        );
        anyhow::ensure!(
            child.writable_roots.iter().all(|root| self
                .writable_roots
                .iter()
                .any(|parent| root.starts_with(parent))),
            "child writable root exceeds parent policy"
        );
        anyhow::ensure!(
            child.budget.max_tokens <= self.budget.max_tokens
                && child.budget.max_terminals <= self.budget.max_terminals,
            "child budget exceeds parent budget"
        );
        anyhow::ensure!(
            !matches!(
                (&self.approval, &child.approval),
                (
                    ApprovalPolicy::Deny,
                    ApprovalPolicy::Ask | ApprovalPolicy::Inherit
                )
            ),
            "child approval policy exceeds parent"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentRecord {
    pub id: AgentId,
    pub parent_id: Option<AgentId>,
    pub name: String,
    pub task: String,
    pub status: AgentStatus,
    pub policy: AgentPolicy,
    pub budget: AgentBudget,
    pub worktree: Option<PathBuf>,
    pub branch: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    pub recent_progress: Vec<String>,
    pub result: Option<String>,
    pub error: Option<String>,
}

/// Logical processors available to this process, respecting OS affinity when supported.
pub fn default_concurrency() -> usize {
    concurrency_for_cpus(
        std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
    )
}
pub(crate) fn concurrency_for_cpus(cpus: usize) -> usize {
    (cpus / 2).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> AgentPolicy {
        AgentPolicy {
            readable_roots: vec!["/work".into()],
            writable_roots: vec!["/work/out".into()],
            allowed_tools: ["read".into(), "shell".into()].into_iter().collect(),
            approval: ApprovalPolicy::Deny,
            budget: AgentBudget {
                max_tokens: 100,

                max_terminals: 2,
            },
        }
    }
    #[test]
    fn remaining_child_budgets_cannot_exceed_parent() {
        let parent = policy();
        for field in ["max_tokens", "max_terminals"] {
            let mut encoded = serde_json::to_value(&parent).unwrap();
            let value = encoded["budget"][field].as_u64().unwrap();
            encoded["budget"][field] = serde_json::json!(value + 1);
            let child: AgentPolicy = serde_json::from_value(encoded).unwrap();
            assert!(parent.validate_child(&child).is_err(), "{field}");
        }
    }

    #[test]
    fn child_policy_can_only_reduce_authority() {
        let parent = policy();
        let mut child = policy();
        child.allowed_tools.remove("shell");
        child.readable_roots = vec!["/work/src".into()];
        child.budget.max_tokens = 50;
        assert!(parent.validate_child(&child).is_ok());
        child.allowed_tools.insert("network".into());
        assert!(parent.validate_child(&child).is_err());
    }
}
