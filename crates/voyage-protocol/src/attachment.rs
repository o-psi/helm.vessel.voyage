//! Replacement attachment domain contract, not the retired v1 worker protocol.
//!
//! Decoding validates syntax/bounds only. The receiver MUST bind machine/principal
//! to the authenticated channel and enforce current local policy and sharing.
//! Never send a raw canonical Helm Session or provider continuation in a frame.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const VERSION: u16 = 2;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_PROMPT_BYTES: usize = 64 * 1024;
pub const MAX_COMMAND_LIFETIME_MS: i64 = 300_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharingMode {
    None,
    Metadata,
    LiveEvents,
    FullTranscript,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    ViewMetadata,
    ViewLive,
    ViewHistory,
    CreateSession,
    SubmitTurn,
    CancelOwn,
    CancelAny,
    RenameSession,
    ChangeModel,
    BranchSession,
    ArchiveSession,
    DeleteSession,
    ChangeSharing,
    ApproveWrite,
    ApproveCommand,
    AdministerEnrollment,
}

/// Authenticated transport context and principal scope remain distinct from these
/// claimed identities. A valid JSON envelope does not grant any capability.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub version: u16,
    pub connection_id: Uuid,
    pub machine_id: Uuid,
    pub principal_id: Uuid,
    pub command_id: Uuid,
    pub expires_at_ms: i64,
    pub operation: Operation,
}

/// Debug output intentionally omits prompts, names, model IDs and other content.
impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttachmentCommand")
            .field("version", &self.version)
            .field("command_id", &self.command_id)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    List {
        after: Option<Uuid>,
        limit: u16,
    },
    Create {
        workspace_id: Uuid,
        name: String,
    },
    Inspect {
        session_id: Uuid,
    },
    Submit {
        session_id: Uuid,
        expected_revision: u64,
        prompt: String,
    },
    Cancel {
        session_id: Uuid,
        run_id: Uuid,
    },
    Rename {
        session_id: Uuid,
        expected_revision: u64,
        name: String,
    },
    ChangeModel {
        session_id: Uuid,
        expected_revision: u64,
        model: String,
    },
    Branch {
        session_id: Uuid,
        expected_revision: u64,
        name: String,
    },
    Archive {
        session_id: Uuid,
        expected_revision: u64,
        archived: bool,
    },
    Delete {
        session_id: Uuid,
        expected_revision: u64,
        confirmation_id: Uuid,
    },
    SetSharing {
        session_id: Uuid,
        expected_revision: u64,
        mode: SharingMode,
        confirmation_id: Uuid,
    },
    /// The request digest binds immutable local full arguments, never redacted
    /// presentation text. Receiver additionally checks epoch/principal/expiry.
    DecideApproval {
        session_id: Uuid,
        run_id: Uuid,
        approval_id: Uuid,
        request_digest: String,
        approve: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    Oversized,
    Malformed,
    UnsupportedVersion,
    InvalidIdentity,
    InvalidBounds,
    InvalidDeadline,
}
impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "attachment frame rejected: {self:?}")
    }
}
impl std::error::Error for DecodeError {}

impl Command {
    /// Bounds bytes before JSON allocation and returns content-free diagnostics.
    /// It deliberately does not echo serde's untrusted field/value error text.
    pub fn decode(bytes: &[u8], now_ms: i64) -> Result<Self, DecodeError> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(DecodeError::Oversized);
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| DecodeError::Malformed)?;
        value.validate(now_ms)?;
        Ok(value)
    }

    pub fn validate(&self, now_ms: i64) -> Result<(), DecodeError> {
        if self.version != VERSION {
            return Err(DecodeError::UnsupportedVersion);
        }
        ids(&[
            self.connection_id,
            self.machine_id,
            self.principal_id,
            self.command_id,
        ])?;
        let max = now_ms
            .checked_add(MAX_COMMAND_LIFETIME_MS)
            .ok_or(DecodeError::InvalidDeadline)?;
        if now_ms < 0 || self.expires_at_ms <= now_ms || self.expires_at_ms > max {
            return Err(DecodeError::InvalidDeadline);
        }
        use Operation::*;
        match &self.operation {
            List { after, limit } => {
                if *limit == 0 || *limit > 100 {
                    return Err(DecodeError::InvalidBounds);
                }
                if let Some(id) = after {
                    ids(&[*id])?;
                }
            }
            Create { workspace_id, name } => {
                ids(&[*workspace_id])?;
                label(name)?;
            }
            Inspect { session_id } => ids(&[*session_id])?,
            Submit {
                session_id,
                expected_revision,
                prompt,
            } => {
                target(*session_id, *expected_revision)?;
                if prompt.trim().is_empty()
                    || prompt.len() > MAX_PROMPT_BYTES
                    || prompt.contains('\0')
                {
                    return Err(DecodeError::InvalidBounds);
                }
            }
            Cancel { session_id, run_id } => ids(&[*session_id, *run_id])?,
            Rename {
                session_id,
                expected_revision,
                name,
            }
            | Branch {
                session_id,
                expected_revision,
                name,
            } => {
                target(*session_id, *expected_revision)?;
                label(name)?;
            }
            ChangeModel {
                session_id,
                expected_revision,
                model,
            } => {
                target(*session_id, *expected_revision)?;
                label(model)?;
            }
            Archive {
                session_id,
                expected_revision,
                ..
            } => target(*session_id, *expected_revision)?,
            Delete {
                session_id,
                expected_revision,
                confirmation_id,
            }
            | SetSharing {
                session_id,
                expected_revision,
                confirmation_id,
                ..
            } => {
                target(*session_id, *expected_revision)?;
                ids(&[*confirmation_id])?;
            }
            DecideApproval {
                session_id,
                run_id,
                approval_id,
                request_digest,
                ..
            } => {
                ids(&[*session_id, *run_id, *approval_id])?;
                if request_digest.len() != 64
                    || !request_digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                {
                    return Err(DecodeError::InvalidBounds);
                }
            }
        }
        // JSON escaping can expand control characters by six times. A domain-
        // valid command must also fit its canonical wire representation.
        if serde_json::to_vec(self)
            .map_err(|_| DecodeError::Malformed)?
            .len()
            > MAX_FRAME_BYTES
        {
            return Err(DecodeError::Oversized);
        }
        Ok(())
    }

    /// Stable admission identity excludes connection_id, which changes during
    /// reconnect. Expiry remains bound: retry cannot extend authority. Persist
    /// the digest of these bytes with command ID before any effects occur.
    pub fn admission_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        #[derive(Serialize)]
        struct Identity<'a> {
            version: u16,
            machine_id: Uuid,
            principal_id: Uuid,
            command_id: Uuid,
            expires_at_ms: i64,
            operation: &'a Operation,
        }
        serde_json::to_vec(&Identity {
            version: self.version,
            machine_id: self.machine_id,
            principal_id: self.principal_id,
            command_id: self.command_id,
            expires_at_ms: self.expires_at_ms,
            operation: &self.operation,
        })
    }
}
fn ids(values: &[Uuid]) -> Result<(), DecodeError> {
    if values.iter().any(Uuid::is_nil) {
        Err(DecodeError::InvalidIdentity)
    } else {
        Ok(())
    }
}
fn target(id: Uuid, revision: u64) -> Result<(), DecodeError> {
    ids(&[id])?;
    // SQLite uses signed integers; reserve one increment before admitting work.
    if revision >= i64::MAX as u64 {
        Err(DecodeError::InvalidBounds)
    } else {
        Ok(())
    }
}
fn label(value: &str) -> Result<(), DecodeError> {
    if value.trim().is_empty()
        || value.len() > 256
        || value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
    {
        Err(DecodeError::InvalidBounds)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn example() -> Command {
        Command {
            version: VERSION,
            connection_id: Uuid::new_v4(),
            machine_id: Uuid::new_v4(),
            principal_id: Uuid::new_v4(),
            command_id: Uuid::new_v4(),
            expires_at_ms: 100_000,
            operation: Operation::Submit {
                session_id: Uuid::new_v4(),
                expected_revision: 0,
                prompt: "private fixture prompt".into(),
            },
        }
    }
    #[test]
    fn strict_roundtrip_and_content_free_debug_errors() {
        let value = example();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(Command::decode(&bytes, 1).unwrap(), value);
        assert!(!format!("{value:?}").contains("private fixture"));
        assert!(
            !Command::decode(b"{\"secret\":\"private fixture\"}", 1)
                .unwrap_err()
                .to_string()
                .contains("private fixture")
        );
        let mut json = serde_json::to_value(&value).unwrap();
        json["provider_state"] = serde_json::json!({"private":"never allowed"});
        assert_eq!(
            Command::decode(&serde_json::to_vec(&json).unwrap(), 1).unwrap_err(),
            DecodeError::Malformed
        );
        json.as_object_mut().unwrap().remove("provider_state");
        json["operation"]["workspace"] = serde_json::json!("/arbitrary/path");
        assert_eq!(
            Command::decode(&serde_json::to_vec(&json).unwrap(), 1).unwrap_err(),
            DecodeError::Malformed
        );
    }
    #[test]
    fn retired_versions_unknown_operations_and_duplicate_fields_fail_closed() {
        let mut value = example();
        for version in [0, 1, 3, u16::MAX] {
            value.version = version;
            assert_eq!(value.validate(1), Err(DecodeError::UnsupportedVersion));
        }
        let mut json = serde_json::to_value(example()).unwrap();
        json["operation"]["type"] = serde_json::json!("task_pull");
        assert_eq!(
            Command::decode(&serde_json::to_vec(&json).unwrap(), 1).unwrap_err(),
            DecodeError::Malformed
        );
        let bytes = serde_json::to_string(&example()).unwrap();
        let duplicate = bytes.replacen("{", "{\"version\":2,", 1);
        assert_eq!(
            Command::decode(duplicate.as_bytes(), 1).unwrap_err(),
            DecodeError::Malformed
        );
    }
    #[test]
    fn byte_limits_unicode_deadlines_and_nil_ids_are_enforced() {
        let mut value = example();
        for now in [-1, 100_000, i64::MAX] {
            assert_eq!(value.validate(now), Err(DecodeError::InvalidDeadline));
        }
        value.expires_at_ms = MAX_COMMAND_LIFETIME_MS + 2;
        assert_eq!(value.validate(1), Err(DecodeError::InvalidDeadline));
        value.expires_at_ms = MAX_COMMAND_LIFETIME_MS + 1;
        assert!(value.validate(1).is_ok());
        value.principal_id = Uuid::nil();
        assert_eq!(value.validate(1), Err(DecodeError::InvalidIdentity));
        value.principal_id = Uuid::new_v4();
        for prompt in [
            "é".repeat(MAX_PROMPT_BYTES / 2 + 1),
            " ".into(),
            "nul\0byte".into(),
        ] {
            value.operation = Operation::Submit {
                session_id: Uuid::new_v4(),
                expected_revision: 0,
                prompt,
            };
            assert_eq!(value.validate(1), Err(DecodeError::InvalidBounds));
        }
        assert_eq!(
            Command::decode(&vec![b' '; MAX_FRAME_BYTES + 1], 1).unwrap_err(),
            DecodeError::Oversized
        );
        assert_eq!(
            Command::decode(&[0xff], 1).unwrap_err(),
            DecodeError::Malformed
        );
    }
    #[test]
    fn every_full_lifecycle_operation_roundtrips_without_raw_session_types() {
        let id = Uuid::new_v4();
        let revision = 5;
        let operations = vec![
            Operation::List {
                after: None,
                limit: 100,
            },
            Operation::Create {
                workspace_id: id,
                name: "managed".into(),
            },
            Operation::Inspect { session_id: id },
            Operation::Cancel {
                session_id: id,
                run_id: id,
            },
            Operation::Rename {
                session_id: id,
                expected_revision: revision,
                name: "renamed".into(),
            },
            Operation::ChangeModel {
                session_id: id,
                expected_revision: revision,
                model: "model".into(),
            },
            Operation::Branch {
                session_id: id,
                expected_revision: revision,
                name: "branch".into(),
            },
            Operation::Archive {
                session_id: id,
                expected_revision: revision,
                archived: true,
            },
            Operation::Delete {
                session_id: id,
                expected_revision: revision,
                confirmation_id: id,
            },
            Operation::SetSharing {
                session_id: id,
                expected_revision: revision,
                confirmation_id: id,
                mode: SharingMode::None,
            },
            Operation::DecideApproval {
                session_id: id,
                run_id: id,
                approval_id: id,
                request_digest: "a".repeat(64),
                approve: false,
            },
        ];
        for operation in operations {
            let mut command = example();
            command.operation = operation;
            assert_eq!(
                Command::decode(&serde_json::to_vec(&command).unwrap(), 1).unwrap(),
                command
            );
        }
    }
    #[test]
    fn reconnect_identity_is_stable_but_authority_and_payload_changes_are_not() {
        let original = example();
        let bytes = original.admission_bytes().unwrap();
        let mut retry = original.clone();
        retry.connection_id = Uuid::new_v4();
        assert_eq!(retry.admission_bytes().unwrap(), bytes);
        retry.principal_id = Uuid::new_v4();
        assert_ne!(retry.admission_bytes().unwrap(), bytes);
        retry = original.clone();
        retry.expires_at_ms += 1;
        assert_ne!(retry.admission_bytes().unwrap(), bytes);
        retry = original.clone();
        retry.operation = Operation::Inspect {
            session_id: Uuid::new_v4(),
        };
        assert_ne!(retry.admission_bytes().unwrap(), bytes);
    }
    #[test]
    fn destructive_confirmation_and_bounds_fail_closed() {
        let mut command = example();
        let id = Uuid::new_v4();
        for operation in [
            Operation::List {
                after: None,
                limit: 0,
            },
            Operation::List {
                after: None,
                limit: 101,
            },
            Operation::Rename {
                session_id: id,
                expected_revision: u64::MAX,
                name: "name".into(),
            },
            Operation::Create {
                workspace_id: id,
                name: "hidden\u{202e}name".into(),
            },
            Operation::Create {
                workspace_id: id,
                name: "é".repeat(129),
            },
            Operation::DecideApproval {
                session_id: id,
                run_id: id,
                approval_id: id,
                request_digest: "Z".repeat(64),
                approve: true,
            },
        ] {
            command.operation = operation;
            assert!(command.validate(1).is_err());
        }
        command.operation = Operation::Delete {
            session_id: id,
            expected_revision: 0,
            confirmation_id: Uuid::nil(),
        };
        assert_eq!(command.validate(1), Err(DecodeError::InvalidIdentity));
    }
    #[test]
    fn encoded_expansion_cannot_make_a_valid_command_unsendable() {
        let mut value = example();
        value.operation = Operation::Submit {
            session_id: Uuid::new_v4(),
            expected_revision: 0,
            prompt: "\u{0001}".repeat(MAX_PROMPT_BYTES),
        };
        assert_eq!(value.validate(1), Err(DecodeError::Oversized));
        for prompt in [
            "a".repeat(MAX_PROMPT_BYTES),
            "é".repeat(MAX_PROMPT_BYTES / 2),
            "\n".repeat(MAX_PROMPT_BYTES - 1) + "x",
        ] {
            value.operation = Operation::Submit {
                session_id: Uuid::new_v4(),
                expected_revision: 0,
                prompt,
            };
            assert!(value.validate(1).is_ok());
            assert_eq!(
                Command::decode(&serde_json::to_vec(&value).unwrap(), 1).unwrap(),
                value
            );
        }
    }
    #[test]
    fn nested_duplicates_missing_fields_trailing_json_and_bad_versions_are_rejected() {
        let value = example();
        let encoded = serde_json::to_string(&value).unwrap();
        for bytes in [
            encoded.replacen(
                "\"type\":\"submit\"",
                "\"type\":\"submit\",\"type\":\"submit\"",
                1,
            ),
            encoded.replacen(
                "\"expected_revision\":0",
                "\"expected_revision\":0,\"expected_revision\":0",
                1,
            ),
            encoded.clone() + "{}",
        ] {
            assert_eq!(
                Command::decode(bytes.as_bytes(), 1).unwrap_err(),
                DecodeError::Malformed
            );
        }
        let mut json = serde_json::to_value(&value).unwrap();
        json.as_object_mut().unwrap().remove("principal_id");
        assert_eq!(
            Command::decode(&serde_json::to_vec(&json).unwrap(), 1).unwrap_err(),
            DecodeError::Malformed
        );
        json = serde_json::to_value(&value).unwrap();
        json["version"] = serde_json::json!(1);
        assert_eq!(
            Command::decode(&serde_json::to_vec(&json).unwrap(), 1).unwrap_err(),
            DecodeError::UnsupportedVersion
        );
    }
    #[test]
    fn revisions_labels_digests_and_semantic_json_identity_boundaries() {
        let mut value = example();
        let id = Uuid::new_v4();
        value.operation = Operation::Rename {
            session_id: id,
            expected_revision: (i64::MAX - 1) as u64,
            name: "é".repeat(128),
        };
        assert!(value.validate(1).is_ok());
        value.operation = Operation::Rename {
            session_id: id,
            expected_revision: i64::MAX as u64,
            name: "ok".into(),
        };
        assert_eq!(value.validate(1), Err(DecodeError::InvalidBounds));
        for digest in ["a".repeat(63), "a".repeat(65), "A".repeat(64)] {
            value.operation = Operation::DecideApproval {
                session_id: id,
                run_id: id,
                approval_id: id,
                request_digest: digest,
                approve: true,
            };
            assert_eq!(value.validate(1), Err(DecodeError::InvalidBounds));
        }
        value = example();
        let sorted = serde_json::to_vec(&serde_json::to_value(&value).unwrap()).unwrap();
        assert_eq!(
            Command::decode(&sorted, 1)
                .unwrap()
                .admission_bytes()
                .unwrap(),
            value.admission_bytes().unwrap()
        );
        let mut changed = value.clone();
        changed.machine_id = Uuid::new_v4();
        assert_ne!(
            changed.admission_bytes().unwrap(),
            value.admission_bytes().unwrap()
        );
        changed = value.clone();
        changed.command_id = Uuid::new_v4();
        assert_ne!(
            changed.admission_bytes().unwrap(),
            value.admission_bytes().unwrap()
        );
        changed = value.clone();
        if let Operation::Submit { prompt, .. } = &mut changed.operation {
            prompt.push('!');
        }
        assert_ne!(
            changed.admission_bytes().unwrap(),
            value.admission_bytes().unwrap()
        );
    }
}
