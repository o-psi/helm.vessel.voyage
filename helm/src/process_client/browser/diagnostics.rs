//! Fixed local failure observations; never receipt outcomes or error strings.
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    #[default]
    Startup,
    SocketObservation,
    ProcessObservation,
    ControlObservation,
    LeaseRenewal,
    HelperHeartbeat,
    NotificationObservation,
    ActionCompletion,
    PendingObservation,
    LocalFence,
    HelperShutdown,
}

#[derive(Default, Serialize)]
pub(super) struct Diagnostic {
    pub version: u32,
    pub failure_phase: Option<Phase>,
    pub socket_unchanged: Option<bool>,
    pub helper_stop_observed: Option<bool>,
    pub local_fence_observed: bool,
    pub helper_shutdown_observed: bool,
    // Deliberately no operation outcome: only exact dispatch receipts establish it.
}

impl Diagnostic {
    pub(super) fn summary(&self) -> String {
        let phase = self
            .failure_phase
            .and_then(|p| serde_json::to_value(p).ok())
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "none".into());
        format!(
            "Local browser diagnostic: {phase}; helper shutdown {}; action outcomes require exact receipts; no replay",
            if self.helper_shutdown_observed {
                "observed"
            } else {
                "not observed"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_schema_keeps_cleanup_distinct_from_failure_and_receipts() {
        let value = serde_json::to_value(Diagnostic {
            version: 1,
            failure_phase: Some(Phase::ActionCompletion),
            socket_unchanged: Some(true),
            helper_stop_observed: Some(false),
            local_fence_observed: true,
            helper_shutdown_observed: true,
        })
        .unwrap();
        assert_eq!(value["failure_phase"], "action_completion");
        let text = Diagnostic {
            version: 1,
            failure_phase: Some(Phase::ActionCompletion),
            helper_shutdown_observed: true,
            ..Default::default()
        }
        .summary();
        assert!(text.contains("action_completion"));
        assert!(text.contains("helper shutdown observed"));
        assert!(text.contains("require exact receipts"));
        assert_eq!(value["helper_shutdown_observed"], true);
        assert!(value.get("operation_outcome").is_none());
        assert!(value.get("error").is_none());
        assert_eq!(value.as_object().unwrap().len(), 6);
        let unknown = serde_json::to_value(Diagnostic::default()).unwrap();
        assert!(unknown["socket_unchanged"].is_null());
        assert!(unknown["helper_stop_observed"].is_null());
        for phase in [
            Phase::Startup,
            Phase::SocketObservation,
            Phase::ProcessObservation,
            Phase::ControlObservation,
            Phase::LeaseRenewal,
            Phase::HelperHeartbeat,
            Phase::NotificationObservation,
            Phase::ActionCompletion,
            Phase::PendingObservation,
            Phase::LocalFence,
            Phase::HelperShutdown,
        ] {
            let code = serde_json::to_value(phase).unwrap();
            assert!(
                code.as_str()
                    .unwrap()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '_')
            );
        }
    }
}
