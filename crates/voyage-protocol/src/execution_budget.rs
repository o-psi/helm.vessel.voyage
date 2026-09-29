//! Deny-only limits for one delegated run. These never grant execution rights.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBudget {
    /// Exact delegated Submit command, also the parent's allocation identity.
    pub command_id: Uuid,
    pub session_id: Uuid,
    pub parent_session_id: Uuid,
    pub parent_run_id: Uuid,
    pub tokens: u64,
    pub elapsed_ms: u64,
    /// Parent deadline; receiving hosts also clamp their local elapsed limit.
    pub expires_at_ms: u64,
}
impl ExecutionBudget {
    pub fn valid_for(&self, command: Uuid) -> bool {
        self.command_id == command
            && !command.is_nil()
            && !self.session_id.is_nil()
            && self.session_id != self.parent_session_id
            && !self.parent_session_id.is_nil()
            && !self.parent_run_id.is_nil()
            && (1..=10_000_000).contains(&self.tokens)
            && (1..=86_400_000).contains(&self.elapsed_ms)
            && self.expires_at_ms > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionUsage {
    pub budget: ExecutionBudget,
    pub session_id: Uuid,
    pub run_id: Uuid,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub elapsed_ms: u64,
    /// Every provider/subordinate request has a final aggregate usage report.
    pub complete: bool,
    /// Separate from successful completion and from a human cleanup attestation.
    pub cleanup_observed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budget_is_finite_and_bound_to_one_command() {
        let b = ExecutionBudget {
            command_id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            tokens: 1,
            elapsed_ms: 1,
            expires_at_ms: 1000,
        };
        assert!(b.valid_for(b.command_id));
        assert!(!b.valid_for(Uuid::new_v4()));
        for bad in [
            ExecutionBudget {
                tokens: 0,
                ..b.clone()
            },
            ExecutionBudget {
                tokens: 10_000_001,
                ..b.clone()
            },
            ExecutionBudget {
                elapsed_ms: 0,
                ..b.clone()
            },
            ExecutionBudget {
                elapsed_ms: 86_400_001,
                ..b.clone()
            },
            ExecutionBudget {
                parent_session_id: Uuid::nil(),
                ..b.clone()
            },
        ] {
            assert!(!bad.valid_for(b.command_id));
        }
    }
    #[test]
    fn goal_budget_submit_omits_absent_extension_and_roundtrips_present_budget() {
        let id = Uuid::new_v4();
        let legacy = serde_json::json!({"op":"submit","command_id":id,"expected_revision":0,"expires_at_ms":2000,"prompt":"child"});
        let command: crate::vessel::VoyageCommand = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), legacy);
        let budget = ExecutionBudget {
            command_id: id,
            session_id: Uuid::new_v4(),
            parent_session_id: Uuid::new_v4(),
            parent_run_id: Uuid::new_v4(),
            tokens: 100,
            elapsed_ms: 1000,
            expires_at_ms: 2000,
        };
        let mut bounded = legacy;
        bounded["budget"] = serde_json::to_value(&budget).unwrap();
        let command: crate::vessel::VoyageCommand =
            serde_json::from_value(bounded.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), bounded);
        bounded["budget"]["continue_automatically"] = serde_json::json!(true);
        assert!(serde_json::from_value::<crate::vessel::VoyageCommand>(bounded).is_err());
    }
}
