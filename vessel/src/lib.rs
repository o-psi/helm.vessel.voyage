//! Vessel supervises independent voyage processes and exposes scoped access.
pub mod origin;

#[cfg(target_os = "linux")]
pub mod process;
