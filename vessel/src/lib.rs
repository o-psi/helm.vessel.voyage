//! Vessel supervises independent voyage processes and exposes scoped access.
pub mod origin;

#[cfg(target_os = "linux")]
pub mod process;

pub mod duplex;

/// Native ordinary ownership/registration contracts; backend remains unqualified.
#[allow(dead_code)]
pub mod native;
