//! Private, root-pipe-only retired journal handoff. These records are not public
//! authority and never contain conversation text. Target configuration remains
//! private and is read only in the target identity after protected publication.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
pub const SCHEMA: u32 = 1;
pub const MAX_REQUEST: usize = 64 * 1024;
pub const MAX_RESPONSE: usize = 64 * 1024;
pub const MAX_TARGET_CONFIG: usize = 64 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionRequest {
    pub schema: u32,
    pub session_id: Uuid,
    pub source_incarnation: Uuid,
    pub operation: TransitionOperation,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitionOperation {
    Observe {
        directory: PathBuf,
    },
    RetainConfiguration {
        directory: PathBuf,
    },
    SourceFreeze {
        directory: PathBuf,
        command_id: Uuid,
        transition_id: Uuid,
        target_incarnation: Uuid,
        expected: RetiredJournalFacts,
        target_uid: u32,
        target_gid: u32,
        target_config_digest: String,
        review_digest: String,
    },
    TargetCommit {
        directory: PathBuf,
        command_id: Uuid,
        expected: PreparedTransitionReceipt,
        target_config_path: PathBuf,
    },
    AbortSource {
        directory: PathBuf,
        command_id: Uuid,
        expected: PreparedTransitionReceipt,
    },
    Lookup {
        directory: PathBuf,
        command_id: Uuid,
    },
}
impl TransitionOperation {
    pub fn directory(&self) -> &std::path::Path {
        match self {
            Self::Observe { directory }
            | Self::RetainConfiguration { directory }
            | Self::SourceFreeze { directory, .. }
            | Self::TargetCommit { directory, .. }
            | Self::Lookup { directory, .. }
            | Self::AbortSource { directory, .. } => directory,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetiredJournalFacts {
    pub session_id: Uuid,
    pub source_incarnation: Uuid,
    pub revision: u64,
    pub frozen_config_digest: String,
    pub history_digest: String,
    pub pending_work_digest: String,
    pub unresolved_cleanup: Vec<Uuid>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedTransitionReceipt {
    pub command_id: Uuid,
    pub transition_id: Uuid,
    pub session_id: Uuid,
    pub source_incarnation: Uuid,
    pub target_incarnation: Uuid,
    pub source_uid: u32,
    pub source_gid: u32,
    pub source_directory_device: u64,
    pub source_directory_inode: u64,
    pub target_uid: u32,
    pub target_gid: u32,
    pub previous_revision: u64,
    pub prepared_revision: u64,
    pub previous_config_digest: String,
    pub target_config_digest: String,
    pub review_digest: String,
    pub history_digest: String,
    pub pending_work_digest: String,
    pub retained_pending_work_digest: String,
    pub unresolved_cleanup: Vec<Uuid>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionReceipt {
    pub command_id: Uuid,
    pub prepared: PreparedTransitionReceipt,
    pub resulting_revision: u64,
    pub config_digest: String,
    pub history_digest: String,
    pub pending_work_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AbortedTransitionReceipt {
    pub command_id: Uuid,
    pub prepared: PreparedTransitionReceipt,
    pub resulting_revision: u64,
    pub history_digest: String,
    pub pending_work_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransitionResponse {
    Facts { facts: RetiredJournalFacts },
    Configuration { path: PathBuf, digest: String },
    Prepared { receipt: PreparedTransitionReceipt },
    Committed { receipt: TransitionReceipt },
    Aborted { receipt: AbortedTransitionReceipt },
    Unavailable,
}
