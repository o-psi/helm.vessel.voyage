//! Content-free operator enrollment observations, never worker authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;
pub const MAX_PAGE: usize = 128;
pub const MAX_CURSOR: usize = 2048;
fn default_limit() -> u16 {
    100
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    #[serde(default = "default_limit")]
    pub limit: u16,
    #[serde(default)]
    pub cursor: Option<String>,
    /// Initial audit bookmark only; not accepted for inventory or with a cursor.
    #[serde(default)]
    pub after: u64,
}
impl Request {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.limit == 0 || self.limit as usize > MAX_PAGE || self.after > i64::MAX as u64 {
            return Err("invalid inspection bounds");
        }
        if let Some(cursor) = &self.cursor
            && (self.after != 0
                || cursor.is_empty()
                || cursor.len() > MAX_CURSOR
                || !cursor
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')))
        {
            return Err("invalid inspection cursor");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub machine_id: Uuid,
    pub epoch: u64,
    pub revoked: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machines {
    pub version: u16,
    pub owner_id: Uuid,
    pub revision: u64,
    pub machines: Vec<Machine>,
    pub next: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditKind {
    InvitationCreated,
    Enrolled,
    Authenticated,
    Rotated,
    Revoked,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    pub sequence: u64,
    pub machine_id: Option<Uuid>,
    pub kind: AuditKind,
    pub time_ms: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Retention {
    AllRetained,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Audit {
    pub version: u16,
    pub owner_id: Uuid,
    pub through: u64,
    pub events: Vec<AuditEvent>,
    pub next: Option<String>,
    pub retention: Retention,
}
