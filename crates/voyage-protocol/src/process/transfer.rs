//! Positive ownership-transfer artifacts. Keys must be pinned locally before use.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VesselIdentity {
    pub vessel_id: Uuid,
    pub public_key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedArtifact<T> {
    pub payload: T,
    pub signature: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferPreparation {
    pub transfer_id: Uuid,
    pub source_vessel_id: Uuid,
    pub destination_vessel_id: Uuid,
    pub session_id: Uuid,
    pub nonce: Uuid,
    pub workspace: PathBuf,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferManifest {
    pub transfer_id: Uuid,
    pub source_vessel_id: Uuid,
    pub destination_vessel_id: Uuid,
    pub session_id: Uuid,
    pub source_incarnation: Uuid,
    pub prepare_digest: String,
    pub artifact_sha256: String,
    pub artifact_bytes: u64,
    pub generation: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortableCheckpoint {
    pub transfer_id: Uuid,
    pub session_id: Uuid,
    pub destination_vessel_id: Uuid,
    pub prepare_digest: String,
    pub generation: u64,
    pub session: Value,
    pub commands: Vec<CommandTombstone>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandTombstone {
    pub command_id: Uuid,
    pub principal_id: Uuid,
    pub payload_sha256: Option<String>,
    pub run_id: Option<Uuid>,
    pub original_status: String,
    pub run_state: Option<String>,
}
