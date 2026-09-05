//! Explicit operator-private defaults, never repository-discovered authority.
pub mod cli;
mod resolution;
mod store;
#[cfg(test)]
use resolution::resolve;
pub(crate) use resolution::{DefaultsGuard, resolve_using};
pub use resolution::{DefaultsPreview, activate, preview};
pub use store::*;
#[cfg(test)]
mod tests;
