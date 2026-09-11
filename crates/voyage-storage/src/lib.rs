//! Private enrollment filesystem primitives; never wire types or authorization.
//! Windows support is limited to local NTFS directories with verified native ACLs.
#[cfg(windows)]
mod policy;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::PrivateDirectory;

pub mod credentials;
