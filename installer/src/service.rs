use anyhow::{Result, bail};

#[cfg(target_os = "linux")]
mod files;
#[cfg(target_os = "linux")]
mod lifecycle;
#[cfg(target_os = "linux")]
mod systemd;

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

pub fn install(args: &[String]) -> Result<()> {
    let mut binary_directory = None;
    let mut start = false;
    let mut dry_run = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bin-dir" if binary_directory.is_none() => {
                binary_directory =
                    Some(args.next().ok_or_else(|| {
                        anyhow::anyhow!("--bin-dir requires an absolute directory")
                    })?);
            }
            "--start" if !start => start = true,
            "--dry-run" if !dry_run => dry_run = true,
            _ => bail!("Unknown or repeated service argument: {arg}"),
        }
    }
    let directory = binary_directory
        .ok_or_else(|| anyhow::anyhow!("Service installation requires --bin-dir"))?;
    #[cfg(target_os = "linux")]
    return systemd::install(std::path::Path::new(directory), start, dry_run);
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (directory, start, dry_run);
        bail!("Service installation is supported only on Linux with systemd user services")
    }
}
