//! Transactional managed-unit upgrades. Session owners never join supervisor cleanup.
use super::{command, files, readiness, unit};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    os::unix::fs::OpenOptionsExt,
    path::Path,
    time::{Duration, Instant},
};
struct Plan {
    layout: unit::Layout,
    content: String,
    previous: Option<String>,
    active: bool,
    enabled: bool,
    start: bool,
    invocation: String,
    restart: bool,
    rollback: Option<String>,
}
fn plan(bin: &Path, start: bool) -> Result<Plan> {
    let layout = unit::Layout::discover()?;
    files::check_path(bin, layout.uid)?;
    let content = unit::render(bin, &layout.state)?;
    let previous = unit::existing(&layout)?;
    unit::check_effective(&layout)?;
    let active = command::query("ActiveState")?;
    ensure!(
        matches!(active.as_str(), "active" | "inactive" | "failed"),
        "Service is transitioning; inspect it and retry after the pending operation"
    );
    ensure!(
        active != "active" || previous.is_some(),
        "Active service has no recognized owned unit"
    );
    let enabled = command::query("UnitFileState")?;
    ensure!(
        matches!(enabled.as_str(), "enabled" | "disabled" | "" | "not-found"),
        "Refusing unexpected systemd enablement state: {enabled}"
    );
    let mut rollback = previous.clone();
    let mut restart = active == "active" && previous.as_deref() != Some(&content);
    if active == "active" {
        let pid: u32 = command::query("MainPID")?
            .parse()
            .context("Active supervisor PID unavailable")?;
        ensure!(pid > 1, "Invalid active supervisor identity");
        let executable = fs::read_link(format!("/proc/{pid}/exe"))
            .context("Cannot inspect active supervisor executable")?;
        ensure!(
            executable.file_name().is_some_and(|name| name == "vessel"),
            "Active unit does not execute a recognized Vessel binary"
        );
        files::executable(&executable, layout.uid)?;
        if executable != bin.join("vessel") {
            restart = true;
            rollback = Some(unit::render(
                executable
                    .parent()
                    .context("Active supervisor path missing")?,
                &layout.state,
            )?);
        }
    }
    Ok(Plan {
        layout,
        content,
        previous,
        active: active == "active",
        enabled: enabled == "enabled",
        start,
        invocation: command::query("InvocationID")?,
        restart,
        rollback,
    })
}
pub(super) fn preview(bin: &Path, start: bool) -> Result<String> {
    let plan = plan(bin, start)?;
    Ok(format!(
        "{}\nUnit: {}\nPrivate state: {}\nActivation: {}\nIndependent voyage owners are preserved; supervisor replacement does not restart voyages.",
        plan.content,
        plan.layout.unit.display(),
        plan.layout.state.display(),
        if plan.start {
            "enable and start/restart"
        } else if plan.active {
            "preserve active state by restarting supervisor"
        } else {
            "leave inactive; manual activation"
        }
    ))
}
pub(super) fn configure(bin: &Path, start: bool, dry_run: bool) -> Result<()> {
    if dry_run {
        println!("{}", preview(bin, start)?);
        return Ok(());
    }
    let layout = unit::Layout::discover()?;
    files::directory(&layout.units, layout.uid, false)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(layout.units.join(".voyage-service.lock"))?;
    lock.try_lock()
        .context("Another service installation is active")?;
    let plan = plan(bin, start)?;
    for name in ["helm", "vessel", "voyage"] {
        files::executable(&bin.join(name), plan.layout.uid)?;
    }
    if let Some(previous) = &plan.previous {
        let old = unit::recognized(previous, &plan.layout.state)?;
        files::executable(&old.join("vessel"), plan.layout.uid)?;
        files::executable(&old.join("voyage"), plan.layout.uid)?;
    }
    let prior = if plan.active {
        // The replacement's public API may differ from the running service's.
        // Observe the old service with its own trusted helper before replacement;
        // readiness::wait below verifies the new service and retained owners.
        let active_bin = unit::recognized(
            plan.rollback
                .as_deref()
                .context("Active service unit missing")?,
            &plan.layout.state,
        )?;
        readiness::catalogue(&active_bin, &plan.layout.state).context(
            "Cannot inspect the active Vessel with its current helper; upgrade left unchanged",
        )?
    } else {
        if (plan.layout.state.join("process-http.json").exists()
            || plan.layout.state.join("vessel.sock").exists())
            && readiness::catalogue(bin, &plan.layout.state).is_ok()
        {
            anyhow::bail!(
                "A Vessel is already running outside this service; stop that supervisor explicitly before adopting its state"
            )
        }
        Vec::new()
    };
    files::directory(&plan.layout.state, plan.layout.uid, true)?;
    let changed = plan.previous.as_deref() != Some(&plan.content);
    if changed {
        files::replace(&plan.layout.unit, &plan.content, plan.previous.as_deref())?;
    }
    let applied = (|| -> Result<()> {
        command::systemctl(&["daemon-reload"])?;
        unit::check_effective(&plan.layout)?;
        if plan.start {
            command::systemctl(&["enable", unit::NAME])?;
        }
        if plan.start || plan.active {
            if plan.restart {
                command::systemctl(&["--no-block", "restart", unit::NAME])?;
            } else if !plan.active {
                command::systemctl(&["--no-block", "start", unit::NAME])?;
            }
            readiness::wait(
                bin,
                &plan.layout.state,
                &prior,
                plan.restart.then_some(plan.invocation.as_str()),
            )?;
        }
        Ok(())
    })();
    if let Err(error) = applied {
        if let Err(rollback) = restore(&plan, &prior) {
            return Err(error.context(format!("Service activation failed and rollback could not be confirmed: {rollback}; unit/data/releases retained for inspection")));
        }
        return Err(error.context("Service activation failed; previous unit and activation state restored, voyage owners preserved"));
    }
    println!(
        "Service unit: {}\nPrivate state: {}\nSupervisor: {}",
        plan.layout.unit.display(),
        plan.layout.state.display(),
        if plan.start || plan.active {
            "active; authenticated endpoint ready"
        } else {
            "inactive; not started"
        }
    );
    Ok(())
}
fn restore(plan: &Plan, prior: &[serde_json::Value]) -> Result<()> {
    ensure!(
        fs::read_to_string(&plan.layout.unit)? == plan.content,
        "Refusing rollback over independently changed unit contents"
    );
    unit::check_effective(&plan.layout)?;
    if matches!(
        command::query("ActiveState")?.as_str(),
        "active" | "activating" | "deactivating"
    ) {
        command::systemctl(&["--no-block", "stop", unit::NAME])?;
        wait_inactive()?;
    }
    if !plan.enabled {
        command::systemctl(&["disable", unit::NAME])?;
    }
    match &plan.rollback {
        Some(previous) => files::replace(&plan.layout.unit, previous, Some(&plan.content))?,
        None => {
            files::remove_reviewed(&plan.layout.unit, &plan.content)?;
        }
    }
    command::systemctl(&["daemon-reload"])?;
    if plan.enabled {
        command::systemctl(&["enable", unit::NAME])?;
    }
    if plan.active {
        command::systemctl(&["--no-block", "start", unit::NAME])?;
        let old = unit::recognized(
            plan.rollback.as_deref().context("previous unit missing")?,
            &plan.layout.state,
        )?;
        readiness::wait(&old, &plan.layout.state, prior, None)?;
    }
    Ok(())
}
pub(super) fn wait_inactive() -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if matches!(
            command::query("ActiveState")?.as_str(),
            "inactive" | "failed"
        ) {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Supervisor stop remains unconfirmed; independent owners are preserved"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
