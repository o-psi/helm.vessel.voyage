//! Versioned private process transport. Authentication remains adapter-owned.
mod codec;
mod types;
pub use codec::*;
pub use types::*;

impl RuntimeCommand {
    /// Durable reads may use an exclusively fenced helper after an unclean exit.
    /// This deliberately excludes controls, decisions and mutating resolution.
    pub fn observes_saved(&self) -> bool {
        matches!(
            self,
            Self::Snapshot
                | Self::History { .. }
                | Self::MessageChunk { .. }
                | Self::RunOutput { .. }
                | Self::ReadArtifact { .. }
                | Self::Receipt { .. }
                | Self::Events { .. }
        )
    }
}

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
