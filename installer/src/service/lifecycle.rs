use super::files;
use anyhow::{Context, Result, bail};
use std::{fs, path::Path, process::Command};

const UNIT: &str = "voyage-vessel.service";

fn query(property: &str) -> Result<String> {
    let output = Command::new("systemctl")
        .args(["--user", "show", UNIT, "--value", "--property", property])
        .output()?;
    if !output.status.success() {
        bail!("Cannot query the systemd user manager");
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn run(args: &[&str]) -> Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()?;
    if !status.success() {
        bail!(
            "systemctl {args:?} failed ({status}); service files and runtime state are preserved"
        );
    }
    Ok(())
}

pub fn manage(command: &str) -> Result<()> {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 || uid != unsafe { libc::getuid() } {
        bail!("Run service management as the ordinary executing user, without sudo");
    }
    let home = std::env::var_os("HOME").context("HOME is required")?;
    let path = Path::new(&home).join(".config/systemd/user").join(UNIT);
    files::check_path(&path, uid)?;
    if command == "service-status" {
        println!(
            "Vessel supervisor: {}\nPID: {}\nUnit: {}",
            query("ActiveState")?,
            query("MainPID")?,
            path.display()
        );
        println!(
            "Individual voyage health is available through Vessel; supervisor state does not establish voyage survival."
        );
        return Ok(());
    }
    let content = fs::read_to_string(&path).context("No installed Voyage service unit")?;
    if !content.contains("Description=Voyage Vessel session supervisor\n") {
        bail!("Refusing to manage an unrecognized service unit");
    }
    match command {
        "service-stop" => {
            if query("KillMode")? != "process" {
                bail!(
                    "Refusing stop: effective KillMode must be process to preserve voyages; inspect service overrides"
                );
            }
            run(&["stop", UNIT])?;
            println!(
                "Supervisor stopped. Voyage processes and their state are preserved; restart Vessel to reconnect."
            );
        }
        "service-uninstall" => {
            if !matches!(query("ActiveState")?.as_str(), "inactive" | "failed") {
                bail!("Stop the supervisor with voyage-installer service-stop before uninstalling");
            }
            run(&["disable", UNIT])?;
            fs::remove_file(&path)?;
            run(&["daemon-reload"])?;
            println!(
                "Service unit removed. Versioned binaries, configuration, credentials and voyage data are retained. Surviving voyage processes are not stopped."
            );
        }
        _ => bail!("Unknown service operation"),
    }
    Ok(())
}
