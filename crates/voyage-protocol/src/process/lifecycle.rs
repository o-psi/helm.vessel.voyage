//! Executing-host initialization provenance; canonical history never crosses Vessel.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeInitialization {
    ManagedImport {
        transfer_id: Uuid,
        source_directory: PathBuf,
        expected_revision: u64,
    },
    Participant {
        assignment_id: Uuid,
        parent_vessel_id: Uuid,
        parent_session_id: Uuid,
        parent_run_id: Uuid,
        policy: super::ParticipantPolicy,
    },
    Transfer {
        transfer_id: Uuid,
        artifact_path: PathBuf,
        sha256: String,
        prepare_digest: String,
        generation: u64,
    },
    Import {
        transfer_id: Uuid,
        source_directory: PathBuf,
        expected_revision: u64,
        source_sha256: String,
    },
    Branch {
        source_directory: PathBuf,
        source_session_id: Uuid,
        source_command_id: Uuid,
        branch_id: Uuid,
    },
}
