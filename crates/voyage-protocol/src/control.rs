//! Metadata-only coordination control. Nomination/lease is NEVER execution authority.
use crate::stream::DenialCode;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_RECORDS: usize = 64;
pub const MAX_PARTICIPANTS: usize = 16;
pub const MAX_RECEIPTS: usize = 4096;
/// Leave room for one immutable final revoke receipt for every retained record.
pub const MAX_ORDINARY_RECEIPTS: usize = MAX_RECEIPTS - MAX_RECORDS;
pub const LEASE_MS: i64 = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Address {
    pub installation_id: Uuid,
    pub session_id: Uuid,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Machine {
    pub machine_id: Uuid,
    pub epoch: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Configured,
    Revoked,
}
/// This capability never grants execution authority, independently of runtime availability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAuthority {
    None,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub address: Address,
    pub source: Machine,
    pub source_revision: u64,
    pub revision: u64,
    pub scope_revision: u64,
    pub coordinator_epoch: u64,
    pub coordinator: Machine,
    pub participants: Vec<Machine>,
    pub status: Status,
    pub execution_authority: ExecutionAuthority,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lease {
    pub lease_id: Uuid,
    pub server_generation: u64,
    pub connection_id: Uuid,
    pub machine: Machine,
    pub record_revision: u64,
    pub coordinator: bool,
    pub participant: bool,
    pub expires_at_ms: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    pub record: Record,
    pub lease: Option<Lease>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientOperation {
    Register {
        command_id: Uuid,
        expires_at_ms: i64,
        address: Address,
        source_revision: u64,
    },
    /// Observe an immutable registration receipt, or prove absence after its deadline.
    ObserveRegistration {
        command_id: Uuid,
        expires_at_ms: i64,
        address: Address,
    },
    /// Returns currently nominated records and maintains control-only leases.
    Refresh {},
    Release {},
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Registered {
        record: Record,
        duplicate: bool,
    },
    Snapshot {
        views: Vec<View>,
    },
    Released {},
    NotRegistered {
        command_id: Uuid,
        address: Address,
        expires_at_ms: i64,
        observed_at_ms: i64,
    },
    Denied {
        code: DenialCode,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mutation {
    pub command_id: Uuid,
    pub expires_at_ms: i64,
    pub address: Address,
    pub expected_revision: u64,
    pub operation: MutationKind,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum MutationKind {
    /// Explicit metadata provenance recovery; does not adopt any execution role.
    RebindSource {
        expected_source: Machine,
        source: Machine,
    },
    Configure {
        coordinator: Machine,
        participants: Vec<Machine>,
    },
    Revoke {},
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub record: Record,
    pub duplicate: bool,
}
fn id(id: Uuid) -> Result<(), &'static str> {
    if id.is_nil() {
        Err("invalid control identity")
    } else {
        Ok(())
    }
}
fn bound(n: u64) -> Result<(), &'static str> {
    crate::events::bound(n)
}
fn positive(n: u64) -> Result<(), &'static str> {
    bound(n)?;
    if n == 0 {
        Err("invalid control revision")
    } else {
        Ok(())
    }
}
impl Address {
    pub fn validate(&self) -> Result<(), &'static str> {
        id(self.installation_id)?;
        id(self.session_id)
    }
}
impl Machine {
    pub fn validate(&self) -> Result<(), &'static str> {
        id(self.machine_id)?;
        positive(self.epoch)
    }
}
fn participants(coordinator: Machine, members: &[Machine]) -> Result<(), &'static str> {
    coordinator.validate()?;
    if members.is_empty() || members.len() > MAX_PARTICIPANTS || !members.contains(&coordinator) {
        return Err("invalid control scope");
    }
    for (i, m) in members.iter().enumerate() {
        m.validate()?;
        if members[..i]
            .iter()
            .any(|old| old.machine_id == m.machine_id)
        {
            return Err("duplicate control participant");
        }
    }
    Ok(())
}
impl Record {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.address.validate()?;
        self.source.validate()?;
        bound(self.source_revision)?;
        positive(self.revision)?;
        positive(self.scope_revision)?;
        positive(self.coordinator_epoch)?;
        if self.scope_revision > self.revision || self.coordinator_epoch > self.revision {
            return Err("invalid control revisions");
        }
        participants(self.coordinator, &self.participants)
    }
}
impl ClientOperation {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Self::Register {
            command_id,
            expires_at_ms,
            address,
            source_revision,
        } = self
        {
            id(*command_id)?;
            address.validate()?;
            bound(*source_revision)?;
            if *expires_at_ms <= 0 {
                return Err("invalid control expiry");
            }
        }
        if let Self::ObserveRegistration {
            command_id,
            expires_at_ms,
            address,
        } = self
        {
            id(*command_id)?;
            address.validate()?;
            if *expires_at_ms <= 0 {
                return Err("invalid control expiry");
            }
        }
        Ok(())
    }
}
impl Mutation {
    pub fn validate(&self) -> Result<(), &'static str> {
        id(self.command_id)?;
        self.address.validate()?;
        positive(self.expected_revision)?;
        if self.expires_at_ms <= 0 {
            return Err("invalid control expiry");
        }
        if let MutationKind::RebindSource {
            expected_source,
            source,
        } = &self.operation
        {
            expected_source.validate()?;
            source.validate()?;
            if expected_source == source {
                return Err("unchanged source binding");
            }
        }
        if let MutationKind::Configure {
            coordinator,
            participants: members,
        } = &self.operation
        {
            participants(*coordinator, members)?
        }
        Ok(())
    }
}
impl Lease {
    pub fn validate(&self) -> Result<(), &'static str> {
        id(self.lease_id)?;
        id(self.connection_id)?;
        positive(self.server_generation)?;
        self.machine.validate()?;
        positive(self.record_revision)?;
        if self.expires_at_ms <= 0 || (!self.coordinator && !self.participant) {
            return Err("invalid control lease");
        }
        Ok(())
    }
}
impl Reply {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Registered { record, .. } => record.validate()?,
            Self::Snapshot { views } => {
                if views.len() > MAX_RECORDS {
                    return Err("too many control records");
                }
                for (i, view) in views.iter().enumerate() {
                    view.record.validate()?;
                    if views[..i]
                        .iter()
                        .any(|v| v.record.address == view.record.address)
                    {
                        return Err("duplicate control record");
                    }
                    if let Some(lease) = &view.lease {
                        lease.validate()?;
                        if lease.record_revision != view.record.revision
                            || lease.expires_at_ms <= 0
                            || view.record.status != Status::Configured
                            || (!lease.coordinator && !lease.participant)
                            || lease.coordinator != (view.record.coordinator == lease.machine)
                            || lease.participant
                                != view.record.participants.contains(&lease.machine)
                        {
                            return Err("invalid control lease");
                        }
                    }
                }
            }
            Self::NotRegistered {
                command_id,
                address,
                expires_at_ms,
                observed_at_ms,
            } => {
                id(*command_id)?;
                address.validate()?;
                if *expires_at_ms <= 0 || observed_at_ms < expires_at_ms {
                    return Err("unconfirmed registration absence");
                }
            }
            Self::Released {} | Self::Denied { .. } => {}
        }
        Ok(())
    }
}
