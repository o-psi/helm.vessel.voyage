use anyhow::{Result, bail};

#[cfg(target_os = "linux")]
pub(crate) mod command;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) mod files;
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
#[allow(dead_code)]
mod launchd;
#[cfg(target_os = "linux")]
mod lifecycle;
#[cfg(target_os = "linux")]
mod readiness;
#[cfg(target_os = "linux")]
mod systemd;
#[cfg(target_os = "linux")]
mod unit;

pub fn manage(command: &str, args: &[String]) -> Result<()> {
    if !args.is_empty() {
        bail!("Service lifecycle commands take no arguments");
    }
    #[cfg(target_os = "linux")]
    return lifecycle::manage(command);
    #[cfg(target_os = "macos")]
    return launchd::manage(command);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = command;
        bail!("Service management is supported only on Linux with systemd user services")
    }
}

#[cfg(target_os = "linux")]
pub fn preview(bin: &std::path::Path, start: bool) -> Result<String> {
    systemd::preview(bin, start)
}
#[cfg(target_os = "linux")]
pub fn configure(bin: &std::path::Path, start: bool, dry_run: bool) -> Result<()> {
    systemd::configure(bin, start, dry_run)
}
#[cfg(target_os = "macos")]
pub fn preview(bin: &std::path::Path, start: bool) -> Result<String> {
    launchd::preview(bin, start)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn preview(_bin: &std::path::Path, _start: bool) -> Result<String> {
    bail!("Service management requires Linux systemd user services")
}
#[cfg(target_os = "macos")]
pub fn configure(bin: &std::path::Path, start: bool, dry_run: bool) -> Result<()> {
    launchd::configure(bin, start, dry_run)
}
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn configure(_bin: &std::path::Path, _start: bool, _dry_run: bool) -> Result<()> {
    bail!("Service management requires Linux systemd user services")
}

#[cfg(target_os = "linux")]
#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub(crate) struct Activation {
    pub active: bool,
    pub enabled: bool,
    pub unit_file_state: String,
    pub definition: Option<String>,
    pub state: std::path::PathBuf,
}
#[cfg(target_os = "linux")]
pub(crate) fn review_activation(bin: &std::path::Path) -> Result<Activation> {
    systemd::review_activation(bin)
}
#[cfg(target_os = "linux")]
pub(crate) fn restore_activation(
    bin: &std::path::Path,
    candidate: &std::path::Path,
    prior: &Activation,
) -> Result<()> {
    systemd::restore_activation(bin, candidate, prior)
}

#[cfg(target_os = "linux")]
pub(crate) fn catalogue(
    bin: &std::path::Path,
    state: &std::path::Path,
) -> Result<Vec<serde_json::Value>> {
    readiness::catalogue(bin, state)
}
#[cfg(target_os = "linux")]
pub(crate) fn quiesce(bin: &std::path::Path, prior: &Activation) -> Result<()> {
    systemd::quiesce(bin, prior)
}
#[cfg(target_os = "linux")]
pub(crate) fn start_quarantined(bin: &std::path::Path, prior: &Activation) -> Result<()> {
    systemd::start_quarantined(bin, prior)
}

#[cfg(target_os = "linux")]
pub(crate) fn state_directory() -> Result<std::path::PathBuf> {
    Ok(unit::Layout::discover()?.state)
}

#[cfg(target_os = "linux")]
pub(crate) fn observe_activation(
    bin: &std::path::Path,
    prior: &Activation,
    previous: bool,
) -> Result<()> {
    systemd::observe_activation(bin, prior, previous)
}

#[cfg(target_os = "linux")]
pub(crate) fn credential_key_path(content: &str) -> Option<&str> {
    unit::credential_key_path(content)
}

#[cfg(target_os = "linux")]
mod failed_candidate;
#[cfg(target_os = "linux")]
pub(crate) fn capture_legacy_activation_context(
    bin: &std::path::Path,
    prior: &Activation,
    accounts: &std::path::Path,
    candidate: &crate::install::release::Manifest,
) -> Result<failed_candidate::ContextPin> {
    failed_candidate::capture(bin, prior, accounts, candidate)
}
#[cfg(target_os = "linux")]
pub(crate) fn quiesce_failed_legacy_candidate(
    bin: &std::path::Path,
    prior: &Activation,
    pin: &failed_candidate::ContextPin,
) -> Result<()> {
    failed_candidate::quiesce(bin, prior, pin)
}
#[cfg(target_os = "linux")]
pub(crate) fn verify_legacy_activation_context(
    prior: &Activation,
    pin: &failed_candidate::ContextPin,
) -> Result<()> {
    failed_candidate::unchanged(prior, pin)
}
