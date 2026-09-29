//! Private local process supervision. No execution library is linked here.
mod launch;
mod models;
mod registry;
mod routing;
mod service;
pub use service::{GatewayConfig, serve, serve_configured};

#[cfg(target_os = "linux")]
pub mod gateway_ipc;

mod recovery;
mod suspension;

mod start;

mod access;
mod exchange;
pub use exchange::exchange;

pub mod grant_cli;

mod lifecycle;

mod identity;
mod transfer;

mod participant;

mod recover_command;

mod api;

pub mod pair_cli;
pub mod pairing;

mod accounts;

mod notifications;

mod catalogue;
mod catalogue_observer;
mod database;
mod runtime_storage;

#[cfg(test)]
#[path = "process/test_support_tests.rs"]
mod test_support;

#[cfg(test)]
mod start_tests;

#[cfg(test)]
mod account_flows_tests;

mod execution_profiles;

mod updates;

#[cfg(test)]
mod authority_routing_tests;

#[cfg(target_os = "linux")]
mod bound_lifecycle;
#[cfg(target_os = "linux")]
mod default_execution;
#[cfg(target_os = "linux")]
pub mod guardian;
#[cfg(target_os = "linux")]
mod guardian_observation;
