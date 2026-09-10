//! Time-based presentation, independent of executor suspension and navigation.
use super::state::View;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voyage_protocol::process::ProcessState;

pub(super) fn retention_from_env() -> anyhow::Result<u64> {
    match std::env::var("HELM_SETTLE_AFTER_SECS") {
        Err(std::env::VarError::NotPresent) => Ok(300),
        Ok(value) => parse_retention(&value),
        Err(_) => anyhow::bail!("HELM_SETTLE_AFTER_SECS must be a nonnegative integer in seconds"),
    }
}

fn parse_retention(value: &str) -> anyhow::Result<u64> {
    anyhow::ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "HELM_SETTLE_AFTER_SECS must be a nonnegative integer in seconds"
    );
    value
        .parse()
        .map_err(|_| anyhow::anyhow!("HELM_SETTLE_AFTER_SECS exceeds the supported integer range"))
}

/// Fallback for peers without canonical completion times, retained across restarts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settlement {
    run_id: Option<Uuid>,
    since: DateTime<Utc>,
}

impl View {
    fn terminal_completion(&self) -> Option<Uuid> {
        let run = self.snapshot.as_ref()?.run.as_ref()?;
        matches!(
            run.state.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted"
        )
        .then_some(run.run_id)
    }

    /// Polls, navigation, and incarnation changes do not start a new timer.
    pub(super) fn observe_settlement(&mut self, now: DateTime<Utc>) -> bool {
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        let run_id = self.terminal_completion();
        let next = if snapshot.run.is_some() && run_id.is_none() {
            None
        } else {
            let canonical = match run_id {
                Some(id) => snapshot
                    .turns
                    .iter()
                    .find(|turn| turn.run_id == id)
                    .and_then(|turn| turn.finished_at),
                None => snapshot.created_at,
            };
            let since = canonical
                .or_else(|| {
                    self.settlement
                        .as_ref()
                        .filter(|saved| saved.run_id == run_id)
                        .map(|saved| saved.since)
                })
                .unwrap_or(now);
            Some(Settlement { run_id, since })
        };
        if self.settlement == next {
            return false;
        }
        self.settlement = next;
        true
    }

    pub(super) fn sidebar_settled(&self, now: DateTime<Utc>, retention_secs: u64) -> bool {
        matches!(
            self.process.state,
            ProcessState::Live | ProcessState::Suspended
        ) && !self.archived()
            && self.error.is_none()
            && !self.connection_unavailable
            && self.pending.is_none()
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                !snapshot.recovery_pending
                    && snapshot.decisions.is_empty()
                    && snapshot.pending_cleanup_run.is_none()
                    && (snapshot.run.is_none() || self.terminal_completion().is_some())
            })
            && self.settlement.as_ref().is_some_and(|saved| {
                saved.run_id == self.terminal_completion()
                    && now
                        .signed_duration_since(saved.since)
                        .to_std()
                        .is_ok_and(|elapsed| elapsed.as_secs() >= retention_secs)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn view(state: &str, finished: Option<DateTime<Utc>>) -> View {
        let session = Uuid::new_v4();
        let run = Uuid::new_v4();
        let mut view = View::new(
            serde_json::from_value(json!({
                "session_id":session,"incarnation":Uuid::new_v4(),
                "workspace":"/tmp","state":"live"
            }))
            .unwrap(),
        );
        view.snapshot = Some(
            serde_json::from_value(json!({
                "session_id":session,"revision":1,"name":null,"model":"fixture",
                "messages":[],"run":{"run_id":run,"state":state},
                "turns":[{"run_id":run,"phase":state,"finished_at":finished,
                    "message_start":null,"message_end":null}]
            }))
            .unwrap(),
        );
        view
    }

    #[test]
    fn outcomes_settle_at_deadline_independent_of_process_and_polling() {
        let end = DateTime::from_timestamp(1_000, 0).unwrap();
        for state in ["completed", "failed", "cancelled", "interrupted"] {
            let mut view = view(state, Some(end));
            assert!(view.observe_settlement(end + chrono::Duration::seconds(200)));
            assert!(!view.sidebar_settled(end + chrono::Duration::seconds(299), 300));
            assert!(view.sidebar_settled(end + chrono::Duration::seconds(300), 300));
            view.process.state = ProcessState::Suspended;
            view.process.incarnation = Uuid::new_v4();
            assert!(!view.observe_settlement(end + chrono::Duration::seconds(400)));
            assert!(view.sidebar_settled(end + chrono::Duration::seconds(400), 300));
            assert!(view.sidebar_settled(end, 0));
            assert!(!view.sidebar_settled(end - chrono::Duration::seconds(1), 0));
        }
    }

    #[test]
    fn old_peer_timer_survives_restart_and_resets_for_new_runs() {
        let end = DateTime::from_timestamp(1_000, 0).unwrap();
        let mut first = view("completed", None);
        first.observe_settlement(end);
        let saved = serde_json::to_vec(&first.settlement).unwrap();
        let mut restarted = View::new(first.process.clone());
        restarted.snapshot = first.snapshot.clone();
        restarted.settlement = serde_json::from_slice(&saved).unwrap();
        assert!(!restarted.observe_settlement(end + chrono::Duration::seconds(400)));
        assert!(restarted.sidebar_settled(end + chrono::Duration::seconds(400), 300));
        restarted
            .snapshot
            .as_mut()
            .unwrap()
            .run
            .as_mut()
            .unwrap()
            .run_id = Uuid::new_v4();
        assert!(restarted.observe_settlement(end + chrono::Duration::seconds(400)));
        assert!(!restarted.sidebar_settled(end + chrono::Duration::seconds(400), 300));
        restarted
            .snapshot
            .as_mut()
            .unwrap()
            .run
            .as_mut()
            .unwrap()
            .state = "running".into();
        assert!(restarted.observe_settlement(end));
        assert!(restarted.settlement.is_none());
        assert!(!restarted.sidebar_settled(end, 0));
    }

    #[test]
    fn attention_and_unavailable_states_never_settle() {
        let end = DateTime::from_timestamp(1_000, 0).unwrap();
        for blocker in [
            "decision",
            "cleanup",
            "recovery",
            "error",
            "connection",
            "stopped",
            "unavailable",
            "archive",
        ] {
            let mut view = view("completed", Some(end));
            view.observe_settlement(end);
            match blocker {
                "decision" => view.snapshot.as_mut().unwrap().decisions.push(
                    serde_json::from_value(json!({
                        "decision_id":Uuid::new_v4(),"run_id":Uuid::new_v4(),
                        "incarnation":Uuid::new_v4(),"expires_at_ms":0,"request":{}
                    }))
                    .unwrap(),
                ),
                "cleanup" => {
                    view.snapshot.as_mut().unwrap().pending_cleanup_run = Some(Uuid::new_v4())
                }
                "recovery" => view.snapshot.as_mut().unwrap().recovery_pending = true,
                "error" => view.error = Some("unavailable".into()),
                "connection" => view.connection_unavailable = true,
                "stopped" => view.process.state = ProcessState::Stopped,
                "unavailable" => view.process.state = ProcessState::Unavailable,
                "archive" => view.snapshot.as_mut().unwrap().lifecycle = json!({"archived":true}),
                _ => unreachable!(),
            }
            assert!(!view.sidebar_settled(end, 0), "{blocker}");
        }
        for state in [
            "accepted",
            "running",
            "awaiting_decision",
            "cancel_requested",
            "unknown",
        ] {
            let mut view = view(state, Some(end));
            view.observe_settlement(end);
            assert!(!view.sidebar_settled(end, 0), "{state}");
        }
    }

    #[test]
    fn idle_voyage_and_missing_snapshot() {
        let end = DateTime::from_timestamp(1_000, 0).unwrap();
        let mut view = view("completed", None);
        view.snapshot.as_mut().unwrap().run = None;
        view.snapshot.as_mut().unwrap().created_at = Some(end);
        view.observe_settlement(end);
        assert!(!view.sidebar_settled(end, 300));
        view.process.state = ProcessState::Suspended;
        assert!(view.sidebar_settled(end + chrono::Duration::seconds(300), 300));
        view.snapshot = None;
        assert!(!view.sidebar_settled(end, 0));
    }

    #[test]
    fn retention_requires_integer_seconds() {
        assert_eq!(parse_retention("0").unwrap(), 0);
        assert_eq!(parse_retention("300").unwrap(), 300);
        assert_eq!(parse_retention(&u64::MAX.to_string()).unwrap(), u64::MAX);
        for invalid in [
            "",
            "-1",
            "+1",
            "1.5",
            "five",
            " 300",
            "18446744073709551616",
        ] {
            assert!(parse_retention(invalid).is_err(), "{invalid}");
        }
    }
}
