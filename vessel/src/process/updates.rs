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
pub(super) fn supported(directory: &Path) -> bool {
    installer(directory).is_ok()
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
