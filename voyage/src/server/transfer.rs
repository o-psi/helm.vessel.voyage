use super::*;
use serde_json::Value;
use voyage_protocol::process::RuntimeCommand;
pub(super) async fn relinquish(state: &Arc<State>, command: RuntimeCommand) -> Result<Value> {
    let RuntimeCommand::Relinquish { transfer_id, .. } = &command else {
        anyhow::bail!("not relinquishment")
    };
    let transfer_id = *transfer_id;
    let _admission = state.admission.lock().await;
    ensure!(
        state.active.lock().await.is_none(),
        "transfer requires idle runtime"
    );
    state
        .controls
        .close_for_command(&state.owner, &command)
        .await?;
    let (receipt, artifact) = state.owner.relinquish(command).await?;
    let directory = state.directory.join("transfers");
    crate::attachment::journal::prepare_directory(directory.clone())?;
    let destination = directory.join(format!("{transfer_id}.json"));
    if destination.exists() {
        ensure!(
            super::bootstrap::read_private_artifact(&destination)? == artifact,
            "transfer artifact collision"
        );
    } else {
        use std::io::Write;
        let mut candidate = tempfile::NamedTempFile::new_in(&directory)?;
        candidate.write_all(&artifact)?;
        candidate.as_file().sync_all()?;
        candidate.persist_noclobber(&destination)?;
    }
    std::fs::File::open(&directory)?.sync_all()?;
    state.shutdown.cancel();
    Ok(receipt)
}
