//! Pre-dispatch failures release only resources positively observed idle.
use super::*;
pub(super) async fn reject_constructed(
    owner: &ManagedSessionOwner,
    run: &mut crate::attachment::runtime::RunOwner,
    resources: &crate::build::ManagedAgent,
    controls: &crate::server::controls::LiveControls,
    reservation: &crate::host_resources::Reservation,
) -> Result<ManagedExecution> {
    let children = tokio::time::timeout(Duration::from_secs(15), resources.subagents.shutdown())
        .await
        .is_ok();
    let resources = match &resources.resources {
        Some(resources) => resources.shutdown_observed(true).await.is_ok(),
        None => false,
    };
    let terminal = controls.shutdown_retained(owner).await.is_ok();
    let observed = children && resources && terminal;
    let actual = run.fail_before_execution().await?;
    if observed {
        reservation.release_observed()?;
        run.confirm_local_cleanup_observed().await?;
    }
    Ok(ManagedExecution {
        actual,
        cleanup_observed: observed,
        construction_failed: true,
    })
}
