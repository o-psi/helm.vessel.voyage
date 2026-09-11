//! Shared Vessel service and independent voyage runtime contracts.

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

/// Versioned independent runtime and local supervisor transport.
pub mod process;

/// Public client-to-Vessel service contract.
pub mod vessel;

/// Ordered turn content metadata. Not yet accepted by submission transports.
pub mod content;

/// Model/account-scoped inference observations.
pub mod inference;

pub mod tool_result;

/// Executing-host account and private enrollment contracts.
pub mod accounts;

/// Voyage-to-voyage presentation provenance.
pub mod coordination;

pub mod browser;
pub mod duplex;

#[cfg(test)]
mod workflow_preview_tests;
