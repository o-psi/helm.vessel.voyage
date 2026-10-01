//! Authored startup failure metadata; never persist parser or storage diagnostics.
use super::*;

pub(super) fn checked<T>(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
    stage: &'static str,
    result: Result<T>,
) -> Result<T> {
    if let Err(error) = &result {
        let category = category(error);
        // A failure marker is diagnostic only. It is never cleanup, admission,
        // restart permission, a command receipt or authorization to replay work.
        let _ = recovery::persist(
            &directory.join(format!("startup-{}.json", registration.incarnation)),
            &serde_json::json!({
                "session_id": registration.session_id,
                "incarnation": registration.incarnation,
                "stage": stage,
                "category": category,
            }),
        );
    }
    result
}
fn category(error: &anyhow::Error) -> &'static str {
    if let Some(error) = error.downcast_ref::<rusqlite::Error>() {
        return match error.sqlite_error_code() {
            Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => {
                "storage_busy"
            }
            _ => "storage_refused",
        };
    }
    if error.downcast_ref::<std::fs::TryLockError>().is_some() {
        return "ownership_refused";
    }
    if let Some(error) = error.downcast_ref::<std::io::Error>() {
        return match error.kind() {
            std::io::ErrorKind::PermissionDenied => "storage_permission",
            std::io::ErrorKind::NotFound => "required_resource_missing",
            _ => "io_refused",
        };
    }
    "startup_refused"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_are_authored_and_never_include_private_error_payloads() {
        assert_eq!(
            category(&anyhow::anyhow!("synthetic secret credential/path")),
            "startup_refused"
        );
        let error = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("synthetic private query".into()),
        );
        assert_eq!(category(&error.into()), "storage_busy");
        let error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "private path");
        assert_eq!(category(&error.into()), "storage_permission");
    }
}
