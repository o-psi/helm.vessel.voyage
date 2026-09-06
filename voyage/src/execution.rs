use crate::{
    Config, EventSink,
    attachment::{
        journal::{RunRecord, RunState},
        runtime::ManagedSessionOwner,
    },
    build::{build_authorized_agent_bundle, redactor},
};
use anyhow::{Context, Result, bail};
mod preparation;
use std::{path::PathBuf, sync::Arc, time::Duration};
async fn poll_local_cancellation<F, Fut>(
    mut read: F,
    stopped: &tokio_util::sync::CancellationToken,
) -> anyhow::Result<bool>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<bool>>,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if stopped.is_cancelled() {
                return Ok(false);
            }
            match read().await {
                Err(error)
                    if error
                        .downcast_ref::<rusqlite::Error>()
                        .and_then(rusqlite::Error::sqlite_error_code)
                        == Some(rusqlite::ErrorCode::DatabaseBusy) =>
                {
                    // Independent managed commands can briefly hold the journal.
                    // Retry only this read, with the same run identity, after its
                    // transaction has returned. Other errors remain fail-closed.
                    // No blocking read is in flight here. A finished foreground
                    // run can stop its watcher without manufacturing a timeout.
                    tokio::select! {
                        biased;
                        _ = stopped.cancelled() => return Ok(false),
                        _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                    }
                }
                result => return result,
            }
        }
    })
    .await
    .context("durable cancellation polling deadline elapsed")?
}

async fn await_execution<T>(
    execution: impl std::future::Future<Output = T>,
    cancel: tokio_util::sync::CancellationToken,
    interrupt: impl std::future::Future<Output = ()>,
    grace: Duration,
) -> Option<T> {
    let mut execution = std::pin::pin!(execution);
    tokio::select! { biased;
        result=&mut execution=>Some(result),
        _=interrupt=> { cancel.cancel(); tokio::time::timeout(grace,&mut execution).await.ok() },
        _=cancel.cancelled()=>tokio::time::timeout(grace,&mut execution).await.ok(),
    }
}

/// Actual durable terminal state and independent owned-resource cleanup observation.
/// An unconfirmed result retains the durable admission blocker.
pub struct ManagedExecution {
    pub actual: RunRecord,
    pub cleanup_observed: bool,
    pub construction_failed: bool,
}
#[allow(clippy::too_many_arguments)]
pub async fn execute_admitted(
    owner: &ManagedSessionOwner,
    run: &mut crate::attachment::runtime::RunOwner,
    config: &Config,
    workspace: PathBuf,
    sink: Arc<dyn EventSink>,
    cancel: tokio_util::sync::CancellationToken,
    interrupt: impl std::future::Future<Output = ()>,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    approver: Option<Arc<dyn crate::tools::Approver>>,
) -> Result<ManagedExecution> {
    execute_admitted_with_controls(
        owner, run, config, workspace, sink, cancel, interrupt, authority, approver, None, None,
    )
    .await
}
#[allow(clippy::too_many_arguments)]
pub async fn execute_admitted_with_controls(
    owner: &ManagedSessionOwner,
    run: &mut crate::attachment::runtime::RunOwner,
    config: &Config,
    workspace: PathBuf,
    sink: Arc<dyn EventSink>,
    cancel: tokio_util::sync::CancellationToken,
    interrupt: impl std::future::Future<Output = ()>,
    authority: Option<Arc<dyn crate::policy::ExecutionAuthority>>,
    approver: Option<Arc<dyn crate::tools::Approver>>,
    controls: Option<Arc<crate::server::controls::LiveControls>>,
    operator: Option<(String, serde_json::Value)>,
) -> Result<ManagedExecution> {
    use tokio_util::sync::CancellationToken;
    let run_id = run.record().await?.id;
    let host_reservation = match crate::host_resources::Reservation::acquire(
        "executors",
        run_id,
        config.subagent_max_concurrency.saturating_add(1),
    ) {
        Ok(reservation) => reservation,
        Err(_) => {
            let actual = run.fail_before_execution().await?;
            run.confirm_local_cleanup_observed().await?;
            return Ok(ManagedExecution {
                actual,
                cleanup_observed: true,
                construction_failed: true,
            });
        }
    };
    let prepared = async {
        if authority.is_some() {
            run.configure_remote_redaction(redactor(config)).await?;
        }
        let record = run.record().await?;
        crate::participant::ParticipantTool::configured(
            owner.clone(),
            run_id,
            record.principal_id,
            config,
        )
        .await
    }
    .await;
    let participant_tool = match prepared {
        Ok(tool) => tool,
        Err(_) => {
            let actual = run.fail_before_execution().await?;
            host_reservation.release_observed()?;
            run.confirm_local_cleanup_observed().await?;
            return Ok(ManagedExecution {
                actual,
                cleanup_observed: true,
                construction_failed: true,
            });
        }
    };
    let retained_terminal = match &controls {
        Some(controls) => controls.retained_tool().await,
        None => None,
    };
    let resources = match build_authorized_agent_bundle(
        config,
        workspace,
        false,
        Some(sink),
        authority.clone(),
        approver,
        crate::build::BuildResources {
            extra_tool: participant_tool,
            terminal_manager: retained_terminal,
        },
    )
    .await
    {
        Ok(resources) => resources,
        Err(_) => {
            let actual = run.fail_before_execution().await?;
            return Ok(ManagedExecution {
                actual,
                cleanup_observed: false,
                construction_failed: true,
            });
        }
    };
    if let Some(controls) = &controls {
        if controls
            .retain_root(owner, run_id, &resources.agent)
            .await
            .is_err()
        {
            return preparation::reject_constructed(
                owner,
                run,
                &resources,
                controls,
                &host_reservation,
            )
            .await;
        }
        controls
            .open(
                run_id,
                resources.agent.clone(),
                resources.subagents.clone(),
                cancel.clone(),
            )
            .await;
    }
    let stop_watch = CancellationToken::new();
    let _stop_on_drop = stop_watch.clone().drop_guard();
    let watcher = {
        let owner = owner.clone();
        let done = stop_watch.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! { biased;
                    _=done.cancelled()=> return Ok::<_,anyhow::Error>(()),
                    _=tokio::time::sleep(Duration::from_millis(50))=>{}
                }
                if authority
                    .as_ref()
                    .is_some_and(|authority| authority.check().is_err())
                {
                    cancel.cancel();
                    return Ok(());
                }
                match poll_local_cancellation(|| owner.local_cancel_requested(run_id), &done).await
                {
                    Ok(true) => {
                        cancel.cancel();
                        return Ok(());
                    }
                    Ok(false) => {}
                    _ => {
                        cancel.cancel();
                        bail!("durable cancellation polling failed");
                    }
                }
            }
        })
    };
    let result = await_execution(
        async {
            if let Some((name, arguments)) = operator {
                run.start_operator_scope(&resources.agent).await?;
                let output = resources
                    .agent
                    .operator_tool(owner.session_id(), run_id, cancel.clone(), &name, arguments)
                    .await;
                let output = output
                    .map(|text| redactor(config).redact(text))
                    .map_err(|_| "operator tool failed".to_owned());
                match output {
                    Ok(text) if !cancel.is_cancelled() => {
                        let lease = resources
                            .agent
                            .operator_lease(owner.session_id(), run_id)
                            .await?;
                        run.finish_operator_scoped(text, lease).await?;
                    }
                    other => {
                        run.finish_operator(other, cancel.is_cancelled()).await?;
                    }
                }
                Ok::<(), anyhow::Error>(())
            } else {
                run.execute(&resources.agent, cancel.clone(), None).await?;
                Ok(())
            }
        },
        cancel.clone(),
        interrupt,
        Duration::from_secs(15),
    )
    .await;
    stop_watch.cancel();
    let watcher_ok = matches!(
        tokio::time::timeout(Duration::from_secs(6), watcher).await,
        Ok(Ok(Ok(())))
    );
    let controls_observed = match &controls {
        Some(controls) => {
            let closed = controls.close().await;
            closed && controls.retire_empty(owner).await.is_ok()
        }
        None => true,
    };
    let retained = resources
        .resources
        .as_ref()
        .context("missing managed resources")?
        .close()?;
    let children_observed =
        tokio::time::timeout(Duration::from_secs(15), resources.subagents.shutdown())
            .await
            .is_ok();
    let session_terminal = match &controls {
        Some(controls) => controls.retained_tool().await,
        None => None,
    };
    let terminal_reports = tokio::time::timeout(
        Duration::from_secs(15),
        futures_util::future::join_all(
            retained
                .terminals
                .iter()
                .filter(|terminal| {
                    !session_terminal
                        .as_ref()
                        .is_some_and(|root| root.same_manager(terminal))
                })
                .map(|terminals| terminals.shutdown(Duration::from_secs(10))),
        ),
    )
    .await;
    let terminals_observed = terminal_reports
        .as_ref()
        .is_ok_and(|reports| reports.iter().all(|report| report.observation_complete));
    let shell_reports = tokio::time::timeout(
        Duration::from_secs(15),
        futures_util::future::join_all(
            retained
                .shells
                .iter()
                .map(|shell| shell.shutdown(Duration::from_secs(10))),
        ),
    )
    .await;
    let shells_observed = shell_reports
        .as_ref()
        .is_ok_and(|reports| reports.iter().all(|report| report.observation_complete));
    let mcp_reports = tokio::time::timeout(
        Duration::from_secs(15),
        futures_util::future::join_all(retained.mcp.iter().map(|server| server.shutdown())),
    )
    .await;
    let mcp_observed = retained.mcp.iter().all(|server| server.observed())
        && mcp_reports
            .as_ref()
            .is_ok_and(|reports| reports.iter().all(Result::is_ok));
    let compatibility_observed = crate::provider::shutdown_compatibility().await.is_ok();
    let actual = run.record().await?;
    let terminal_persisted = !matches!(actual.state, RunState::Accepted | RunState::Running);
    let observed = terminal_persisted
        && compatibility_observed
        && controls_observed
        && mcp_observed
        && children_observed
        && terminals_observed
        && shells_observed
        && result.is_some()
        && watcher_ok;
    drop(resources);
    if observed {
        host_reservation.release_observed()?;
        run.confirm_local_cleanup_observed().await?;
    }
    Ok(ManagedExecution {
        actual,
        cleanup_observed: observed,
        construction_failed: false,
    })
}
