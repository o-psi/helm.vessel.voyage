//! Versioned private process transport. Authentication remains adapter-owned.
mod codec;
mod types;
pub use codec::*;
pub use types::*;

mod lifecycle;
pub use lifecycle::*;

mod access;
pub use access::*;
mod transfer;
pub use transfer::*;
mod participation;
pub use participation::*;

// Executing-host management adapters also use the public service API.
pub use crate::vessel::{
    VesselCommand, VesselEvent, VesselEventRequest, VesselEventSubscription, VesselRequest,
    VesselResponse,
};
