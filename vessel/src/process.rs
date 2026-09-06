//! Private local process supervision. No execution library is linked here.
mod launch;
mod registry;
mod routing;
mod service;
pub use service::serve;

mod client;
pub use client::request;

mod recovery;
