//! Pre-dispatch failures release only resources positively observed idle.
use super::*;
pub(super) const RECORD_READ: &str = "Runtime startup failed during preparation record read.";
pub(super) const PARTICIPANT_CONFIG: &str =
    "Runtime startup failed during participant configuration.";
pub(super) fn terminal_registration_failure(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<rusqlite::Error>() {
        Some(rusqlite::Error::SqliteFailure(code, _))
            if matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) =>
        {
            "Runtime startup failed during terminal registration: database busy."
        }
        _ => "Runtime startup failed during terminal registration.",
    }
}

pub(super) async fn reject_constructed(
    owner: &ManagedSessionOwner,
    run: &mut crate::attachment::runtime::RunOwner,
    resources: &crate::build::ManagedAgent,
    controls: &crate::server::controls::LiveControls,
    reservation: &crate::host_resources::Reservation,
    reason: &str,
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
    let actual = run.fail_before_execution_reason(reason).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_registration_labels_are_safe_and_specific() {
        for code in [rusqlite::ffi::SQLITE_BUSY, rusqlite::ffi::SQLITE_LOCKED] {
            let error = anyhow::Error::new(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                Some("private database path".into()),
            ))
            .context("private wrapper");
            assert_eq!(
                terminal_registration_failure(&error),
                "Runtime startup failed during terminal registration: database busy."
            );
        }
        assert_eq!(
            terminal_registration_failure(&anyhow::anyhow!("private error")),
            "Runtime startup failed during terminal registration."
        );
    }
}
