//! Presentation provenance for messages sent by a voyage. Never execution authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CoordinationSource {
    pub vessel_id: Uuid,
    pub session_id: Uuid,
    /// Name observed when sending; navigation always uses stable identities.
    pub session_name: String,
    /// Matches the originating vessel tool call's command_id, not a message offset.
    pub command_id: Uuid,
    pub tool_call_id: String,
}

impl CoordinationSource {
    pub fn valid_for(&self, command_id: Uuid) -> bool {
        !self.tool_call_id.is_empty()
            && self.tool_call_id.len() <= 1024
            && !self.tool_call_id.chars().any(char::is_control)
            && !self.vessel_id.is_nil()
            && !self.session_id.is_nil()
            && self.command_id == command_id
            && !command_id.is_nil()
            && !self.session_name.trim().is_empty()
            && self.session_name.len() <= 256
            && !self.session_name.chars().any(char::is_control)
    }
}
