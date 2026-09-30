//! Private metadata authority bridge. Lease secrets never enter diagnostics.
use crate::process::{GrantBinding, ProcessRight};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
pub const SCHEMA: u32 = 1;
pub const MAX_FRAME: usize = 16 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionScopeHandle {
    pub socket_name: String,
    pub lease_id: Uuid,
    pub secret: String,
}
impl std::fmt::Debug for ExecutionScopeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExecutionScopeHandle([private])")
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeCheck {
    pub schema: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub token: String,
    pub handle: ExecutionScopeHandle,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeScope {
    pub schema: u32,
    pub session_id: Uuid,
    pub incarnation: Uuid,
    pub binding: GrantBinding,
    pub full_access: bool,
    pub rights: Vec<ProcessRight>,
    pub accounts: Vec<Uuid>,
    pub enrollment_connections: Vec<Uuid>,
    pub expires_at_ms: u64,
    pub workspace: PathBuf,
    pub connection_binding: Option<GrantBinding>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeReply {
    pub scope: Option<RuntimeScope>,
}
