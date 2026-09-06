//! Offline recovery acquires the same OS fence; it never constructs executors.
use super::*;
#[derive(clap::Args)]
pub struct RecoverArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub session: Uuid,
    #[arg(long)]
    pub incarnation: Uuid,
    #[arg(long)]
    pub command_id: Uuid,
    #[arg(long)]
    pub acknowledge_cleanup: Option<Uuid>,
    #[arg(long = "acknowledge-resource")]
    pub acknowledge_resources: Vec<Uuid>,
    #[arg(long)]
    pub reconcile_tools: Option<Uuid>,
    #[arg(long)]
    pub expected_revision: Option<u64>,
}
#[cfg(unix)]
pub async fn recover(args: RecoverArgs) -> Result<serde_json::Value> {
    let directory = crate::attachment::journal::prepare_directory(args.directory)?;
    let registration = super::transport::registration(&directory)?;
    ensure!(
        registration.session_id == args.session
            && registration.incarnation == args.incarnation
            && !args.command_id.is_nil(),
        "recovery identity mismatch"
    );
    let startup = crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
    startup.try_lock().context("runtime startup owned")?;
    let owner = ManagedSessionOwner::open(directory.join("journal"), args.session).await?;
    owner.initialize_process_commands().await?;
    owner.initialize_session_resources().await?;
    let request = serde_json::json!({"command_id":args.command_id,"acknowledge_cleanup":args.acknowledge_cleanup,"acknowledge_resources":args.acknowledge_resources,"reconcile_tools":args.reconcile_tools,"expected_revision":args.expected_revision});
    let receipts = crate::attachment::journal::prepare_directory(directory.join("recoveries"))?;
    let marker = receipts.join(format!("{}.json", args.command_id));
    if marker.exists() {
        let saved: serde_json::Value =
            serde_json::from_slice(&bootstrap::read_private_artifact(&marker)?)?;
        if saved["command_id"] == args.command_id.to_string() {
            ensure!(
                saved["request"] == request,
                "recovery command payload conflict"
            );
            if saved["restart_permitted"] == true {
                persist(&directory.join("recovered.json"), &saved)?;
                match std::fs::remove_file(directory.join("runtime.sock")) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                std::fs::File::open(&directory)?.sync_all()?;
                return Ok(saved);
            }
        }
    }
    if !marker.exists() {
        persist(
            &marker,
            &serde_json::json!({"command_id":args.command_id,"request":request}),
        )?;
    }
    owner.recover_interrupted().await?;
    let actor = LocalActorStore::open(&directory.join("identity"))?.identity()?;
    let binding = if matches!(
        registration.initialize,
        Some(voyage_protocol::process::RuntimeInitialization::Outbound { .. })
    ) {
        let journal = Journal::open(directory.join("journal"))?;
        Some(journal.remote_local_binding(&actor)?.1)
    } else {
        None
    };
    if let Some(run) = args.acknowledge_cleanup {
        if let Some(binding) = &binding {
            owner
                .attest_remote_cleanup(binding.clone(), run, actor)
                .await?;
        } else {
            owner.attest_local_cleanup(run, actor).await?;
        }
    }
    for resource in &args.acknowledge_resources {
        owner.attest_session_resource(*resource, actor).await?;
    }
    if let Some(run) = args.reconcile_tools {
        let request = crate::attachment::journal::LocalReconcileRequest {
            session_id: args.session,
            run_id: run,
            installation_id: actor.installation_id,
            principal_id: actor.principal_id,
            expected_revision: args
                .expected_revision
                .context("tool reconciliation requires expected revision")?,
        };
        if let Some(binding) = binding {
            owner.reconcile_remote_tools(binding, request).await?;
        } else {
            owner.reconcile_local_tools(request).await?;
        }
    }
    let snapshot = owner.process_snapshot().await?;
    let resources = owner.session_resources().await?;
    if !snapshot["pending_cleanup_run"].is_null()
        || !resources.as_array().is_some_and(Vec::is_empty)
    {
        let pending = serde_json::json!({"session_id":args.session,"incarnation":args.incarnation,"command_id":args.command_id,"request":request,"restart_permitted":false,"pending_cleanup_run":snapshot["pending_cleanup_run"],"session_resources":resources,"revision":snapshot["revision"]});
        persist(&marker, &pending)?;
        return Ok(pending);
    }
    let result = serde_json::json!({"session_id":args.session,"incarnation":args.incarnation,"command_id":args.command_id,"request":request,"cleanup_disposition":if owner.has_cleanup_attestation().await?{"operator_attested"}else{"observed"},"restart_permitted":true,"revision":snapshot["revision"]});
    persist(&marker, &result)?;
    persist(&directory.join("recovered.json"), &result)?;
    // The OS fence excludes a live owner; stale endpoints are now safe to remove.
    match std::fs::remove_file(directory.join("runtime.sock")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    std::fs::File::open(&directory)?.sync_all()?;
    Ok(result)
}

fn persist(path: &std::path::Path, value: &serde_json::Value) -> Result<()> {
    use std::io::Write;
    let parent = path.parent().context("recovery receipt parent missing")?;
    let mut candidate = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut candidate, value)?;
    candidate.flush()?;
    candidate.as_file().sync_all()?;
    candidate.persist(path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
pub async fn recover(_args: RecoverArgs) -> Result<serde_json::Value> {
    anyhow::bail!("private process recovery unsupported on this platform")
}
