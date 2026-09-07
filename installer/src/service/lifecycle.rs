use super::{command, files, systemd, unit};
use anyhow::{Result, ensure};
pub fn manage(operation: &str) -> Result<()> {
    let layout = unit::Layout::discover()?;
    let previous = unit::existing(&layout)?;
    if operation == "service-status" {
        println!(
            "Vessel supervisor: {}\nPID: {}\nUnit: {}\nState directory: {}",
            command::query("ActiveState")?,
            command::query("MainPID")?,
            layout.unit.display(),
            layout.state.display()
        );
        println!("Individual voyage liveness requires authenticated runtime inspection.");
        return Ok(());
    }
    ensure!(
        previous.is_some(),
        "No recognized installed Voyage service unit"
    );
    unit::check_effective(&layout)?;
    match operation {
        "service-stop" => {
            command::systemctl(&["--no-block", "stop", unit::NAME])?;
            systemd::wait_inactive()?;
            println!("Supervisor stopped; independent voyages and their state are preserved.");
        }
        "service-uninstall" => {
            ensure!(
                matches!(
                    command::query("ActiveState")?.as_str(),
                    "inactive" | "failed"
                ),
                "Stop the supervisor before uninstalling its service"
            );
            command::systemctl(&["disable", unit::NAME])?;
            files::check_path(&layout.unit, layout.uid)?;
            files::remove_reviewed(&layout.unit, previous.as_deref().unwrap_or_default())?;
            command::systemctl(&["daemon-reload"])?;
            println!(
                "Service unit removed. Versioned binaries, credentials, configuration and voyage state retained."
            );
        }
        _ => anyhow::bail!("Unknown service operation"),
    };
    Ok(())
}
