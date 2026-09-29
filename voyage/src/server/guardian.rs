//! Linux subreaper, independent of both Vessel and the canonical session owner.
//! Kernel child ownership, not remembered PID numbers, authorizes cleanup.
use super::*;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::{
        path::Path,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Evidence {
        session_id: Uuid,
        incarnation: Uuid,
        boot_id: Uuid,
        cleanup_observed: bool,
    }
    fn boot() -> Result<Uuid> {
        Ok(std::fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .parse()?)
    }
    fn marker(directory: &Path, incarnation: Uuid) -> PathBuf {
        directory.join(format!("guardian-{incarnation}.json"))
    }
    pub(super) fn observed(directory: &Path, registration: &ProcessRegistration) -> Result<bool> {
        let path = marker(directory, registration.incarnation);
        if !path.try_exists()? {
            return Ok(false); // Legacy owners never had a guardian.
        }
        let lock = crate::attachment::journal::open_private_file(&directory.join("guardian.lock"))?;
        if lock.try_lock().is_err() {
            return Ok(false); // A live guardian may still be cleaning up.
        }
        let saved: Evidence = serde_json::from_slice(&bootstrap::read_private_artifact(&path)?)?;
        ensure!(
            saved.session_id == registration.session_id
                && saved.incarnation == registration.incarnation,
            "guardian evidence identity mismatch"
        );
        // A reboot proves local descendants from that boot cannot still run.
        // Neither proof establishes completion of a remote effect/assignment.
        Ok(saved.cleanup_observed || saved.boot_id != boot()?)
    }
    pub(super) fn run(args: ServeArgs) -> Result<()> {
        let directory = crate::attachment::journal::prepare_directory(args.directory.clone())?;
        let guardian =
            crate::attachment::journal::open_private_file(&directory.join("guardian.lock"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match guardian.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(anyhow::anyhow!("runtime guardian still owned: {error}")),
            }
        }
        let startup =
            crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match startup.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(anyhow::anyhow!("runtime startup still owned: {error}")),
            }
        }
        let registration = super::super::transport::registration(&directory)?;
        ensure!(
            registration.session_id == args.session
                && registration.incarnation == args.incarnation
                && registration.workspace == args.workspace
                && registration.config_path == args.config
                && registration.state != voyage_protocol::process::ProcessState::Relinquished,
            "guardian registration mismatch"
        );
        // Never adopt an existing unguarded incarnation: its escaped children
        // would not become ours. Existing journals require a proved predecessor.
        if directory.join("journal/journal.sqlite3").try_exists()? {
            let previous = registration
                .restart_from
                .context("existing voyage needs fenced recovery before guardian launch")?;
            let proved = ["stopped.json", "recovered.json"].iter().any(|name| {
                bootstrap::read_private_artifact(&directory.join(name))
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .is_some_and(|saved| {
                        saved["session_id"] == args.session.to_string()
                            && saved["incarnation"] == previous.to_string()
                            && (saved["cleanup_observed"] == true
                                || (saved["restart_permitted"] == true
                                    && matches!(
                                        saved["cleanup_disposition"].as_str(),
                                        Some(
                                            "observed"
                                                | "operator_attested"
                                                | "unresolved_retained"
                                        )
                                    )))
                    })
            });
            ensure!(proved, "previous incarnation cleanup is unconfirmed");
        }
        let owner_path = directory
            .join("journal")
            .join(format!("{}.execution.lock", args.session));
        let owner_fence = if owner_path.try_exists()? {
            let file = crate::attachment::journal::open_private_file(&owner_path)?;
            file.try_lock().context("session execution still owned")?;
            Some(file)
        } else {
            None
        };
        ensure!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } == 0,
            "child cleanup guardian unavailable"
        );
        let path = marker(&directory, args.incarnation);
        ensure!(!path.try_exists()?, "guardian incarnation already launched");
        let mut evidence = Evidence {
            session_id: args.session,
            incarnation: args.incarnation,
            boot_id: boot()?,
            cleanup_observed: false,
        };
        super::super::recovery::persist(&path, &serde_json::to_value(&evidence)?)?;
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("serve")
            .arg("--directory")
            .arg(&directory)
            .arg("--session")
            .arg(args.session.to_string())
            .arg("--incarnation")
            .arg(args.incarnation.to_string())
            .arg("--workspace")
            .arg(&args.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Some(config) = &args.config {
            command.arg("--config").arg(config);
        }
        // The startup gate keeps new owners out while handing off the fence.
        drop(owner_fence);
        let child = command.spawn();
        drop(startup); // The child acquires the same gate before opening its journal.
        if let Ok(mut child) = child {
            child.wait()?;
        }
        // Reparenting includes double-forked and setsid descendants.
        voyage_storage::descendant_cleanup::drain(Duration::from_secs(10))?;
        evidence.cleanup_observed = true;
        super::super::recovery::persist(&path, &serde_json::to_value(&evidence)?)?;
        Ok(())
    }
}

pub fn run(args: ServeArgs) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::run(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        anyhow::bail!("automatic process cleanup requires Linux");
    }
}
pub(super) fn observed(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::observed(directory, registration)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (directory, registration);
        Ok(false)
    }
}
