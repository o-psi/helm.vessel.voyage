//! Provisional display only. Never deserialize this into an executable call.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_CALLS: usize = 32;
pub const MAX_ARGUMENT_BYTES: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPreview {
    pub attempt_id: Uuid,
    pub index: usize,
    /// Correlation only, never an execution authorization.
    pub call_id: Option<String>,
    pub name: String,
    /// Decoded, privacy-filtered preview, not valid or complete JSON.
    pub arguments: String,
    pub truncated: bool,
}
