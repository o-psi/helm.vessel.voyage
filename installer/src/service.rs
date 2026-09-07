use anyhow::{Result, bail};

#[cfg(target_os = "linux")]
mod command;
#[cfg(target_os = "linux")]
mod files;
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
    #[cfg(not(target_os = "linux"))]
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
#[cfg(not(target_os = "linux"))]
pub fn preview(_bin: &std::path::Path, _start: bool) -> Result<String> {
    bail!("Service management requires Linux systemd user services")
}
#[cfg(not(target_os = "linux"))]
pub fn configure(_bin: &std::path::Path, _start: bool, _dry_run: bool) -> Result<()> {
    bail!("Service management requires Linux systemd user services")
}
