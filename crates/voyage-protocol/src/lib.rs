//! Common management-plane response types. Legacy pairing/task wire types are retired.
//! Attachment command-domain foundations are defined separately; transport is not enabled.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

pub mod attachment;

pub mod enrollment;

/// Strict wire codec foundations; no network adapter or authentication is enabled.
pub mod stream;

/// Typed bounded observations; no execution authority.
pub mod events;
