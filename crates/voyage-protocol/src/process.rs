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
