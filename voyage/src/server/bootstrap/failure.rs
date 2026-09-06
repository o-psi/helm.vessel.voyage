//! Failed bootstrap may establish absence of resources, never erase old obligations.
use super::*;
use std::path::Path;
pub(crate) fn record(directory: &Path, registration: &ProcessRegistration) -> Result<()> {
    ensure!(
        !directory.join("runtime.sock").exists(),
        "runtime endpoint still exists"
    );
    let journal = Journal::open(directory.join("journal"))?;
    let _guard = match journal.load_session(registration.session_id) {
        Ok(_) => {
            let guard = journal.acquire_execution(registration.session_id)?;
            ensure!(
                journal.startup_cleanup_clear(&guard)?,
                "prior execution cleanup unresolved"
            );
            Some(guard)
        }
        Err(error)
            if error
                .downcast_ref::<rusqlite::Error>()
                .is_some_and(|error| matches!(error, rusqlite::Error::QueryReturnedNoRows)) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    use std::io::Write;
    let mut candidate = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(
        &mut candidate,
        &serde_json::json!({"session_id":registration.session_id,"incarnation":registration.incarnation,"cleanup_observed":true,"startup_failed":true}),
    )?;
    candidate.flush()?;
    candidate.as_file().sync_all()?;
    candidate.persist(directory.join("stopped.json"))?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}
