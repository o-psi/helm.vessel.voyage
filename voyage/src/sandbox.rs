//! Explicit process isolation, separate from application approvals and model transport.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Off,
    Required,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Network {
    #[default]
    Denied,
    Host,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub mode: Mode,
    pub network: Network,
    pub address_space_bytes: u64,
    pub cpu_seconds: u64,
    pub file_size_bytes: u64,
    pub open_files: u64,
    /// RLIMIT_NPROC counts every process of the real UID, including outside Helm.
    pub uid_processes: u64,
    pub temporary_bytes: u64,
    // Decode-only compatibility for saved configurations predating bridge removal.
    // Discard values: they must not grant authority, affect equality, or be
    // serialized into new configurations. Other unknown fields remain errors.
    #[doc(hidden)]
    #[serde(
        rename = "bridge_read",
        deserialize_with = "discard_retired_setting",
        skip_serializing
    )]
    pub legacy_bridge_read: (),
    #[doc(hidden)]
    #[serde(
        rename = "bridge_inherit_env",
        deserialize_with = "discard_retired_setting",
        skip_serializing
    )]
    pub legacy_bridge_inherit_env: (),
    #[doc(hidden)]
    #[serde(
        rename = "bridge_network",
        deserialize_with = "discard_retired_setting",
        skip_serializing
    )]
    pub legacy_bridge_network: (),
}
fn discard_retired_setting<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<(), D::Error> {
    serde::de::IgnoredAny::deserialize(deserializer).map(|_| ())
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Off,
            network: Network::Denied,
            address_space_bytes: 4 * 1024 * 1024 * 1024,
            cpu_seconds: 600,
            file_size_bytes: 1024 * 1024 * 1024,
            open_files: 1024,
            uid_processes: 4096,
            temporary_bytes: 256 * 1024 * 1024,
            legacy_bridge_read: (),
            legacy_bridge_inherit_env: (),
            legacy_bridge_network: (),
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("sandbox configuration is invalid: {0}")]
    Invalid(&'static str),
    #[error("required sandbox supports Linux x86_64 only")]
    Unsupported,
    #[error(
        "required sandbox setup failed; install system bubblewrap and enable supported unprivileged namespaces; no unsandboxed fallback"
    )]
    Setup,
}
impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        if [
            self.address_space_bytes,
            self.cpu_seconds,
            self.file_size_bytes,
            self.uid_processes,
            self.temporary_bytes,
        ]
        .iter()
        .any(|value| *value == 0 || *value == u64::MAX)
            || self.open_files < 32
            || self.open_files > 1_048_576
        {
            return Err(Error::Invalid(
                "finite positive limits required (open_files 32..1048576)",
            ));
        }
        Ok(())
    }
}
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod linux;
#[derive(Clone, Debug)]
pub struct Sandbox {
    pub settings: Settings,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    roots: std::sync::Arc<Vec<linux::Root>>,
}
/// Immutable package bytes; never a user-supplied executable path.
pub(crate) struct ExtensionImage {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    file: std::sync::Arc<std::fs::File>,
}
impl ExtensionImage {
    pub(crate) fn seal(bytes: &[u8]) -> Result<Self, Error> {
        crate::extensions::executable::validate_elf(bytes)
            .map_err(|_| Error::Invalid("unsupported executable artifact"))?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            linux::seal_extension(bytes)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            Err(Error::Unsupported)
        }
    }
}
impl Sandbox {
    pub fn new(settings: &Settings, read: &[PathBuf], write: &[PathBuf]) -> Result<Self, Error> {
        settings.validate()?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            Ok(Self {
                settings: settings.clone(),
                roots: std::sync::Arc::new(if settings.mode == Mode::Off {
                    vec![]
                } else {
                    linux::roots(read, write)?
                }),
            })
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (read, write);
            if settings.mode == Mode::Required {
                return Err(Error::Unsupported);
            }
            Ok(Self {
                settings: settings.clone(),
            })
        }
    }
    pub fn required(&self) -> bool {
        self.settings.mode == Mode::Required
    }
    /// Dedicated SDK profile: reuse enforced limits/filter/namespaces, but do
    /// not expose ordinary process roots, runtime mounts, environment or network.
    /// Callers must retain image, process ownership and durable admission intent.
    pub(crate) fn apply_extension(
        &self,
        command: &mut Command,
        image: &ExtensionImage,
    ) -> Result<(), Error> {
        if !self.required() {
            return Err(Error::Invalid(
                "executable extensions require enforced isolation",
            ));
        }
        if command.get_program() != std::ffi::OsStr::new("/extension")
            || command.get_args().next().is_some()
            || command.get_envs().any(|(_, value)| value.is_some())
        {
            return Err(Error::Invalid(
                "executable launch contains unsupported arguments or environment",
            ));
        }
        command.current_dir("/").env_clear();
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            linux::apply_extension(self, command, image)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = image;
            Err(Error::Unsupported)
        }
    }
    /// Apply last, after payload arguments, environment, stdio and pre-exec hooks.
    /// The adapter receives an empty environment; payload bindings travel in a private FD.
    pub fn apply(&self, command: &mut Command, cwd: &Path) -> Result<(), Error> {
        self.apply_read_only(command, cwd, false)
    }
    /// Narrow already-pinned write mounts for the current dispatch access.
    pub fn apply_read_only(
        &self,
        command: &mut Command,
        cwd: &Path,
        read_only: bool,
    ) -> Result<(), Error> {
        if !self.required() {
            return Ok(());
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            linux::apply(self, command, cwd, read_only)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (command, cwd, read_only);
            Err(Error::Unsupported)
        }
    }
}

/// Bounded operational probe. A successful `/bin/true` verifies setup on this host,
/// not the completeness of every restriction or any user's requested workload.
pub fn diagnostics(config: &crate::Config, workspace: &Path) -> serde_json::Value {
    let report = (|| -> anyhow::Result<bool> {
        let policy = crate::policy::Policy::new(config, workspace.into())?;
        if !policy.sandbox().required() {
            return Ok(true);
        }
        let mut command = Command::new("/bin/true");
        command
            .env_clear()
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        policy.isolate_process(&mut command, workspace)?;
        let mut child = command.spawn()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(status.success());
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(false);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    })();
    let ready = matches!(report, Ok(true));
    serde_json::json!({"mode":config.sandbox.mode,"ready":ready,"adapter":if config.sandbox.mode==Mode::Required {"linux-bubblewrap-x86_64"}else{"none"},"settings":config.sandbox,"limits_scope":"address space, CPU, file size and descriptors are per process; process count includes the same real UID outside this sandbox; temporary_bytes bounds private tmpfs only; no aggregate descendant CPU or memory budget", "remediation":if ready {None}else{Some("required isolation unavailable: check normalized roots, system bubblewrap, unprivileged namespaces and kernel support; no automatic fallback")}})
}
