//! Persistable display metadata. Never admission, liveness, or cleanup authority.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CatalogueSummary {
    pub session_id: Uuid,
    pub revision: u64,
    pub observation_cursor: u64,
    pub name: Option<String>,
    pub model: String,
    pub created_at: Option<String>,
    pub last_turn_end: Option<String>,
    pub total_messages: u64,
    pub run_id: Option<Uuid>,
    pub run_state: Option<String>,
    pub archived: bool,
    pub deleted: bool,
    pub pending_cleanup_run: Option<Uuid>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CatalogueMetadata {
    pub summary: Option<CatalogueSummary>,
    pub observed_at_ms: Option<u64>,
    pub stale: bool,
    pub error_code: Option<String>,
}
