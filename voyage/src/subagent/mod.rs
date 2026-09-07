//! Durable coordination primitives for isolated child agents.
mod archive;
mod history;
mod history_storage;
pub use history::{HistoryCursor, HistoryNotice, HistoryReplay, HistoryStatus};
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
    /// Optional per-response output limit; zero means no Helm-imposed cap.
    pub max_tokens: u64,

    pub max_terminals: u32,
}

impl AgentBudget {
    pub(crate) fn allows(&self, child: &Self) -> bool {
        (self.max_tokens == 0 || (child.max_tokens > 0 && child.max_tokens <= self.max_tokens))
            && child.max_terminals <= self.max_terminals
    }

    /// Intersect explicit limits without treating an absent limit as zero output.
    pub fn response_limit(&self, configured: u32) -> u32 {
        let delegated = self.max_tokens.min(u32::MAX as u64) as u32;
        match (configured, delegated) {
            (0, limit) | (limit, 0) => limit,
            (configured, delegated) => configured.min(delegated),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPolicy {
    /// Captured immediate-parent access ceiling; legacy records fail closed.
    #[serde(default = "legacy_access_ceiling")]
    pub access: crate::config::AccessMode,
    pub readable_roots: Vec<PathBuf>,
    pub writable_roots: Vec<PathBuf>,
    pub allowed_tools: BTreeSet<String>,
    pub approval: ApprovalPolicy,
    pub budget: AgentBudget,
}
fn legacy_access_ceiling() -> crate::config::AccessMode {
    crate::config::AccessMode::ReadOnly
}
impl AgentPolicy {
    pub(crate) fn limit_access(&mut self, mode: crate::config::AccessMode) {
        self.access = Self::restrict_access(self.access, mode);
    }
    fn restrict_access(
        a: crate::config::AccessMode,
        b: crate::config::AccessMode,
    ) -> crate::config::AccessMode {
        use crate::config::AccessMode::*;
        match (a, b) {
            (ReadOnly, _) | (_, ReadOnly) => ReadOnly,
            (Approval, _) | (_, Approval) => Approval,
            _ => Unrestricted,
        }
    }
    /// A child can only reduce its parent's authority and resources.
    pub fn validate_child(&self, child: &Self) -> anyhow::Result<()> {
        anyhow::ensure!(
            Self::restrict_access(self.access, child.access) == child.access,
            "child access exceeds parent policy"
        );
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
            self.budget.allows(&child.budget),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<crate::completion::runtime::RunReference>,
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
