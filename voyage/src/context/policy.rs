//! Context decisions require request token accounting, never byte measurements.
use voyage_protocol::context_accounting::{
    ContextObservation, ContextPressure, ModelContextCapacity, RequestTokenCount,
};

#[derive(Clone, Debug, Default)]
pub struct ContextPolicy {
    /// Absolute tokens of additional headroom; zero imposes no extra margin.
    pub safety_margin: u64,
    /// Operator-selected headroom when the provider output default is unknown.
    pub output_reserve: Option<u64>,
}
impl ContextPolicy {
    pub fn observe(
        &self,
        execution_id: uuid::Uuid,
        generation: u64,
        count: RequestTokenCount,
        capacity: Option<ModelContextCapacity>,
        operator_limit: usize,
        requested_output: Option<u64>,
    ) -> ContextObservation {
        let output_reserve = requested_output
            .filter(|n| *n > 0)
            .or_else(|| {
                capacity
                    .as_ref()
                    .and_then(|c| c.default_output_tokens)
                    .filter(|n| *n > 0)
            })
            .or(self.output_reserve.filter(|n| *n > 0));
        let reserve_source = if requested_output.is_some_and(|n| n > 0) {
            "request_output_limit_upper_bound"
        } else if capacity
            .as_ref()
            .is_some_and(|c| c.default_output_tokens.is_some_and(|n| n > 0))
        {
            "provider_catalog_default"
        } else if self.output_reserve.is_some_and(|n| n > 0) {
            "operator_headroom"
        } else {
            "unknown"
        };
        let operator_limit = (operator_limit > 0).then_some(operator_limit as u64);
        let window = effective_limit(
            operator_limit,
            capacity.as_ref().and_then(|c| c.enabled_window_tokens),
        );
        let pressure = match (count.reliable_input_tokens(), window, output_reserve) {
            (Some(input), Some(window), _) if input > window => ContextPressure::PreparationNeeded,
            (Some(input), Some(window), Some(reserve)) => {
                if input
                    .checked_add(reserve)
                    .and_then(|n| n.checked_add(self.safety_margin))
                    .is_none_or(|n| n > window)
                {
                    ContextPressure::PreparationNeeded
                } else {
                    ContextPressure::WithinBudget
                }
            }
            _ => ContextPressure::Unknown,
        };
        ContextObservation {
            execution_id,
            projection_generation: generation,
            count,
            capacity,
            operator_limit,
            output_reserve,
            reserve_source: reserve_source.into(),
            safety_margin: self.safety_margin,
            pressure,
            scope: "prepared_provider_input_before_dispatch".into(),
            preparation: None,
        }
    }
}
pub fn effective_limit(local: Option<u64>, provider: Option<u64>) -> Option<u64> {
    match (local, provider) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}
/// A provider capacity is preparation advice, not an automatic admission veto.
/// Only an explicitly configured operator cap can refuse dispatch.
pub fn check_operator_limit(observation: &ContextObservation) -> Result<(), super::ContextError> {
    if let (Some(input), Some(limit)) = (
        observation.count.reliable_input_tokens(),
        observation.operator_limit,
    ) {
        let required = observation
            .output_reserve
            .and_then(|reserve| input.checked_add(reserve));
        if input > limit || required.is_some_and(|n| n > limit) {
            return Err(super::ContextError {
                input_tokens: input,
                required_tokens: required,
                limit,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voyage_protocol::context_accounting::{ContextScope, CountPrecision};
    fn count(tokens: Option<u64>) -> RequestTokenCount {
        RequestTokenCount {
            scope: ContextScope {
                model: "fixture".into(),
                transport: "fixture".into(),
                endpoint_fingerprint: None,
                account: None,
            },
            observed_at_ms: 1,
            input_tokens: tokens,
            precision: if tokens.is_some() {
                CountPrecision::ProviderExact
            } else {
                CountPrecision::Unknown
            },
            method: "fixture".into(),
            complete: tokens.is_some(),
            input_fingerprint: Some("a".repeat(64)),
            limitations: vec![],
        }
    }
    fn capacity(window: u64) -> ModelContextCapacity {
        ModelContextCapacity {
            scope: count(None).scope,
            observed_at_ms: 1,
            source: "fixture".into(),
            default_window_tokens: Some(window),
            maximum_selectable_window_tokens: Some(window * 4),
            enabled_window_tokens: Some(window),
            default_output_tokens: None,
            maximum_output_tokens: Some(200),
        }
    }
    #[test]
    fn enabled_windows_and_tokens_control_pressure() {
        let policy = ContextPolicy::default();
        let id = uuid::Uuid::new_v4();
        assert_eq!(
            policy
                .observe(id, 0, count(Some(900)), Some(capacity(1000)), 0, Some(200))
                .pressure,
            ContextPressure::PreparationNeeded
        );
        assert_eq!(
            policy
                .observe(id, 0, count(Some(900)), Some(capacity(10000)), 0, Some(200))
                .pressure,
            ContextPressure::WithinBudget
        );
        let observed = policy.observe(id, 0, count(Some(900)), Some(capacity(1000)), 0, Some(200));
        assert!(check_operator_limit(&observed).is_ok());
    }
    #[test]
    fn unknown_count_or_reserve_is_not_zero_and_does_not_veto() {
        let policy = ContextPolicy::default();
        let id = uuid::Uuid::new_v4();
        let missing = policy.observe(id, 0, count(None), Some(capacity(1000)), 1, Some(200));
        assert_eq!(missing.pressure, ContextPressure::Unknown);
        assert!(check_operator_limit(&missing).is_ok());
        let uncapped = policy.observe(id, 0, count(Some(900)), Some(capacity(1000)), 0, None);
        assert_eq!(uncapped.output_reserve, None);
        assert_eq!(uncapped.pressure, ContextPressure::Unknown);
        let explicit = policy.observe(id, 0, count(Some(900)), Some(capacity(1000)), 800, None);
        assert!(check_operator_limit(&explicit).is_err());
    }
}
