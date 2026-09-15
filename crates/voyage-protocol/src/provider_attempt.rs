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
    ContinuationScheduled,
    RecoveryInterrupted,
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
    #[serde(default)]
    pub retry: RetryObservation,
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

/// Disjoint final JSON byte buckets; not tokens, charge, or proof of dispatch.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestBytes {
    pub total: u64,
    pub instructions: u64,
    pub schemas: u64,
    pub history: u64,
    pub envelope: u64,
}

/// Numeric/allowlisted facts only; absent legacy observations remain unknown.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RetryObservation {
    /// Exact encoded model-request bytes. None means not observed, not zero.
    pub request_bytes: Option<RequestBytes>,
    /// Previous interrupted attempt retained in history before a new request.
    pub recovery_of: Option<Uuid>,
    /// Absolute deadline for admitting further attempts; not a live-process lease.
    pub recovery_deadline_at_ms: Option<u64>,
    pub upstream_request_id: Option<String>,
    pub response_timeout_ms: Option<u64>,
    pub stream_idle_ms: Option<u64>,
    pub server_delay_ms: Option<u64>,
    pub max_delay_ms: Option<u64>,
    pub max_elapsed_ms: Option<u64>,
    pub elapsed_ms: Option<u64>,
    pub backoff_ms: Option<u64>,
    pub eligible: bool,
    pub failure_phase: Option<AttemptPhase>,
    pub provider_code: Option<String>,
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
            RetryDecision::RetryScheduled => "reconnection retry scheduled",
            RetryDecision::ContinuationScheduled => {
                "partial response retained; history-based continuation scheduled"
            }
            RetryDecision::RecoveryInterrupted => {
                "recovery interrupted; previous request is not active; review saved outcomes before continuing"
            }
            RetryDecision::AttemptsExhausted => "attempt limit reached; try again later",
            RetryDecision::ElapsedBudget => "retry time budget reached; try again later",
            RetryDecision::ServerDelayLimit => "server wait exceeds retry limit; try again later",
            RetryDecision::NonRetryable => {
                "request cannot be retried automatically; check provider configuration"
            }
            RetryDecision::PartialResponse => {
                "partial response retained; failure does not permit automatic continuation; review saved output before continuing"
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
            Some("transport_timeout") => "connection timeout",
            Some("stream_interrupted") => "response stream interrupted",
            Some("connection") => "connection establishment failed",
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
        if matches!(
            self.decision,
            RetryDecision::RetryScheduled | RetryDecision::ContinuationScheduled
        ) && let Some(delay) = self.retry_delay_ms
        {
            summary.push_str(&format!(" · wait {delay} ms"));
        }
        if let Some(wait) = self.retry.server_delay_ms {
            summary.push_str(&format!(" · server wait {wait} ms"));
        }
        if let Some(timeout) = self.retry.response_timeout_ms {
            summary.push_str(&format!(" · response-start limit {timeout} ms"));
        }
        if let Some(idle) = self.retry.stream_idle_ms {
            summary.push_str(&format!(" · stream idle limit {idle} ms"));
        }
        if let Some(max) = self.retry.max_delay_ms {
            summary.push_str(&format!(" · retry delay limit {max} ms"));
        }
        if let Some(max) = self.retry.max_elapsed_ms {
            summary.push_str(&format!(" · retry window {max} ms"));
        }
        if let (Some(max), Some(elapsed)) = (self.retry.max_elapsed_ms, self.retry.elapsed_ms) {
            summary.push_str(&format!(
                " · recovery budget at observation {} ms",
                max.saturating_sub(elapsed)
            ));
        }
        if self.retry.recovery_of.is_some() {
            summary.push_str(" · continues an earlier interrupted response");
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
        let mut attempt: ProviderAttempt = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(
            serde_json::from_value::<ProviderAttempt>(serde_json::to_value(&attempt).unwrap())
                .unwrap(),
            attempt
        );
        let summary = attempt.summary();
        assert!(summary.contains("2/3"));
        assert!(summary.contains("HTTP 429"));
        assert!(summary.contains("wait 500 ms"));
        for forbidden in ["secret", "private-model", "raw-diagnostic", "\u{1b}"] {
            assert!(!summary.contains(forbidden));
        }
        attempt.category = Some("transport_timeout".into());
        attempt.retry.response_timeout_ms = Some(60_000);
        attempt.retry.stream_idle_ms = Some(300_000);
        attempt.retry.max_delay_ms = Some(30_000);
        let summary = attempt.summary();
        assert!(attempt.retry.recovery_of.is_none());
        assert!(attempt.retry.recovery_deadline_at_ms.is_none());
        assert!(summary.contains("connection timeout"));
        assert!(summary.contains("response-start limit 60000 ms"));
        assert!(summary.contains("stream idle limit 300000 ms"));
        assert!(summary.contains("retry delay limit 30000 ms"));
        attempt.decision = RetryDecision::ContinuationScheduled;
        attempt.category = Some("stream_interrupted".into());
        attempt.retry.recovery_of = Some(Uuid::new_v4());
        attempt.retry.max_elapsed_ms = Some(120_000);
        attempt.retry.elapsed_ms = Some(5_000);
        let summary = attempt.summary();
        assert!(summary.contains("history-based continuation scheduled"));
        assert!(summary.contains("wait 500 ms"));
        assert!(summary.contains("recovery budget at observation 115000 ms"));
        assert!(summary.contains("response stream interrupted"));
        attempt.decision = RetryDecision::RecoveryInterrupted;
        assert!(attempt.summary().contains("previous request is not active"));
    }
    #[test]
    fn every_decision_has_stable_snake_case_encoding() {
        for (decision, wire) in [
            (RetryDecision::InFlight, "in_flight"),
            (RetryDecision::Completed, "completed"),
            (RetryDecision::RetryScheduled, "retry_scheduled"),
            (
                RetryDecision::ContinuationScheduled,
                "continuation_scheduled",
            ),
            (RetryDecision::RecoveryInterrupted, "recovery_interrupted"),
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
