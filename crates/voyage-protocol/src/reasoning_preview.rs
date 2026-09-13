//! Display-only provider disclosures, never private replay state or an answer.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_BLOCKS: usize = 32;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningKind {
    Summary,
    Thinking,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningPreview {
    pub attempt_id: Uuid,
    pub index: usize,
    pub kind: ReasoningKind,
    /// Privacy-filtered, bounded provider-exposed text; never signatures/encrypted data.
    pub text: String,
    pub truncated: bool,
    /// Set only by the same atomic checkpoint that accepts the canonical answer.
    pub finalized: bool,
}
