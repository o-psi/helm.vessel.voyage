//! Vessel-owned unsent composition. Saving never creates a session or admits work.
//! Image references here address the draft staging namespace until explicit promotion.
use crate::content::ContentPart;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftTarget {
    NewChat {
        workspace: PathBuf,
    },
    Message {
        session_id: Uuid,
    },
    Steer {
        session_id: Uuid,
        run_id: Uuid,
        incarnation: Uuid,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DraftDocument {
    pub target: DraftTarget,
    pub parts: Vec<ContentPart>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub draft_id: Uuid,
    pub revision: u64,
    pub document: DraftDocument,
    pub updated_at_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftOperation {
    List {},
    Get {
        draft_id: Uuid,
    },
    Put {
        command_id: Uuid,
        draft_id: Uuid,
        expected_revision: u64,
        document: DraftDocument,
    },
    Delete {
        command_id: Uuid,
        draft_id: Uuid,
        expected_revision: u64,
    },
    UploadImage {
        command_id: Uuid,
        draft_id: Uuid,
        name: String,
        data_base64: String,
    },
    ReadImage {
        draft_id: Uuid,
        attachment_id: Uuid,
        offset: u64,
        limit: u32,
    },
    /// Copies immutable staged rasters through the session owner's UploadImage.
    /// Does not submit work, change target or delete the draft. Caller must separately
    /// observe admission and Put empty parts at the sent revision; a conflict preserves later edits.
    /// Delete is explicit discard, removing staging and tombstoning the draft identity.
    Promote {
        command_id: Uuid,
        draft_id: Uuid,
        expected_revision: u64,
        session_id: Uuid,
    },
}
impl DraftOperation {
    pub fn mutation_id(&self) -> Option<Uuid> {
        match self {
            Self::Put { command_id, .. }
            | Self::Delete { command_id, .. }
            | Self::UploadImage { command_id, .. }
            | Self::Promote { command_id, .. } => Some(*command_id),
            _ => None,
        }
    }
}

// Upload bytes and draft prose must not enter diagnostics.
impl std::fmt::Debug for DraftOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DraftOperation")
            .field("variant", &std::mem::discriminant(self))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_json_roundtrip_and_unknown_fields_refuse() {
        let id = Uuid::new_v4();
        let wire = serde_json::json!({"op":"drafts","operation":{"op":"put","command_id":Uuid::new_v4(),"draft_id":id,"expected_revision":0,"document":{"target":{"type":"new_chat","workspace":"/workspace"},"parts":[{"type":"text","text":""}]}}});
        let command: crate::vessel::VesselCommand = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(command).unwrap(), wire);
        let mut bad = wire;
        bad["operation"]["unexpected"] = true.into();
        assert!(serde_json::from_value::<crate::vessel::VesselCommand>(bad).is_err());
        let upload = DraftOperation::UploadImage {
            command_id: Uuid::new_v4(),
            draft_id: id,
            name: "secret label".into(),
            data_base64: "private image bytes".into(),
        };
        let debug = format!("{upload:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("private"));
    }
}
