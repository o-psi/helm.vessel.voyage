//! Minimal notification references. None of these types confer disclosure,
//! execution or decision authority. Authenticate the current recipient grant and
//! source separately on every operation, including receipt and exact retry.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_DESTINATION_LIFETIME_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const MAX_NOTIFICATION_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub const MAX_INBOX_PAGE: u32 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Completed,
    Incomplete,
    Failed,
    Cancelled,
    Interrupted,
    Attention,
    Budget,
    /// Synthetic receipt test, never evidence of a successful run or decision.
    Test,
}

/// Minutes since midnight UTC, start inclusive and end exclusive. Overnight
/// intervals wrap midnight; equal endpoints are invalid (use None to disable).
/// UTC is intentional: no local time zone or DST interpretation is permitted.
/// Quiet hours suppress attention presentation, not inbox visibility or expiry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuietHoursUtc {
    pub start_minute: u16,
    pub end_minute: u16,
}

impl QuietHoursUtc {
    pub fn valid(&self) -> bool {
        self.start_minute < 1440 && self.end_minute < 1440 && self.start_minute != self.end_minute
    }

    pub fn contains(&self, now_ms: u64) -> bool {
        if !self.valid() {
            return false;
        }
        let minute = ((now_ms / 60_000) % 1440) as u16;
        if self.start_minute < self.end_minute {
            minute >= self.start_minute && minute < self.end_minute
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }
}

/// Immutable explicit recipient/source consent. A change requires a new ID.
/// The store enforces bounds, not the grant's current validity or permissions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Destination {
    pub id: Uuid,
    pub recipient_grant_id: Uuid,
    pub recipient_principal_id: Uuid,
    pub recipient_grant_revision: u64,
    pub source_vessel_id: Uuid,
    pub source_session_id: Uuid,
    /// Nonempty, unique, at most eight kinds. Order is part of exact retry identity.
    pub event_kinds: Vec<NotificationKind>,
    pub expires_at_ms: u64,
    pub notification_ttl_ms: u64,
    pub quiet_hours_utc: Option<QuietHoursUtc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationRecord {
    pub destination: Destination,
    pub revoked_at_ms: Option<u64>,
    pub accepted_at_ms: Option<u64>,
}

/// Strict metadata only: no titles, summaries, transcripts, tool arguments,
/// URLs, credentials, provider continuation or private terminal data.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub event_id: Uuid,
    pub session_id: Uuid,
    pub run_id: Option<Uuid>,
    pub incarnation: Option<Uuid>,
    pub kind: NotificationKind,
    pub source_event_id: Uuid,
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    pub decision_id: Option<Uuid>,
    pub budget: Option<BudgetDetail>,
}

/// Opaque budget provenance only; no amounts, labels, paths or explanations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetDetail {
    pub dimension: BudgetDimension,
    pub scope: BudgetScope,
    pub scope_id: Uuid,
    pub revision: u64,
    pub threshold: BudgetThreshold,
    pub ledger_sequence: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetDimension {
    Tokens,
    EstimatedMicrocurrency,
    RuntimeMilliseconds,
    CpuMilliseconds,
    DiskReadBytes,
    DiskWriteBytes,
    NetworkBytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScope {
    Session,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetThreshold {
    Warning,
    Denied,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProducerError {
    Capacity,
    Unavailable,
    Gap,
    InvalidSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProducerCursor {
    pub after: u64,
    pub error: Option<ProducerError>,
}

/// Monotonic per-destination observation, not proof of human reading, source
/// settlement or approval. A stale interface cannot undo a later receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptState {
    Available,
    Seen,
    Dismissed,
}

/// Acceptance/receipt evidence deliberately omits the notification payload so an
/// exact publish retry can recover after expiry or revocation without disclosure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationReceipt {
    pub sequence: u64,
    pub event_id: Uuid,
    pub state: ReceiptState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxEntry {
    pub receipt: NotificationReceipt,
    pub notification: Notification,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboxPage {
    pub entries: Vec<InboxEntry>,
    /// Exclusive acceptance-sequence cursor. On an empty page, equals `after`.
    /// Receipt updates do not renumber entries: rescan from zero to refresh them.
    pub next_after: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum NotificationOperation {
    /// Current authorized unread count; quiet hours defer attention, never expiry.
    Attention,
    Configure {
        command_id: Uuid,
        destination: Destination,
    },
    Accept {
        command_id: Uuid,
        destination_id: Uuid,
    },
    Revoke {
        command_id: Uuid,
        destination_id: Uuid,
    },
    #[serde(deserialize_with = "deserialize_empty_operation")]
    Destinations,
    Test {
        command_id: Uuid,
        destination_id: Uuid,
    },
    Inbox {
        destination_id: Uuid,
        after: u64,
        limit: u32,
    },
    Receipt {
        destination_id: Uuid,
        event_id: Uuid,
        state: ReceiptState,
    },
    Open {
        destination_id: Uuid,
        event_id: Uuid,
    },
}

// Internally tagged unit variants otherwise need not consume their remaining
// map. Explicitly reject extras even on the payload-free inventory operation.
fn deserialize_empty_operation<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<(), D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    Empty::deserialize(deserializer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_quiet_hours_wrap_and_have_exact_boundaries() {
        let hours = QuietHoursUtc {
            start_minute: 23 * 60,
            end_minute: 60,
        };
        assert!(hours.contains(23 * 60 * 60_000));
        assert!(hours.contains(24 * 60 * 60_000));
        assert!(!hours.contains(60 * 60_000));
        assert!(!hours.contains(22 * 60 * 60_000));
        assert!(
            !QuietHoursUtc {
                start_minute: 0,
                end_minute: 0
            }
            .valid()
        );
        assert!(
            !QuietHoursUtc {
                start_minute: 1440,
                end_minute: 1
            }
            .valid()
        );
    }

    #[test]
    fn unknown_operation_fields_and_free_text_are_refused() {
        assert!(
            serde_json::from_str::<NotificationOperation>(
                r#"{"operation":"destinations","summary":"secret"}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<NotificationKind>(r#""arbitrary text""#).is_err());
    }
}
