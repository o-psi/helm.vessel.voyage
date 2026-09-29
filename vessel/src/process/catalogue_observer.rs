//! Bound journal parsing happens only after the helper drops to its execution identity.
use anyhow::{Context, Result, ensure};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;
use voyage_protocol::process::{CatalogueSummary, ProcessRegistration};

pub(super) async fn read(
    root: &Path,
    registration: &ProcessRegistration,
) -> Result<CatalogueSummary> {
    #[cfg(target_os = "linux")]
    {
        let identity = super::database::bound_observer_identity(root, registration).await?;
        let directory = super::runtime_storage::bound_directory(
            root,
            registration.session_id,
            identity.uid,
            identity.gid,
        )?;
        let binary = registration
            .executable
            .as_ref()
            .context("catalogue observer binary missing")?;
        super::launch::protected_binary(binary)?;
        let mut command = tokio::process::Command::new(binary);
        command
            .arg("observe-catalogue")
            .arg("--directory")
            .arg(directory)
            .arg("--session")
            .arg(registration.session_id.to_string())
            .current_dir(&identity.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        super::launch::configure_identity(command.as_std_mut(), &identity)?;
        // SQLite is untrusted input even after the privilege drop. Bound the
        // helper's address space and CPU as well as its output and wall time.
        unsafe {
            command.pre_exec(|| {
                for (resource, limit) in
                    [(libc::RLIMIT_AS, 512 * 1024 * 1024), (libc::RLIMIT_CPU, 2)]
                {
                    let limits = libc::rlimit {
                        rlim_cur: limit,
                        rlim_max: limit,
                    };
                    if libc::setrlimit(resource, &limits) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn()?;
        let result = tokio::time::timeout(Duration::from_secs(3), async {
            let mut bytes = Vec::new();
            child
                .stdout
                .take()
                .context("catalogue observer output missing")?
                .take(8193)
                .read_to_end(&mut bytes)
                .await?;
            ensure!(bytes.len() <= 8192, "catalogue observer exceeds bounds");
            ensure!(child.wait().await?.success(), "catalogue observer failed");
            decode(&bytes, registration.session_id)
        })
        .await;
        match result {
            Ok(Ok(summary)) => {
                ensure!(
                    super::database::bound_observer_identity(root, registration).await? == identity,
                    "catalogue execution identity changed during observation"
                );
                super::launch::validate_identity(&identity)?;
                Ok(summary)
            }
            failure => {
                let _ = child.kill().await;
                match failure {
                    Ok(Err(error)) => Err(error),
                    _ => anyhow::bail!("catalogue observer deadline elapsed"),
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("bound catalogue observation is unsupported on this host")
}

fn decode(bytes: &[u8], session: uuid::Uuid) -> Result<CatalogueSummary> {
    ensure!(bytes.len() <= 8192, "catalogue observer exceeds bounds");
    let summary: CatalogueSummary = serde_json::from_slice(bytes)?;
    ensure!(
        summary.session_id == session
            && summary.name.as_ref().is_none_or(|s| s.len() <= 512)
            && summary.model.len() <= 512
            && summary.created_at.as_ref().is_none_or(|s| s.len() <= 128)
            && summary
                .last_turn_end
                .as_ref()
                .is_none_or(|s| s.len() <= 128)
            && summary.run_state.as_ref().is_none_or(|s| s.len() <= 128),
        "invalid catalogue observer identity or metadata"
    );
    Ok(summary)
}

#[cfg(test)]
#[path = "catalogue_observer_tests.rs"]
mod tests;
