//! Version-one private executable transport. No discovery or implicit launch effects.
//! Packaging, admission, sealed-file isolation and durable cleanup are adapters,
//! not authority inferred from a manifest or a successful protocol handshake.
mod definitions;
mod transport;
mod wire;

pub(crate) use definitions::{Definitions, Kind, validate_definitions};
pub(crate) use transport::{
    Cleanup, Executor, Host, Identity, Invocation, LaunchAdapter, Launched, Lease, MAX_READ,
};
pub(crate) use wire::deserialize_json;
