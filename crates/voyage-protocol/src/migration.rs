//! Private, host-operator migration data. These are data records, never source
//! administrator authority or a generic execution command.
use crate::{execution_identity::ConfiguredExecutionIdentity, process::*};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum UserRequest {
    HoldOwner,
    OpaqueExport,
    ExportHeld,

    ReviewServices,
    Quiesce {
        definitions: std::collections::BTreeMap<String, String>,
    },
    RestoreServices {
        definitions: std::collections::BTreeMap<String, String>,
    },
    ObserveRestoredServices {
        definitions: std::collections::BTreeMap<String, String>,
    },
    Export,
    ProviderFingerprint,
    PrepareConfig {
        session_id: Uuid,
    },
    Restore {
        runtime_root: PathBuf,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema_version: u32,
    pub uid: u32,
    pub home: PathBuf,
    pub identity: serde_json::Value,
    pub sessions: Vec<ProcessRegistration>,
    pub connections: Vec<ConnectionGrant>,
    pub grants: Vec<ProcessGrant>,
    pub grant_credentials: Vec<AccessCredential>,
    pub trusted_vessels: Vec<VesselIdentity>,
    pub profiles: Option<serde_json::Value>,
    pub legacy_commands: Vec<Uuid>,
    pub pairing: Vec<u8>,
    pub provider_fingerprint: String,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Snapshot {
        snapshot: Box<Snapshot>,
    },
    File {
        session_id: Uuid,
        relative: PathBuf,
        offset: u64,
        bytes: String,
        last: bool,
    },
    End {
        sha256: String,
    },
    ReviewedServices {
        definitions: std::collections::BTreeMap<String, String>,
    },
    Quiesced {
        stop_requested: bool,
        services: Vec<String>,
        definitions: std::collections::BTreeMap<String, String>,
    },
    Config {
        sha256: String,
    },
    ProviderFingerprint {
        sha256: String,
    },
    Quiescent {
        incarnations_retired: bool,
    },
    RestoredServices {
        ready: bool,
    },
    OpaqueFile {
        relative: PathBuf,
        offset: u64,
        bytes: String,
        last: bool,
        mode: u32,
    },
    OpaqueEnd {
        sha256: String,
    },
    OwnerHeld,
    Complete,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub operation_id: Uuid,
    pub review_digest: String,
    pub source_boot: Uuid,
    pub target_boot: Uuid,
    pub identity: ConfiguredExecutionIdentity,
    pub snapshot: Snapshot,
    pub executable: PathBuf,
    pub target_incarnations: std::collections::BTreeMap<Uuid, Uuid>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlRequest {
    Import { import: Box<Import> },
    Quiescence { drain: bool },
}
