//! One-shot observations under the execution fence, without recovery or executors.
use super::*;
use serde_json::{Value, json};
use voyage_protocol::process::{
    PROCESS_PROTOCOL, RuntimeCommand, RuntimeRequest, RuntimeResponse, read_frame, write_frame,
};

#[derive(clap::Args)]
pub struct Args {
    #[arg(long)]
    pub directory: PathBuf,
}

/// The caller also bounds the child lifetime. No diagnostics enter the frame stream.
pub async fn run(args: Args) -> Result<()> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(25), async {
        let request: RuntimeRequest = read_frame(&mut tokio::io::stdin()).await?;
        let result = observe(&args.directory, &request).await;
        let response = RuntimeResponse {
            protocol: PROCESS_PROTOCOL,
            session_id: request.session_id,
            incarnation: request.incarnation,
            outcome_unknown: false,
            resumed_from: None,
            result: result.as_ref().cloned().unwrap_or(Value::Null),
            error: result
                .err()
                .map(|_| "suspended observation unavailable".into()),
        };
        write_frame(&mut tokio::io::stdout(), &response).await?;
        Ok::<_, anyhow::Error>(())
    })
    .await;
    // Malformed framing and timeout fail closed; callers see EOF, never diagnostics.
    if !matches!(result, Ok(Ok(()))) {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(unix)]
async fn observe(directory: &std::path::Path, request: &RuntimeRequest) -> Result<Value> {
    ensure!(directory.is_dir(), "missing runtime directory");
    let directory = crate::attachment::journal::prepare_directory(directory.to_path_buf())?;
    let startup = crate::attachment::journal::open_private_file(&directory.join("startup.lock"))?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match startup.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => return Err(anyhow::anyhow!("runtime startup owned: {error}")),
        }
    }
    let registration = super::transport::registration(&directory)?;
    authenticate(&registration, request)?;
    let check_retirement = || {
        check_retired(
            &directory,
            &registration,
            !matches!(request.command, RuntimeCommand::Resolve { .. }),
        )
    };
    check_retirement()?;
    // Read existing attribution directly: observation must not initialize an identity.
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Identity {
        version: u32,
        actor: LocalActor,
    }
    let identity: Identity = serde_json::from_slice(&bootstrap::read_private_artifact(
        &directory.join("identity/actor.json"),
    )?)?;
    ensure!(identity.version == 1, "unsupported actor identity");
    super::authorization::authorize_parts(identity.actor, &registration, request, &directory)?;
    let owner = open_owner(directory.join("journal"), registration.session_id).await?;
    check_retirement()?;
    let value = match &request.command {
        RuntimeCommand::Resolve {
            command_id,
            original,
        } => {
            if let Some(original) = original {
                super::dispatch::validate_public(
                    original,
                    &config(&owner, &registration, &directory).await?,
                )?;
            }
            let authorization = super::authorization::authorize_parts(
                identity.actor,
                &registration,
                request,
                &directory,
            )?;
            owner
                .resolve_process_command(
                    *command_id,
                    authorization.actor.principal_id,
                    original.clone(),
                )
                .await?
        }
        _ => inspect(&owner, &registration, &request.command, &directory).await?,
    };
    // Re-read authority immediately before releasing the fenced result.
    authenticate(&super::transport::registration(&directory)?, request)?;
    check_retirement()?;
    super::authorization::authorize_parts(identity.actor, &registration, request, &directory)?;
    Ok(value)
}

/// Cleanup evidence can precede the old owner's final guard drop by a few ticks.
#[cfg(unix)]
pub(crate) async fn open_owner(journal: PathBuf, session: Uuid) -> Result<ManagedSessionOwner> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match ManagedSessionOwner::open(journal.clone(), session).await {
            Ok(owner) => return Ok(owner),
            Err(error)
                if matches!(
                    error.downcast_ref::<std::fs::TryLockError>(),
                    Some(std::fs::TryLockError::WouldBlock)
                ) && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(unix)]
fn authenticate(registration: &ProcessRegistration, request: &RuntimeRequest) -> Result<()> {
    let token_equal = request.token.len() == registration.token.len()
        && request
            .token
            .bytes()
            .zip(registration.token.bytes())
            .fold(0u8, |different, (a, b)| different | (a ^ b))
            == 0;
    ensure!(
        request.protocol == PROCESS_PROTOCOL
            && request.session_id == registration.session_id
            && request.incarnation == registration.incarnation
            && token_equal,
        "runtime authentication rejected"
    );
    Ok(())
}

#[cfg(unix)]
pub(super) fn check_suspended(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
) -> Result<()> {
    check_retired(directory, registration, true)
}

#[cfg(unix)]
fn check_retired(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
    require_suspension: bool,
) -> Result<()> {
    ensure!(
        require_suspension
            || registration.state != voyage_protocol::process::ProcessState::Relinquished,
        "source ownership has been relinquished"
    );
    ensure!(
        matches!(std::fs::symlink_metadata(directory.join("runtime.sock")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound),
        "runtime endpoint present"
    );
    let evidence: Value = serde_json::from_slice(&bootstrap::read_private_artifact(
        &directory.join("stopped.json"),
    )?)?;
    ensure!(
        evidence["session_id"] == registration.session_id.to_string()
            && evidence["incarnation"] == registration.incarnation.to_string()
            && evidence["cleanup_observed"] == true
            && (!require_suspension || evidence["suspended"] == true),
        "clean retirement evidence missing"
    );
    Ok(())
}

#[cfg(unix)]
async fn inspect(
    owner: &ManagedSessionOwner,
    registration: &ProcessRegistration,
    command: &RuntimeCommand,
    directory: &std::path::Path,
) -> Result<Value> {
    match command.clone() {
        RuntimeCommand::Stop => Ok(json!({"status":"stopped","cleanup":"observed"})),
        RuntimeCommand::Health => Ok(json!({"pid":null,"session_id":registration.session_id,
            "incarnation":registration.incarnation,"suspended":true,
            "capabilities":["snapshot","history","message_chunk","run_output","submit",
                "receipt","resolve","cancel","steer","rename","set_model","set_access","decisions",
                "respond","archive","delete","branch","clear","compact","events","controls",
                "operator_tool","configure","workflow_submit","terminal","assignment_observe",
                "relinquish","stop"],"outbound":null,"decisions":"bounded_120_seconds"})),
        RuntimeCommand::Snapshot => {
            let mut snapshot = owner.process_snapshot().await?;
            let config = config(owner, registration, directory).await?;
            snapshot["access"] =
                crate::runtime_policy::RuntimePolicy::resolve(&config, &registration.workspace)
                    .ok()
                    .and_then(|p| serde_json::to_value(p.policy().access_mode()).ok())
                    .unwrap_or(Value::Null);
            snapshot["outbound"] = Value::Null;
            snapshot["decisions"] = owner.decisions(registration.incarnation).await?;
            snapshot["suspended"] = json!(true);
            Ok(snapshot)
        }
        RuntimeCommand::History {
            offset,
            limit,
            expected_revision,
        } => {
            owner
                .process_history(offset, limit, expected_revision)
                .await
        }
        RuntimeCommand::MessageChunk {
            index,
            offset,
            limit,
            expected_revision,
        } => {
            owner
                .process_message_chunk(index, offset, limit, expected_revision)
                .await
        }
        RuntimeCommand::RunOutput {
            run_id,
            offset,
            limit,
        } => owner.process_run_output(run_id, offset, limit).await,
        RuntimeCommand::Receipt { command_id } => Ok(owner
            .process_receipt(command_id)
            .await?
            .unwrap_or(json!({"command_id":command_id,"status":"unknown"}))),
        RuntimeCommand::Events {
            after,
            limit,
            wait_ms,
        } => {
            ensure!(wait_ms <= 10_000, "event wait exceeds ten seconds");
            // No producer exists until wake. Return immediately instead of fencing a new turn.
            owner.observations(after, limit).await
        }
        RuntimeCommand::Decisions => owner.decisions(registration.incarnation).await,
        RuntimeCommand::Controls { run_id, section } => {
            let mut config = config(owner, registration, directory).await?;
            config.model = owner.snapshot().await?.session.model;
            crate::build::set_resource_root(directory.join("resources"))?;
            if section == "models" {
                let value =
                    serde_json::to_value(vec![crate::provider::ModelInfo::minimal(config.model)])?;
                return Ok(
                    json!({"run_id":null,"section":section,"value":value,"execution":"idle","source":"saved_configuration","refresh":"next_run"}),
                );
            }
            controls::LiveControls::default()
                .inspect_or_idle(run_id, &section, &config, &registration.workspace)
                .await
        }
        _ => anyhow::bail!("command requires resumed runtime"),
    }
}

#[cfg(unix)]
async fn config(
    owner: &ManagedSessionOwner,
    registration: &ProcessRegistration,
    directory: &std::path::Path,
) -> Result<Config> {
    let saved = owner.saved_configuration().await?;
    let initial = if saved.is_none() {
        Journal::open(directory.join("journal"))?.initial_configuration(registration.session_id)?
    } else {
        None
    };
    let settings = saved.or(initial);
    let mut config = match settings {
        Some(settings) => serde_json::from_str::<crate::launch_config::LaunchConfig>(&settings)?
            .resolve(&registration.workspace)?,
        None => {
            bootstrap::load_config(registration.config_path.as_deref(), &registration.workspace)?
        }
    };
    bootstrap::limit_participant(&mut config, registration)?;
    Ok(config)
}

#[cfg(not(unix))]
async fn observe(_: &std::path::Path, _: &RuntimeRequest) -> Result<Value> {
    anyhow::bail!("private suspended observation unsupported on this platform")
}
