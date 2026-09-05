//! Private enrollment filesystem primitives; never wire types or authorization.
//! Windows support is limited to local NTFS directories with verified native ACLs.
#[cfg(any(windows, test))]
mod policy;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::PrivateDirectory;
