//! The supervisor delegates only a typed update operation to its installed
//! updater. It never runs caller-supplied commands, paths, URLs or agent work.
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use voyage_protocol::vessel::VesselCommand;

fn installer(directory: &Path) -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    if super::runtime_storage::has_bound_layout(directory) {
        return system_installer(directory);
    }
    ensure!(cfg!(target_os = "linux"), "Remote updates require Linux");
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME missing")?);
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"))
        .join("voyage/vessel");
    ensure!(
        directory == state,
        "This Vessel is not managed by the user installer"
    );
    let bin = home
        .join(".local/share/voyage/install/current/bin")
        .canonicalize()?;
    ensure!(
        std::env::current_exe()?.canonicalize()? == bin.join("vessel"),
        "Running Vessel differs from the managed installation"
    );
    let updater = bin.join("voyage-installer");
    ensure!(updater.is_file(), "Installed updater unavailable");
    Ok(updater)
}

#[cfg(target_os = "linux")]
fn system_installer(directory: &Path) -> Result<PathBuf> {
    ensure!(
        unsafe { libc::geteuid() } == 0 && directory == Path::new("/var/lib/voyage/vessel"),
        "system updates require the managed root supervisor"
    );
    let _control = voyage_storage::protected_linux::RootDirectory::open(directory)?;
    let config = voyage_storage::protected_linux::RootDirectory::open(Path::new("/etc/voyage"))?;
    let record: Value =
        serde_json::from_slice(&config.read("system-install.json".as_ref(), 65536)?)?;
    let release = record["release"]
        .as_str()
        .context("system release missing")?;
    ensure!(
        record["schema_version"] == 1
            && record["phase"] == "active"
            && release.len() == 64
            && release.bytes().all(|b| b.is_ascii_hexdigit()),
        "system installation is not active"
    );
    let bin = Path::new("/opt/voyage/releases").join(release).join("bin");
    ensure!(
        std::env::current_exe()?.canonicalize()? == bin.join("vessel"),
        "running system Vessel differs from the managed installation"
    );
    let updater = bin.join("voyage-installer");
    super::launch::protected_binary(&updater)?;
    Ok(updater)
}
pub(super) fn supported(directory: &Path) -> bool {
    installer(directory).is_ok()
}

/// Explicit protocol admission for the current ordinary-user update controller.
/// Historical releases advertised remote_updates without its rollback/quarantine
/// contract; version strings alone must not authorize a current client to apply.
pub(super) fn verified_user_updates(directory: &Path) -> bool {
    !super::runtime_storage::has_bound_layout(directory) && supported(directory)
}

pub(super) fn running_release() -> Option<String> {
    let executable = std::env::current_exe().ok()?;
    let release = executable.parent()?.parent()?.file_name()?.to_str()?;
    (release.len() == 64 && release.bytes().all(|c| c.is_ascii_hexdigit()))
        .then(|| release.to_owned())
}

impl super::service::Supervisor {
    pub(super) async fn update(&self, command: VesselCommand) -> Result<Value> {
        let mut args = vec!["remote-update".to_string()];
        if super::runtime_storage::has_bound_layout(&self.directory) {
            args.push("system".into());
        }
        match command {
            VesselCommand::UpdatePrepare {
                operation_id,
                channel,
            } => {
                ensure!(!operation_id.is_nil(), "Invalid update identity");
                args.extend(["prepare".into(), operation_id.to_string(), channel]);
            }
            VesselCommand::UpdateStatus { operation_id } => {
                args.extend(["status".into(), operation_id.to_string()])
            }
            VesselCommand::UpdateApply {
                operation_id,
                release_id,
            } => {
                ensure!(
                    release_id.len() == 64 && release_id.bytes().all(|c| c.is_ascii_hexdigit()),
                    "Invalid approved release"
                );
                args.extend(["apply".into(), operation_id.to_string(), release_id]);
            }
            VesselCommand::UpdateDiscard { operation_id } => {
                args.extend(["discard".into(), operation_id.to_string()])
            }
            _ => anyhow::bail!("Invalid update operation"),
        }
        let mut command = tokio::process::Command::new(installer(&self.directory)?);
        if super::runtime_storage::has_bound_layout(&self.directory) {
            command.env_clear().env("PATH", "/usr/bin:/bin");
        }
        command
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let result = tokio::time::timeout(Duration::from_secs(25), command.output())
            .await
            .context("Update admission is unconfirmed; read the same operation status")??;
        ensure!(
            result.status.success() && result.stdout.len() <= 65536,
            "Updater refused the operation; check its existing status before retrying"
        );
        Ok(serde_json::from_slice(&result.stdout)?)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod remote_update_tests {
    #[test]
    fn ordinary_supervisor_cannot_acquire_system_updater_authority() {
        if unsafe { libc::geteuid() } == 0 {
            return;
        }
        let error =
            super::system_installer(std::path::Path::new("/var/lib/voyage/vessel")).unwrap_err();
        assert!(error.to_string().contains("managed root supervisor"));
    }
}
