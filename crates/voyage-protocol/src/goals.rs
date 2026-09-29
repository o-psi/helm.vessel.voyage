//! Persistent, user-authored task data. Goal state never grants tool authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalLimits {
    pub runs: u32,
    pub tokens: u64,
    pub elapsed_ms: u64,
    pub no_progress_runs: u32,
}

impl Default for GoalLimits {
    fn default() -> Self {
        Self {
            runs: 20,
            tokens: 200_000,
            elapsed_ms: 3_600_000,
            no_progress_runs: 3,
        }
    }
}

impl GoalLimits {
    pub fn valid(&self) -> bool {
        (1..=1_000).contains(&self.runs)
            && (1..=10_000_000).contains(&self.tokens)
            && (1_000..=86_400_000).contains(&self.elapsed_ms)
            && (1..=10).contains(&self.no_progress_runs)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalUsage {
    pub runs: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
    pub no_progress_runs: u32,
    /// A known lower bound is retained, but missing aggregate usage prevents resume.
    #[serde(default)]
    pub unmeasured_runs: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Complete,
    Blocked,
    Limited,
    NeedsAttention,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStopReason {
    UserPaused,
    RunLimit,
    TokenLimit,
    TimeLimit,
    NoProgress,
    Interrupted,
    Cancelled,
    AuthorityRevoked,
    ApprovalRequired,
    ProviderFailure,
    UsageUnknown,
    UnresolvedEffects,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Goal {
    pub id: Uuid,
    pub session_id: Uuid,
    pub objective: String,
    pub status: GoalStatus,
    pub continuation_authorized: bool,
    pub limits: GoalLimits,
    pub usage: GoalUsage,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub stop_reason: Option<GoalStopReason>,
}

impl Goal {
    pub fn limit_reached(&self) -> Option<GoalStopReason> {
        if self.usage.runs >= self.limits.runs {
            Some(GoalStopReason::RunLimit)
        } else if self
            .usage
            .input_tokens
            .saturating_add(self.usage.output_tokens)
            >= self.limits.tokens
        {
            Some(GoalStopReason::TokenLimit)
        } else if self.usage.elapsed_ms >= self.limits.elapsed_ms {
            Some(GoalStopReason::TimeLimit)
        } else if self.usage.no_progress_runs >= self.limits.no_progress_runs {
            Some(GoalStopReason::NoProgress)
        } else {
            None
        }
    }
}

/// Revision survives clearing, so an old command cannot replace newer work.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalSnapshot {
    pub revision: u64,
    pub goal: Option<Goal>,
}

/// Only authenticated human control accepts these actions. Model completion and
/// blocked reports use a separate evidence-checked path, never this owner API.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum GoalAction {
    Set {
        objective: String,
        #[serde(default)]
        limits: GoalLimits,
        replace_goal_id: Option<Uuid>,
        continue_automatically: bool,
    },
    Edit {
        goal_id: Uuid,
        objective: String,
        limits: GoalLimits,
    },
    Pause {
        goal_id: Uuid,
    },
    Resume {
        goal_id: Uuid,
    },
    Clear {
        goal_id: Uuid,
    },
}

pub fn valid_objective(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= 8192
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_and_objectives_are_finite_and_bounded() {
        assert!(GoalLimits::default().valid());
        let mut limits = GoalLimits::default();
        limits.tokens = 0;
        assert!(!limits.valid());
        assert!(!valid_objective("\n\t"));
        assert!(!valid_objective("task\u{1b}[31m"));
        assert!(!valid_objective(&"a".repeat(8193)));
        assert!(valid_objective("Read the file.\nCheck the result."));
    }

    #[test]
    fn model_or_observer_cannot_acquire_goal_control_through_execute_scope() {
        use crate::process::{
            ProcessRight, RuntimeCommand, owner_connection_right, required_process_right,
        };
        let command = RuntimeCommand::GoalUpdate {
            command_id: Uuid::new_v4(),
            expected_revision: 0,
            expires_at_ms: 1000,
            action: GoalAction::Set {
                objective: "task".into(),
                limits: GoalLimits::default(),
                replace_goal_id: None,
                continue_automatically: true,
            },
        };
        assert_eq!(required_process_right(&command), None);
        assert_eq!(
            owner_connection_right(&command),
            Some(ProcessRight::Execute)
        );
        assert_eq!(
            required_process_right(&RuntimeCommand::GoalRead),
            Some(ProcessRight::History)
        );
        let wrapped = RuntimeCommand::Resolve {
            command_id: command.mutation_id().unwrap(),
            original: Some(Box::new(command)),
        };
        assert_eq!(required_process_right(&wrapped), None);
        assert_eq!(
            owner_connection_right(&wrapped),
            Some(ProcessRight::Execute)
        );
    }
}
