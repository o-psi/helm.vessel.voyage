//! Additive, public live-observation contract. A client must opt in to `public-v2`;
//! existing `public-v1` invalidations remain valid for older peers.
//!
//! The cursor is a retained, monotonically increasing journal position, not a
//! contiguous per-session counter. Only `replay_gap` or an incarnation change
//! requires snapshot recovery; never infer a gap from cursor arithmetic.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const PROJECTION: &str = "public-v2";
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 32 * 1024;

/// Ordered replay page. `cursor` is the last delivered position (or `after`
/// when empty); `latest_cursor` may be greater while `has_more` is true.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveEventPage {
    pub projection: String,
    pub replay_gap: bool,
    pub cursor: u64,
    pub latest_cursor: u64,
    pub has_more: bool,
    pub events: Vec<LiveEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

/// `payload` is a bounded public projection, never a raw Session, provider
/// response, tool arguments, private input or internal diagnostic.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveEvent {
    pub cursor: u64,
    pub session_id: Uuid,
    pub kind: LiveEventKind,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    pub payload: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveEventKind {
    MessageCreated,
    MessageFinalized,
    TextDelta,
    ToolActivity,
    RunState,
    Decision,
    CommandOutcome,
    Session,
    Lifecycle,
    Catalogue,
}

/// Canonical message index is stable across snapshots and history pages.
/// Oversize messages retain the existing message-chunk read route.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageUpdate {
    pub message_index: u64,
    pub message: Value,
}

/// A UTF-8 append to the provisional assistant text of this run. The offset
/// is the exact previous byte length; duplicates may be discarded and an
/// offset mismatch requires replay or snapshot, never guessed concatenation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextAppend {
    pub offset: u64,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn public_v2_fixture_round_trips_and_keeps_sparse_cursor() {
        let session_id = Uuid::nil();
        let page = LiveEventPage {
            projection: PROJECTION.into(),
            replay_gap: false,
            cursor: 14,
            latest_cursor: 20,
            has_more: true,
            recovery: None,
            events: vec![LiveEvent {
                cursor: 14,
                session_id,
                kind: LiveEventKind::TextDelta,
                revision: 4,
                run_id: Some(session_id),
                entity_id: None,
                payload: serde_json::to_value(TextAppend {
                    offset: 3,
                    text: "é".into(),
                })
                .unwrap(),
            }],
        };
        let wire = serde_json::to_value(&page).unwrap();
        assert_eq!(wire["events"][0]["payload"], json!({"offset":3,"text":"é"}));
        assert_eq!(
            serde_json::from_value::<LiveEventPage>(wire)
                .unwrap()
                .events[0]
                .cursor,
            14
        );
    }
}
