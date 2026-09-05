//! Attachment foundations. No network entry point is enabled by this module.
//!
//! Until all local CLI/TUI writers use the coordinator, these foundations must not
//! be used to expose remote execution. See docs/attachment-production-contract.md.
pub mod journal;

pub mod runtime;

pub mod sharing;

pub mod client;

pub mod migration;

pub mod local_actor;
