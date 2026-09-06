//! Strict attachment wire projections, not an authenticated transport.
//!
//! Decode/encode check timeless structure, identities and bounded content. They
//! do not grant authority: the adapter must verify Connect proofs against trusted
//! origin/key/time, consume challenges, bind commands to authenticated scopes,
//! check current epochs/sharing, and validate deadlines before new admission.
//! Expired commands may only retrieve existing authorized deduplication evidence.
//! Canonical Helm sessions and provider continuation cannot be represented here.
use crate::events::{EventCursor, Features, MAX_REPLAY_EVENTS, SequencedEvent, validate_replay};
use crate::{
    attachment::{Command, MAX_FRAME_BYTES, MAX_PROMPT_BYTES, SharingMode, VERSION, label},
    enrollment::{
        MAX_ORIGIN_BYTES, MAX_PROOF_BYTES, PROOF_VERSION, ProofOperation, SignedChallenge,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

pub const MAX_SESSIONS: usize = 100;
pub const MAX_HISTORY_ENTRIES: usize = 100;
pub const MAX_ENTRY_BYTES: usize = MAX_PROMPT_BYTES;
pub const MIN_LEASE_MS: u32 = 1_000;
pub const MAX_LEASE_MS: u32 = 30_000;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionView {
    pub id: Uuid,
    pub revision: u64,
    pub name: String,
    pub model: String,
    pub sharing: SharingMode,
    pub archived: bool,
}

/// Runtime system instructions have no conversation projection role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRole {
    User,
    Assistant,
    Tool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationEntry {
    pub role: ConversationRole,
    pub text: String,
}

/// Execution state is distinct from frame delivery or command acceptance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Accepted,
    Running,
    Completed,
    Incomplete,
    Cancelled,
    Failed,
    Interrupted,
}

/// Fixed codes prevent accidentally exporting raw provider/storage error text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenialCode {
    InvalidRequest,
    Unauthorized,
    Conflict,
    NotFound,
    Expired,
    Busy,
    ResourceExhausted,
    SnapshotRequired,
    Internal,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionView {
    pub run_id: Uuid,
    pub state: RunState,
    pub cleanup: crate::events::CleanupState,
    pub input_tokens: u64,
    pub output_tokens: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Sessions {
        sessions: Vec<SessionView>,
    },
    Session {
        session: SessionView,
    },
    Run {
        session_id: Uuid,
        run_id: Uuid,
        state: RunState,
    },
    /// Bounded current projection, independent from canonical/private history.
    ExecutionSnapshot {
        session: SessionView,
        run: Option<ExecutionView>,
        latest: EventCursor,
    },
    Accepted {},
    Denied {
        code: DenialCode,
    },
    History {
        session_id: Uuid,
        entries: Vec<ConversationEntry>,
        next: Option<u64>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    ControlRequest {
        connection_id: Uuid,
        request_id: Uuid,
        operation: crate::control::ClientOperation,
    },
    ControlResult {
        connection_id: Uuid,
        request_id: Uuid,
        reply: crate::control::Reply,
    },
    Authenticate {
        version: u16,
        proof: SignedChallenge,
        #[serde(default, skip_serializing_if = "Features::is_empty")]
        features: Features,
    },
    Welcome {
        version: u16,
        connection_id: Uuid,
        machine_id: Uuid,
        owner_id: Uuid,
        epoch: u64,
        lease_ms: u32,
        #[serde(default, skip_serializing_if = "Features::is_empty")]
        features: Features,
    },
    Event {
        connection_id: Uuid,
        session_id: Uuid,
        event: SequencedEvent,
    },
    ReplayRequest {
        connection_id: Uuid,
        request_id: Uuid,
        session_id: Uuid,
        after: EventCursor,
        limit: u16,
    },
    Replay {
        connection_id: Uuid,
        request_id: Uuid,
        session_id: Uuid,
        after: EventCursor,
        latest: EventCursor,
        events: Vec<SequencedEvent>,
    },
    SnapshotRequired {
        connection_id: Uuid,
        request_id: Uuid,
        session_id: Uuid,
        after: EventCursor,
        latest: EventCursor,
    },
    Command {
        command: Command,
    },
    /// A command response, or a ReplayRequest refusal with Reply::Denied.
    /// In the latter case command_id equals the replay request_id; no new wire
    /// variant or feature is needed and earlier v2 decoders accept the envelope.
    Result {
        connection_id: Uuid,
        command_id: Uuid,
        reply: Reply,
    },
    Heartbeat {
        connection_id: Uuid,
    },
    Lease {
        connection_id: Uuid,
        lease_ms: u32,
    },
}
impl Frame {
    /// Reject byte limits before JSON allocation. No authentication is performed.
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err("attachment frame too large");
        }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| "invalid attachment frame")?;
        value.validate_structure()?;
        // Canonical serialization also has to fit, including all JSON escaping.
        value.bounded_encoding()?;
        Ok(value)
    }

    pub fn encode(&self) -> Result<String, &'static str> {
        self.validate_structure()?;
        self.bounded_encoding()
    }

    /// Structural decoding alone does not establish negotiation. Authenticated
    /// adapters must call this on inbound AND outbound post-handshake frames,
    /// then independently recheck authorization at dispatch/disclosure commit.
    pub fn validate_features(&self, negotiated: &Features) -> Result<(), &'static str> {
        use crate::events::Feature;
        self.validate_structure()?;
        negotiated.validate()?;
        match self {
            Self::ControlRequest { .. } | Self::ControlResult { .. } => {
                if !negotiated.contains(Feature::CoordinationControl) {
                    return Err("coordination control not negotiated");
                }
                Ok(())
            }
            Self::Result {
                reply: Reply::ExecutionSnapshot { .. },
                ..
            } => {
                if !negotiated.contains(Feature::ManagedExecution) {
                    return Err("managed execution not negotiated");
                }
                Ok(())
            }
            Self::Event { event, .. } => negotiated.permits_event(&event.event),
            Self::Replay { events, .. } => {
                if !negotiated.contains(Feature::Replay) {
                    return Err("replay not negotiated");
                }
                for event in events {
                    negotiated.permits_event(&event.event)?;
                }
                Ok(())
            }
            Self::ReplayRequest { .. } | Self::SnapshotRequired { .. } => {
                if !negotiated.contains(Feature::Replay) {
                    return Err("replay not negotiated");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn bounded_encoding(&self) -> Result<String, &'static str> {
        let encoded = serde_json::to_string(self).map_err(|_| "attachment encoding failed")?;
        if encoded.len() > MAX_FRAME_BYTES {
            return Err("attachment frame too large");
        }
        Ok(encoded)
    }

    fn validate_structure(&self) -> Result<(), &'static str> {
        match self {
            Self::ControlRequest {
                connection_id,
                request_id,
                operation,
            } => {
                identities(&[*connection_id, *request_id])?;
                operation.validate()?;
            }
            Self::ControlResult {
                connection_id,
                request_id,
                reply,
            } => {
                identities(&[*connection_id, *request_id])?;
                reply.validate()?;
            }
            Self::Authenticate {
                version,
                proof,
                features,
            } => {
                features.validate()?;
                version_two(*version)?;
                connect_proof(proof)?;
            }
            Self::Welcome {
                version,
                connection_id,
                machine_id,
                owner_id,
                epoch,
                lease_ms,
                features,
            } => {
                features.validate()?;
                version_two(*version)?;
                identities(&[*connection_id, *machine_id, *owner_id])?;
                epoch_value(*epoch)?;
                lease(*lease_ms)?;
            }
            Self::Event {
                connection_id,
                session_id,
                event,
            } => {
                identities(&[*connection_id, *session_id])?;
                event.validate()?;
            }
            Self::ReplayRequest {
                connection_id,
                request_id,
                session_id,
                after,
                limit,
            } => {
                identities(&[*connection_id, *request_id, *session_id])?;
                crate::events::bound(after.get())?;
                if *limit == 0 || usize::from(*limit) > MAX_REPLAY_EVENTS {
                    return Err("invalid replay limit");
                }
            }
            Self::Replay {
                connection_id,
                request_id,
                session_id,
                after,
                latest,
                events,
            } => {
                identities(&[*connection_id, *request_id, *session_id])?;
                validate_replay(*after, *latest, events)?;
            }
            Self::SnapshotRequired {
                connection_id,
                request_id,
                session_id,
                after,
                latest,
            } => {
                identities(&[*connection_id, *request_id, *session_id])?;
                crate::events::bound(after.get())?;
                crate::events::bound(latest.get())?;
                if after >= latest {
                    return Err("invalid snapshot requirement");
                }
            }
            Self::Command { command } => command
                .validate_structure()
                .map_err(|_| "invalid attachment command")?,
            Self::Result {
                connection_id,
                command_id,
                reply,
            } => {
                identities(&[*connection_id, *command_id])?;
                reply.validate()?;
            }
            Self::Heartbeat { connection_id } => identities(&[*connection_id])?,
            Self::Lease {
                connection_id,
                lease_ms,
            } => {
                identities(&[*connection_id])?;
                lease(*lease_ms)?;
            }
        }
        Ok(())
    }
}

impl Reply {
    fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Sessions { sessions } => {
                if sessions.len() > MAX_SESSIONS {
                    return Err("too many attachment sessions");
                }
                let mut ids = BTreeSet::new();
                for session in sessions {
                    session.validate()?;
                    if !ids.insert(session.id) {
                        return Err("duplicate attachment session");
                    }
                }
            }
            Self::Session { session } => session.validate()?,
            Self::ExecutionSnapshot {
                session,
                run,
                latest,
            } => {
                session.validate()?;
                crate::events::bound(latest.get())?;
                if let Some(run) = run {
                    identities(&[run.run_id])?;
                    crate::events::bound(run.input_tokens)?;
                    crate::events::bound(run.output_tokens)?;
                    if matches!(run.state, RunState::Accepted | RunState::Running)
                        && !matches!(
                            run.cleanup,
                            crate::events::CleanupState::Pending
                                | crate::events::CleanupState::Unconfirmed
                        )
                    {
                        return Err("active run cannot have confirmed cleanup");
                    }
                }
            }
            Self::Run {
                session_id, run_id, ..
            } => identities(&[*session_id, *run_id])?,
            Self::History {
                session_id,
                entries,
                next,
            } => {
                identities(&[*session_id])?;
                if entries.len() > MAX_HISTORY_ENTRIES || next.is_some_and(|n| n > i64::MAX as u64)
                {
                    return Err("invalid attachment history bounds");
                }
                for entry in entries {
                    if entry.text.len() > MAX_ENTRY_BYTES || entry.text.contains('\0') {
                        return Err("invalid attachment history text");
                    }
                }
            }
            Self::Accepted {} | Self::Denied { .. } => (),
        }
        Ok(())
    }
}
impl SessionView {
    fn validate(&self) -> Result<(), &'static str> {
        identities(&[self.id])?;
        // A final read-only snapshot may hold the last signed revision even when
        // admitting another mutation at that revision would be forbidden.
        if self.revision > i64::MAX as u64 {
            return Err("invalid attachment session revision");
        }
        label(&self.name).map_err(|_| "invalid attachment session name")?;
        label(&self.model).map_err(|_| "invalid attachment model label")
    }
}
fn version_two(version: u16) -> Result<(), &'static str> {
    if version == VERSION {
        Ok(())
    } else {
        Err("unsupported attachment version")
    }
}
fn identities(ids: &[Uuid]) -> Result<(), &'static str> {
    if ids.iter().any(Uuid::is_nil) {
        Err("invalid attachment identity")
    } else {
        Ok(())
    }
}
fn epoch_value(epoch: u64) -> Result<(), &'static str> {
    if (1..i64::MAX as u64).contains(&epoch) {
        Ok(())
    } else {
        Err("invalid attachment epoch")
    }
}
fn lease(ms: u32) -> Result<(), &'static str> {
    if (MIN_LEASE_MS..=MAX_LEASE_MS).contains(&ms) {
        Ok(())
    } else {
        Err("invalid attachment lease")
    }
}
fn connect_proof(proof: &SignedChallenge) -> Result<(), &'static str> {
    let challenge = &proof.challenge;
    if challenge.version != PROOF_VERSION || challenge.expires_at_ms <= 0 {
        return Err("invalid attachment proof version or expiry");
    }
    identities(&[challenge.id])?;
    if challenge.origin.is_empty()
        || challenge.origin.len() > MAX_ORIGIN_BYTES
        || challenge.origin.chars().any(char::is_control)
    {
        return Err("invalid attachment proof origin");
    }
    let ProofOperation::Connect { machine_id, epoch } = &challenge.operation else {
        return Err("attachment authentication requires a connect proof");
    };
    identities(&[*machine_id])?;
    epoch_value(*epoch)?;
    if challenge.server_tag.len() != 32
        || proof.signature.len() != 64
        || proof.new_signature.is_some()
    {
        return Err("invalid attachment proof shape");
    }
    if serde_json::to_vec(proof)
        .map_err(|_| "invalid attachment proof")?
        .len()
        > MAX_PROOF_BYTES
    {
        return Err("attachment proof too large");
    }
    Ok(())
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AttachmentFrame { .. }")
    }
}
