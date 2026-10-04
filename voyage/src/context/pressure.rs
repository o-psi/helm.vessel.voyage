//! Request-bound pressure facts. Payload size and cumulative billing are not inputs.
use serde::{Deserialize, Serialize};

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
