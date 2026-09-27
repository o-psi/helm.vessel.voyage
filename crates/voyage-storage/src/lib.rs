//! Private enrollment filesystem primitives; never wire types or authorization.
//! Windows support is limited to local NTFS directories with verified native ACLs.
#[cfg(windows)]
mod policy;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::PrivateDirectory;

pub mod credentials;

/// Staged privileged-control storage; requires protected root-owned ancestors.
#[cfg(target_os = "linux")]
pub mod protected_linux;
