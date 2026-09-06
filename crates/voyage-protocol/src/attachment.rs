//! Replacement attachment domain contract, not the retired v1 worker protocol.
//!
//! Admission decoding validates syntax, bounds and the deadline. Timeless
//! structural validation is separate for duplicate lookup. Neither authenticates:
//! the receiver MUST bind machine/principal to the channel and enforce local
//! policy and sharing.
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

    /// Admission-time validation. A structurally valid expired command may be
    /// looked up for deduplication, but MUST NOT be newly admitted using this API.
    pub fn validate(&self, now_ms: i64) -> Result<(), DecodeError> {
        self.validate_structure()?;
        let max = now_ms
            .checked_add(MAX_COMMAND_LIFETIME_MS)
            .ok_or(DecodeError::InvalidDeadline)?;
        if now_ms < 0 || self.expires_at_ms <= now_ms || self.expires_at_ms > max {
            return Err(DecodeError::InvalidDeadline);
        }
        Ok(())
    }

    /// Timeless wire validation for authenticated duplicate-outcome lookup.
    /// Does not authorize a principal, authenticate a channel, or admit execution.
    /// The original deadline remains part of admission_bytes and cannot renew.
    pub fn validate_structure(&self) -> Result<(), DecodeError> {
        if self.version != VERSION {
            return Err(DecodeError::UnsupportedVersion);
        }
        ids(&[
            self.connection_id,
            self.machine_id,
            self.principal_id,
            self.command_id,
        ])?;
        if self.expires_at_ms <= 0 {
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
pub(crate) fn label(value: &str) -> Result<(), DecodeError> {
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
