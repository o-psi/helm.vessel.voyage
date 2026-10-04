//! Request-bound pressure facts. Payload size and cumulative billing are not inputs.
use serde::{Deserialize, Serialize};

/// Explicit policy, in tokens. Zero means no additional policy reserve/margin;
/// absent target uses the enabled capacity, never an invented percentage.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PressurePolicy {
    pub enabled_capacity: Option<u64>,
    pub reserve_tokens: Option<u64>,
    pub safety_tokens: u64,
    pub target_tokens: Option<u64>,
}
impl PressurePolicy {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.enabled_capacity == Some(0) || self.target_tokens == Some(0) {
            return Err("capacity and target must be positive when configured");
        }
        if let (Some(capacity), Some(target)) = (self.enabled_capacity, self.target_tokens) {
            if target > capacity {
                return Err("context target exceeds enabled capacity");
            }
        }
        Ok(())
    }
    pub fn apply(&self, pressure: &mut RequestPressure) {
        pressure.enabled_capacity = match (self.enabled_capacity, pressure.enabled_capacity) {
            (Some(local), Some(remote)) => Some(local.min(remote)),
            (Some(local), None) => Some(local),
            (None, remote) => remote,
        };
        pressure.reserve_tokens = match (self.reserve_tokens, pressure.reserve_tokens) {
            (Some(local), Some(remote)) => Some(local.max(remote)),
            (Some(local), None) => Some(local),
            (None, remote) => remote,
        };
        pressure.safety_tokens = pressure.safety_tokens.max(self.safety_tokens);
        // Target occupancy is an explicit token threshold; the capacity report
        // remains enabled capacity. Trigger comparisons use a separate target.
    }
    pub fn needs_reduction(&self, pressure: &RequestPressure) -> bool {
        let mut compared = pressure.clone();
        if let (Some(target), Some(capacity)) = (self.target_tokens, compared.enabled_capacity) {
            compared.enabled_capacity = Some(target.min(capacity));
        }
        compared.should_prepare()
    }
}

/// Supplied only by an adapter that counts its final encoded request, including
/// instructions, tools, replay and modalities. Partial counts must remain unknown.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RequestPressure {
    pub enabled_capacity: Option<u64>,
    pub input_tokens: Option<u64>,
    pub complete: bool,
    pub method: String,
    /// Includes response/reasoning headroom; None means provider defaults unknown.
    pub reserve_tokens: Option<u64>,
    pub safety_tokens: u64,
}

impl RequestPressure {
    pub fn remaining(&self) -> Option<u64> {
        if !self.complete || self.enabled_capacity == Some(0) {
            return None;
        }
        Some(
            self.enabled_capacity?
                .saturating_sub(self.input_tokens?)
                .saturating_sub(self.reserve_tokens?)
                .saturating_sub(self.safety_tokens),
        )
    }

    pub fn should_prepare(&self) -> bool {
        if !self.complete {
            return false;
        }
        match (
            self.enabled_capacity,
            self.input_tokens,
            self.reserve_tokens,
        ) {
            (Some(capacity), Some(input), Some(reserve)) if capacity > 0 => {
                input
                    .saturating_add(reserve)
                    .saturating_add(self.safety_tokens)
                    >= capacity
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn facts(capacity: u64, tokens: u64) -> RequestPressure {
        RequestPressure {
            enabled_capacity: Some(capacity),
            input_tokens: Some(tokens),
            complete: true,
            method: "offline exact fixture".into(),
            reserve_tokens: Some(100),
            safety_tokens: 50,
        }
    }
    #[test]
    fn capacity_and_tokens_determine_pressure() {
        assert!(facts(1000, 900).should_prepare());
        assert!(!facts(10000, 900).should_prepare());
        assert!(!facts(1000, 800).should_prepare());
        assert_eq!(facts(1000, 800).remaining(), Some(50));
    }
    #[test]
    fn zero_capacity_and_unbounded_reserve_do_not_wrap() {
        assert!(!facts(0, 900).should_prepare());
        let mut f = facts(u64::MAX, u64::MAX - 1);
        f.reserve_tokens = Some(u64::MAX);
        assert!(f.should_prepare());
        assert_eq!(f.remaining(), Some(0));
    }
    #[test]
    fn incomplete_or_unknown_is_not_pressure() {
        let mut f = facts(1000, 900);
        f.complete = false;
        assert!(!f.should_prepare());
        assert_eq!(f.remaining(), None);
        f.complete = true;
        f.reserve_tokens = None;
        assert!(!f.should_prepare());
        assert_eq!(f.remaining(), None);
        f.reserve_tokens = Some(100);
        f.input_tokens = None;
        assert!(!f.should_prepare());
        assert_eq!(f.remaining(), None);
        f.input_tokens = Some(900);
        f.enabled_capacity = None;
        assert!(!f.should_prepare());
        assert_eq!(f.remaining(), None);
    }
}

#[cfg(test)]
mod policy_tests {
    use super::*;
    fn facts() -> RequestPressure {
        RequestPressure {
            enabled_capacity: Some(1000),
            input_tokens: Some(500),
            complete: true,
            method: "offline oracle".into(),
            reserve_tokens: Some(100),
            safety_tokens: 0,
        }
    }
    #[test]
    fn explicit_target_and_reserve_do_not_change_capacity_or_invent_unknown() {
        let policy = PressurePolicy {
            enabled_capacity: Some(900),
            reserve_tokens: Some(200),
            safety_tokens: 50,
            target_tokens: Some(700),
        };
        assert!(policy.validate().is_ok());
        let mut p = facts();
        policy.apply(&mut p);
        assert_eq!(p.enabled_capacity, Some(900));
        assert_eq!(p.reserve_tokens, Some(200));
        assert!(policy.needs_reduction(&p));
        p.input_tokens = None;
        assert!(!policy.needs_reduction(&p));
        assert_eq!(p.remaining(), None);
    }
    #[test]
    fn narrower_remote_capacity_and_larger_reserve_win() {
        let policy = PressurePolicy {
            enabled_capacity: Some(2000),
            reserve_tokens: Some(50),
            ..Default::default()
        };
        let mut p = facts();
        policy.apply(&mut p);
        assert_eq!(p.enabled_capacity, Some(1000));
        assert_eq!(p.reserve_tokens, Some(100));
        assert!(!policy.needs_reduction(&p));
    }
    #[test]
    fn invalid_targets_and_unknown_reserve_remain_explicit() {
        assert!(
            PressurePolicy {
                enabled_capacity: Some(0),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            PressurePolicy {
                enabled_capacity: Some(100),
                target_tokens: Some(200),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        let mut p = facts();
        p.reserve_tokens = None;
        PressurePolicy::default().apply(&mut p);
        assert!(!p.should_prepare());
        let restored: PressurePolicy = serde_json::from_str("{\"safety_tokens\":50}").unwrap();
        assert_eq!(restored.enabled_capacity, None);
    }
}
