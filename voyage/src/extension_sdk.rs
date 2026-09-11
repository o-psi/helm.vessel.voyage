//! Version-one private executable transport. No discovery or implicit launch effects.
//! Packaging, admission, sealed-file isolation and durable cleanup are adapters,
//! not authority inferred from a manifest or a successful protocol handshake.
mod definitions;
mod transport;
mod wire;

pub(crate) use definitions::{Definition, Definitions, Kind, validate_definitions};
pub(crate) use transport::{Cleanup, Completion, Host, Identity, Invocation, InvocationHandle,
    Executor, LaunchAdapter, Launched, Lease, MAX_READ};
pub(crate) use wire::{MAX_FRAME, parse_json};
