use crate::{
    Config, EventSink,
    attachment::{journal::RunRecord, runtime::ManagedSessionOwner},
    build::{build_authorized_agent_bundle, redactor},
};
use anyhow::{Result, bail};
pub mod cleanup;
mod preparation;
use cleanup::{CleanupSlot, PendingCleanup};
use std::{path::PathBuf, sync::Arc, time::Duration};
async fn poll_local_cancellation<F, Fut>(
    mut read: F,
    stopped: &tokio_util::sync::CancellationToken,
    cancel: &tokio_util::sync::CancellationToken,
    monitor_failed: &std::sync::atomic::AtomicBool,
) -> anyhow::Result<bool>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<bool>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if stopped.is_cancelled() {
            return Ok(false);
        }
        let mut reading = std::pin::pin!(read());
        let result = match tokio::time::timeout_at(deadline, &mut reading).await {
            Ok(result) => result,
            Err(_) => {
                monitor_failed.store(true, std::sync::atomic::Ordering::Release);
                cancel.cancel();
                // Keep the exact read alive until its blocking worker is observed.
                // Cancellation of the run is independent of observing this task exit.
                let _ = reading.await;
                bail!("durable cancellation polling deadline elapsed");
            }
        };
        match result {
            Err(error)
                if error
                    .downcast_ref::<rusqlite::Error>()
                    .and_then(rusqlite::Error::sqlite_error_code)
                    == Some(rusqlite::ErrorCode::DatabaseBusy)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::select! { biased;
                    _ = stopped.cancelled() => return Ok(false),
                    _ = tokio::time::sleep(Duration::from_millis(5)) => {}
                }
            }
            result => return result,
        }
    }
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
    cleanup: Arc<CleanupSlot>,
) -> Result<ManagedExecution> {
    execute_admitted_with_controls(
        owner, run, config, workspace, sink, cancel, interrupt, authority, approver, cleanup, None,
        None,
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
    cleanup: Arc<CleanupSlot>,
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
        Err(failure) => {
            let reason = format!("Runtime startup failed during {}.", failure.stage);
            let actual = run.fail_before_execution_reason(&reason).await?;
            let retained = match &controls {
                Some(controls) => controls.shutdown_retained(owner).await.is_ok(),
                None => true,
            };
            let observed = failure.cleanup_observed && retained;
            if observed {
                host_reservation.release_observed()?;
                run.confirm_local_cleanup_observed().await?;
            }
            return Ok(ManagedExecution {
                actual,
                cleanup_observed: observed,
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
    let monitor_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let checkpoint = run.checkpoint();
        let monitor_failed = monitor_failed.clone();
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
                match poll_local_cancellation(
                    || checkpoint.local_cancel_requested(),
                    &done,
                    &cancel,
                    &monitor_failed,
                )
                .await
                {
                    Ok(true) => {
                        cancel.cancel();
                        return Ok(());
                    }
                    Ok(false) => {}
                    _ => {
                        cancel.cancel();
                        monitor_failed.store(true, std::sync::atomic::Ordering::Release);
                        bail!("durable cancellation polling failed");
                    }
                }
            }
        })
    };
    let result = await_execution(
        async {
            if let Some((name, arguments)) = operator {
                run.execute_operator(
                    &resources.agent,
                    cancel.clone(),
                    &name,
                    arguments,
                    redactor(config),
                )
                .await?;
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
    cleanup
        .install(PendingCleanup::new(
            run.checkpoint(),
            resources,
            host_reservation,
            watcher,
            monitor_failed,
            controls.clone(),
            owner.clone(),
        ))
        .await?;
    // The borrowed RunOwner and cleanup checkpoint are the two expected token
    // owners here. After return only the retained checkpoint remains.
    let budget = if controls.is_some() {
        Duration::from_secs(2)
    } else {
        Duration::from_secs(60)
    };
    let observed = cleanup.wait(budget, 2).await;
    let actual = run.record().await?;
    let _ = result; // Execution failure never substitutes for cleanup observation.
    Ok(ManagedExecution {
        actual,
        cleanup_observed: observed,
        construction_failed: false,
    })
}
