//! Durable provider observations, not canonical messages or permission to retry.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPhase {
    Dispatch,
    Stream,
    Backoff,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryDecision {
    InFlight,
    Completed,
    RetryScheduled,
    AttemptsExhausted,
    ElapsedBudget,
    ServerDelayLimit,
    NonRetryable,
    PartialResponse,
    Cancelled,
    PolicyRevoked,
    LocalFailure,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ProviderAttempt {
    pub request_id: Uuid,
    pub attempt_id: Uuid,
    /// Executing runtime supplies bounded, redacted identifiers and an allowlisted category.
    pub provider: String,
    pub model: String,
    pub attempt: usize,
    pub limit: usize,
    pub started_at_ms: u64,
    pub duration_ms: u64,
    pub phase: AttemptPhase,
    pub category: Option<String>,
    pub http_status: Option<u16>,
    pub text_observed: bool,
    pub tool_fragment_observed: bool,
    pub retry_delay_ms: Option<u64>,
    pub decision: RetryDecision,
}

impl ProviderAttempt {
    /// Authored display text deliberately never interpolates provider-controlled strings.
    /// In-flight is an observation, not proof the request survived owner restart.
    pub fn summary(&self) -> String {
        let detail = match self.decision {
            RetryDecision::InFlight => match self.phase {
                AttemptPhase::Dispatch => "dispatch recorded; outcome not yet recorded",
                AttemptPhase::Stream => "response started; outcome not yet recorded",
                AttemptPhase::Backoff => "backoff recorded; next dispatch not yet recorded",
            },
            RetryDecision::Completed => "response completed",
            RetryDecision::RetryScheduled => "retry scheduled",
            RetryDecision::AttemptsExhausted => "attempt limit reached; try again later",
            RetryDecision::ElapsedBudget => "retry time budget reached; try again later",
            RetryDecision::ServerDelayLimit => "server wait exceeds retry limit; try again later",
            RetryDecision::NonRetryable => {
                "request cannot be retried automatically; check provider configuration"
            }
            RetryDecision::PartialResponse => {
                "partial response retained; automatic retry stopped to avoid duplicate effects; review saved output before submitting continuation"
            }
            RetryDecision::Cancelled => "cancelled; no further automatic retry",
            RetryDecision::LocalFailure => {
                "local admission, accounting, or checkpoint failed; check the executing host before retrying"
            }
            RetryDecision::PolicyRevoked => "execution policy revoked; no further automatic retry",
        };
        let phase = match self.phase {
            AttemptPhase::Dispatch => "dispatch",
            AttemptPhase::Stream => "stream",
            AttemptPhase::Backoff => "backoff",
        };
        let category = match self.category.as_deref() {
            Some("authentication") => "authentication failure",
            Some("usage_limit") => "account usage limit",
            Some("context_length") => "context rejection",
            Some("rate_limit") => "rate limit",
            Some("unavailable") => "service unavailable",
            Some("timeout") => "timeout",
            Some("transport") => "transport failure; remote outcome uncertain",
            Some("request") => "request failure",
            Some("invalid_response") => "invalid or truncated response",
            Some("incomplete") => "incomplete response",
            _ => "provider observation",
        };
        let mut summary = format!(
            "Provider attempt {}/{} · {phase} · {category} · {} · {} ms",
            self.attempt, self.limit, detail, self.duration_ms
        );
        if let Some(status) = self.http_status {
            summary.push_str(&format!(" · HTTP {status}"));
        }
        if self.decision == RetryDecision::RetryScheduled
            && let Some(delay) = self.retry_delay_ms
        {
            summary.push_str(&format!(" · wait {delay} ms"));
        }
        summary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_round_trip_and_authored_summary() {
        let value = serde_json::json!({
            "request_id": Uuid::new_v4(), "attempt_id": Uuid::new_v4(),
            "provider": "secret\u{1b}[31m", "model": "private-model", "category": "raw-diagnostic",
            "attempt": 2, "limit": 3, "started_at_ms": 100, "duration_ms": 40,
            "phase": "backoff", "http_status": 429, "text_observed": false,
            "tool_fragment_observed": false, "retry_delay_ms": 500, "decision": "retry_scheduled"
        });
        let attempt: ProviderAttempt = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(&attempt).unwrap(), value);
        let summary = attempt.summary();
        assert!(summary.contains("2/3"));
        assert!(summary.contains("HTTP 429"));
        assert!(summary.contains("wait 500 ms"));
        for forbidden in ["secret", "private-model", "raw-diagnostic", "\u{1b}"] {
            assert!(!summary.contains(forbidden));
        }
    }
    #[test]
    fn every_decision_has_stable_snake_case_encoding() {
        for (decision, wire) in [
            (RetryDecision::InFlight, "in_flight"),
            (RetryDecision::Completed, "completed"),
            (RetryDecision::RetryScheduled, "retry_scheduled"),
            (RetryDecision::AttemptsExhausted, "attempts_exhausted"),
            (RetryDecision::ElapsedBudget, "elapsed_budget"),
            (RetryDecision::ServerDelayLimit, "server_delay_limit"),
            (RetryDecision::NonRetryable, "non_retryable"),
            (RetryDecision::PartialResponse, "partial_response"),
            (RetryDecision::Cancelled, "cancelled"),
            (RetryDecision::PolicyRevoked, "policy_revoked"),
            (RetryDecision::LocalFailure, "local_failure"),
        ] {
            assert_eq!(serde_json::to_value(&decision).unwrap(), wire);
            assert_eq!(
                serde_json::from_value::<RetryDecision>(serde_json::json!(wire)).unwrap(),
                decision
            );
        }
        for (phase, wire) in [
            (AttemptPhase::Dispatch, "dispatch"),
            (AttemptPhase::Stream, "stream"),
            (AttemptPhase::Backoff, "backoff"),
        ] {
            assert_eq!(serde_json::to_value(&phase).unwrap(), wire);
        }
    }
}
