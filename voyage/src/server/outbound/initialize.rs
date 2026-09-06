use super::*;
pub(crate) fn initialize(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
    workspace: &std::path::Path,
    initialization: &RuntimeInitialization,
) -> Result<()> {
    let RuntimeInitialization::Outbound {
        enrollment_directory,
        origin,
        allow_insecure_loopback,
        source_directory,
        expected_revision,
    } = initialization
    else {
        bail!("not outbound")
    };
    let actor = LocalActorStore::open(&directory.join("identity"))?.identity()?;
    let client =
        EnrollmentClient::open_existing(enrollment_directory, origin, *allow_insecure_loopback)?;
    let inspection = client.inspection();
    let binding = RemoteBinding {
        origin: client.origin().into(),
        machine_id: inspection.machine_id,
        owner_id: inspection.owner_id.context("enrollment inactive")?,
        epoch: inspection.epoch,
        local_installation_id: actor.installation_id,
        local_principal_id: actor.principal_id,
    };
    let mut journal = Journal::open(directory.join("journal"))?;
    if let Some(source) = source_directory {
        journal.import_managed(
            &source.join("journal"),
            registration.session_id,
            registration.session_id,
            expected_revision.context("source revision missing")?,
            workspace,
            true,
        )?;
    }
    match journal.remote_session(&binding)? {
        Some(id) => ensure!(
            id == registration.session_id,
            "outbound canonical session identity mismatch"
        ),
        None => {
            let config =
                super::bootstrap::load_config(registration.config_path.as_deref(), workspace)?;
            let mut session = Session::new(workspace.to_path_buf(), config.model);
            session.id = registration.session_id;
            journal.create_remote_session(&session, &binding)?;
        }
    }
    Ok(())
}
