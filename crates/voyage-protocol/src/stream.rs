//! Authenticated attachment stream projections. Canonical Helm sessions and
//! provider continuation types deliberately cannot be represented here.
use crate::{attachment::{Command, SharingMode, VERSION, MAX_FRAME_BYTES}, enrollment::SignedChallenge};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationEntry { pub role: String, pub text: String }
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case", deny_unknown_fields)]
pub enum Reply {
    Sessions { sessions: Vec<SessionView> },
    Session { session: SessionView },
    Run { session_id: Uuid, run_id: Uuid, state: String },
    Accepted,
    Denied { code: String },
    History { session_id: Uuid, entries: Vec<ConversationEntry>, next: Option<u64> },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag="type", rename_all="snake_case", deny_unknown_fields)]
pub enum Frame {
    Authenticate { version: u16, proof: SignedChallenge },
    Welcome { version: u16, connection_id: Uuid, machine_id: Uuid, owner_id: Uuid, epoch: u64, lease_ms: u32 },
    Command { command: Command },
    Result { connection_id: Uuid, command_id: Uuid, reply: Reply },
    Heartbeat { connection_id: Uuid },
    Lease { connection_id: Uuid, lease_ms: u32 },
}
impl Frame {
    pub fn decode(bytes: &[u8]) -> Result<Self, &'static str> {
        if bytes.len() > MAX_FRAME_BYTES { return Err("attachment frame too large"); }
        let value: Self = serde_json::from_slice(bytes).map_err(|_| "invalid attachment frame")?;
        match &value {
            Self::Authenticate {version,..} | Self::Welcome {version,..} if *version != VERSION => return Err("unsupported attachment version"),
            Self::Welcome{connection_id,machine_id,owner_id,epoch,lease_ms,..} if connection_id.is_nil() || machine_id.is_nil() || owner_id.is_nil() || *epoch == 0 || !(1000..=30000).contains(lease_ms) => return Err("invalid attachment welcome"),
            Self::Result{connection_id,command_id,..} if connection_id.is_nil() || command_id.is_nil() => return Err("invalid attachment identity"),
            Self::Heartbeat{connection_id} | Self::Lease{connection_id,..} if connection_id.is_nil() => return Err("invalid attachment identity"),
            Self::Lease{lease_ms,..} if !(1000..=30000).contains(lease_ms) => return Err("invalid attachment lease"),
            _ => (),
        }
        Ok(value)
    }
    pub fn encode(&self) -> Result<String, &'static str> {
        let encoded = serde_json::to_string(self).map_err(|_| "attachment encoding failed")?;
        if encoded.len() > MAX_FRAME_BYTES {return Err("attachment frame too large");}
        Ok(encoded)
    }
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, f:&mut std::fmt::Formatter<'_>)->std::fmt::Result { f.write_str("AttachmentFrame { .. }") }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn frames_are_bounded_strict_and_content_free() {
        let id=Uuid::new_v4();
        let frame=Frame::Result{connection_id:id,command_id:id,reply:Reply::Denied{code:"private sentinel".into()}};
        assert!(!format!("{frame:?}").contains("sentinel"));
        assert!(Frame::decode(frame.encode().unwrap().as_bytes()).is_ok());
        assert!(Frame::decode(br#"{"type":"heartbeat","unknown":true}"#).is_err());
        assert!(Frame::decode(&vec![b' ';MAX_FRAME_BYTES+1]).is_err());
        assert!(Frame::decode(Frame::Lease{connection_id:id,lease_ms:30001}.encode().unwrap().as_bytes()).is_err());
        assert!(Frame::decode(Frame::Welcome{version:1,connection_id:id,machine_id:id,owner_id:id,epoch:1,lease_ms:1000}.encode().unwrap().as_bytes()).is_err());
    }
}
