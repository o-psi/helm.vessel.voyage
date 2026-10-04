//! Owner-only reviewed host updates. No session/tool authority or automatic apply.
use super::{App, observe::Update, state::Route};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use uuid::Uuid;
use voyage_protocol::vessel::VesselCommand;

#[cfg(unix)]
fn receipt_root() -> std::path::PathBuf {
    let root =
        crate::process_client::cli::default_directory().with_file_name("helm-update-reviews");
    #[cfg(test)]
    let root = super::account_test_support::root("helm-update-reviews", root);
    root
}

#[cfg(unix)]
fn retain(connection: Uuid, ids: [Uuid; 2]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    let root = receipt_root();
    std::fs::create_dir_all(root.parent().context("update receipt parent unavailable")?)?;
    match std::fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    };
    crate::process_client::local::check_private_directory(&root)?;
    let name = root.join(format!("{}-{}.json", connection, "update"));
    if let Ok(metadata) = std::fs::symlink_metadata(&name) {
        ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "unsafe update receipt"
        );
    }
    let temporary = root.join(format!("{}.tmp", Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(&ids)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, &name)?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(unix)]
fn restored(connection: Uuid) -> Result<Option<[Uuid; 2]>> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let root = receipt_root();
    if !root.try_exists()? {
        return Ok(None);
    }
    crate::process_client::local::check_private_directory(&root)?;
    let name = root.join(format!("{}-{}.json", connection, "update"));
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(name)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && metadata.len() <= 1024,
        "unsafe update receipt"
    );
    let mut bytes = Vec::new();
    file.take(1025).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024, "update receipt exceeds bound");
    let ids: [Uuid; 2] = serde_json::from_slice(&bytes)?;
    ensure!(ids.iter().all(|id| !id.is_nil()), "invalid update receipt");
    Ok(Some(ids))
}
#[cfg(not(unix))]
fn retain(_connection: Uuid, _ids: [Uuid; 2]) -> Result<()> {
    anyhow::bail!("private update receipt storage is unavailable on this platform")
}
#[cfg(not(unix))]
fn restored(_connection: Uuid) -> Result<Option<[Uuid; 2]>> {
    Ok(None)
}

fn admitted(caps: &Value) -> Result<()> {
    ensure!(
        caps["remote_updates"] == true,
        "This Vessel has no owner-authorized updater. Older hosts require an administrator bootstrap; your draft is retained."
    );
    ensure!(
        caps["features"]
            .as_array()
            .is_some_and(|v| v.iter().any(|f| f == "verified_user_updates")),
        "This updater lacks verified rollback/recovery admission. Do not apply; an administrator must bootstrap the host."
    );
    Ok(())
}
fn ready(receipt: &Value, operation: Uuid, release: &str, caps: &Value) -> Result<()> {
    ensure!(
        receipt["operation_id"] == operation.to_string(),
        "Update receipt identity changed"
    );
    ensure!(
        receipt["phase"] == "ready",
        "Update is not ready; observe the retained receipt, never replay an uncertain apply"
    );
    ensure!(
        release.len() == 64
            && release.bytes().all(|b| b.is_ascii_hexdigit())
            && receipt["release_id"] == release,
        "Approval must name the exact reviewed release hash"
    );
    ensure!(
        receipt["current_release"] == caps["running_release"]
            && caps["running_release"].as_str().is_some(),
        "Installation changed since review; prepare a fresh review after resolving this operation"
    );
    ensure!(
        receipt["expires_at"]
            .as_u64()
            .is_some_and(|expiry| expiry > chrono::Utc::now().timestamp().max(0) as u64),
        "Review expired; discard and prepare a fresh review"
    );
    Ok(())
}
async fn perform(
    client: &crate::process_client::transport::Client,
    words: Vec<String>,
) -> Result<String> {
    let caps = client.request(VesselCommand::Capabilities).await?;
    admitted(&caps)?;
    let vessel = caps["vessel_id"]
        .as_str()
        .context("Vessel identity unavailable")?;
    if let Some(connection) = client.managed() {
        ensure!(
            vessel == connection.vessel_id.to_string(),
            "Vessel identity changed; update refused"
        );
    }
    let vessel_id = Uuid::parse_str(vessel)?;
    let retained = restored(client.id())?;
    if let Some(ids) = retained {
        ensure!(
            ids[0] == vessel_id,
            "Retained update belongs to another Vessel; do not replay"
        );
    }
    let command = match words
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["status"] => VesselCommand::UpdateStatus {
            operation_id: retained.map(|ids| ids[1]).unwrap_or(Uuid::nil()),
        },
        ["status", operation] => VesselCommand::UpdateStatus {
            operation_id: Uuid::parse_str(operation)?,
        },
        ["prepare", "nightly"] => {
            if let Some(ids) = retained {
                let previous = client
                    .request(VesselCommand::UpdateStatus {
                        operation_id: ids[1],
                    })
                    .await?;
                ensure!(
                    previous["operation_id"] == ids[1].to_string()
                        && matches!(
                            previous["phase"].as_str(),
                            Some("complete" | "discarded" | "failed")
                        ),
                    "Retained update unresolved; /update status before a fresh review"
                );
            }
            VesselCommand::UpdatePrepare {
                operation_id: Uuid::new_v4(),
                channel: "nightly".into(),
            }
        }
        ["discard", operation] => VesselCommand::UpdateDiscard {
            operation_id: Uuid::parse_str(operation)?,
        },
        ["approve", operation, release] => {
            let operation = Uuid::parse_str(operation)?;
            ensure!(!operation.is_nil(), "Explicit operation required");
            let receipt = client
                .request(VesselCommand::UpdateStatus {
                    operation_id: operation,
                })
                .await?;
            ready(&receipt, operation, release, &caps)?;
            VesselCommand::UpdateApply {
                operation_id: operation,
                release_id: (*release).into(),
            }
        }
        _ => anyhow::bail!(
            "/update status [UUID] | prepare nightly | approve UUID EXACT_RELEASE_HASH | discard UUID. Review status before explicit approval; no automatic update."
        ),
    };
    let requested = match &command {
        VesselCommand::UpdatePrepare { operation_id, .. }
        | VesselCommand::UpdateStatus { operation_id }
        | VesselCommand::UpdateApply { operation_id, .. }
        | VesselCommand::UpdateDiscard { operation_id } => *operation_id,
        _ => unreachable!(),
    };
    if !requested.is_nil() {
        retain(client.id(), [vessel_id, requested])?;
    }
    let receipt = client.request(command).await.with_context(|| format!("Update admission unconfirmed. Reconnect and /update status {requested}; do not repeat apply or prepare."))?;
    if !requested.is_nil() {
        ensure!(
            receipt["operation_id"] == requested.to_string(),
            "Update receipt identity conflict"
        );
    }
    if receipt["phase"] == "complete" {
        let fresh = client.request(VesselCommand::Capabilities).await?;
        ensure!(
            fresh["vessel_id"] == caps["vessel_id"]
                && fresh["running_release"] == receipt["release_id"]
                && fresh["running_release"].as_str().is_some(),
            "Completed historical receipt is not current activation. Reconnect and verify host identity/release before setup."
        );
    }
    // Only the structured public receipt crosses this boundary, not acquisition logs.
    Ok(format!(
        "Vessel update review: {}. Explicit consent: /update approve UUID EXACT_RELEASE_HASH. Setup draft retained; reconnect then /update status UUID after apply.",
        serde_json::to_string(&receipt)?
    ))
}
impl App {
    pub(super) fn remote_update_command(&mut self, route: Route, text: &str) -> Result<()> {
        ensure!(
            self.clients.available(route),
            "Reconnect, then observe /update status; never replay an uncertain update"
        );
        let client = self.clients[route].clone();
        let sender = self.sender.clone();
        let words = text.split_whitespace().skip(1).map(str::to_owned).collect();
        self.status =
            "Reading owner update readiness; no update is automatically approved. Draft retained."
                .into();
        tokio::spawn(async move {
            let result = perform(&client, words).await.map_err(|e| e.to_string());
            let _ = sender.send(Update::RemoteUpdate { route, result }).await;
        });
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[cfg(unix)]
    #[test]
    fn reconnect_restores_exact_operation_and_refuses_unsafe_local_receipts() {
        use std::os::unix::fs::PermissionsExt;
        let _fixture = super::super::account_test_support::Fixture::new();
        let connection = Uuid::new_v4();
        let ids = [Uuid::new_v4(), Uuid::new_v4()];
        assert!(restored(connection).unwrap().is_none());
        retain(connection, ids).unwrap();
        assert_eq!(restored(connection).unwrap(), Some(ids));
        let path = receipt_root().join(format!("{}-{}.json", connection, "update"));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(restored(connection).is_err());
        assert!(retain(connection, ids).is_err());
    }
    #[test]
    fn owner_capability_and_verified_contract_are_both_required() {
        assert!(
            admitted(&json!({"remote_updates":false,"features":["verified_user_updates"]}))
                .is_err()
        );
        assert!(admitted(&json!({"remote_updates":true,"features":[]})).is_err());
        assert!(
            admitted(&json!({"remote_updates":true,"features":["verified_user_updates"]})).is_ok()
        );
    }
    #[test]
    fn approval_pins_operation_release_original_installation_and_expiry() {
        let id = Uuid::new_v4();
        let release = "b".repeat(64);
        let caps = json!({"running_release":"a".repeat(64)});
        let receipt = json!({"operation_id":id,"phase":"ready","release_id":release,"current_release":"a".repeat(64),"expires_at":u64::MAX});
        assert!(ready(&receipt, id, &release, &caps).is_ok());
        for (key, value) in [
            ("operation_id", json!(Uuid::new_v4())),
            ("phase", json!("unconfirmed")),
            ("release_id", json!("c".repeat(64))),
            ("current_release", json!("c".repeat(64))),
            ("expires_at", json!(0)),
        ] {
            let mut changed = receipt.clone();
            changed[key] = value;
            assert!(ready(&changed, id, &release, &caps).is_err());
        }
    }
}
