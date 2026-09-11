//! Bootstrap is serialized before ownership; imports never silently fall back to creation.
use super::*;
use sha2::{Digest, Sha256};
use voyage_protocol::process::RuntimeInitialization;
mod source;
mod validate;
pub use validate::{ValidateStartArgs, validate_start};
mod config;
mod workspace;
pub(super) use config::load as load_config;
pub use source::{ImportPlanArgs, import_plan};
pub(crate) use workspace::NOTICE as WORKSPACE_RECREATED_NOTICE;
pub(super) use workspace::annotate as annotate_workspace;
pub(super) use workspace::prepare as prepare_workspace;
pub(super) use workspace::recreated as workspace_recreated;
pub(super) async fn initialize(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
    workspace: &std::path::Path,
) -> Result<()> {
    let Some(initialization) = &registration.initialize else {
        return Ok(());
    };
    match initialization {
        RuntimeInitialization::ManagedImport {
            transfer_id,
            source_directory,
            expected_revision,
        } => {
            let mut journal = Journal::open(directory.join("journal"))?;
            journal.import_managed(
                &source_directory.join("journal"),
                registration.session_id,
                *transfer_id,
                *expected_revision,
                workspace,
            )?;
        }
        RuntimeInitialization::Participant {
            parent_session_id, ..
        } => {
            ensure!(
                *parent_session_id != registration.session_id && !parent_session_id.is_nil(),
                "participant must have distinct child identity"
            );
        }
        RuntimeInitialization::Transfer { artifact_path, .. } => {
            let bytes = source::read(artifact_path)?;
            let mut journal = Journal::open(directory.join("journal"))?;
            journal.import_transfer(initialization, &bytes, registration.session_id, workspace)?;
        }
        RuntimeInitialization::Import {
            transfer_id,
            source_directory,
            expected_revision,
            source_sha256,
        } => {
            let journal = Journal::open(directory.join("journal"))?;
            if journal.json_import_finalized(registration.session_id, *transfer_id)? {
                journal.load_session(registration.session_id)?;
                return Ok(());
            }
            if let Some(provenance) =
                journal.json_import_provenance(registration.session_id, *transfer_id)?
            {
                let marker: serde_json::Value = serde_json::from_slice(&source::read(
                    &source_directory.join(format!("{}.json", registration.session_id)),
                )?)?;
                if marker["format"] == "helm.session-transfer"
                    && marker["version"] == 1
                    && marker["provenance"] == provenance
                {
                    ensure!(
                        provenance["source_revision"] == *expected_revision
                            && provenance["source_sha256"] == *source_sha256,
                        "import retry provenance mismatch"
                    );
                    journal.load_session(registration.session_id)?;
                    journal.finalize_json_import(registration.session_id, *transfer_id)?;
                    return Ok(());
                }
            }
            drop(journal);
            let original = source::original(
                source_directory,
                registration.session_id,
                &directory.join("journal"),
                *transfer_id,
            )?;
            ensure!(
                original.workspace.canonicalize()? == workspace,
                "import workspace mismatch"
            );
            let parent = source_directory
                .parent()
                .context("source data directory missing")?;
            let completion_root = parent.join("completion");
            crate::attachment::journal::prepare_directory(completion_root.clone())?;
            let key = hex::encode(Sha256::digest(
                original.workspace.as_os_str().as_encoded_bytes(),
            ));
            let coordinator = crate::completion::runtime::Coordinator::open(
                completion_root.join(key),
                workspace,
            )?;
            crate::attachment::migration::transfer(
                crate::session::SessionStore::new(source_directory.clone()),
                coordinator,
                directory.join("journal"),
                crate::attachment::migration::TransferRequest {
                    transfer_id: *transfer_id,
                    session_id: registration.session_id,
                    expected_revision: *expected_revision,
                    source_sha256: source_sha256.clone(),
                },
                CancellationToken::new(),
            )
            .await?;
            Journal::open(directory.join("journal"))?
                .finalize_json_import(registration.session_id, *transfer_id)?;
        }
        RuntimeInitialization::Branch { branch_id, .. } => {
            ensure!(
                *branch_id == registration.session_id,
                "branch initialization identity mismatch"
            );
            let mut journal = Journal::open(directory.join("journal"))?;
            journal.import_process_branch(initialization, workspace)?;
        }
    }
    Ok(())
}

pub(super) use source::read as read_private_artifact;

mod failure;
pub(super) use failure::record as record_failure;

mod participant;
pub(super) use participant::limit as limit_participant;

pub(super) fn prepare_identity(
    directory: &std::path::Path,
    registration: &ProcessRegistration,
) -> Result<()> {
    let source_directory = match &registration.initialize {
        Some(RuntimeInitialization::ManagedImport {
            source_directory, ..
        }) => source_directory,
        _ => return Ok(()),
    };
    let transfer = match &registration.initialize {
        Some(RuntimeInitialization::ManagedImport { transfer_id, .. }) => *transfer_id,
        _ => registration.session_id,
    };
    if Journal::open(directory.join("journal"))?
        .json_import_finalized(registration.session_id, transfer)?
    {
        ensure!(
            directory.join("identity/actor.json").is_file(),
            "finalized import identity missing"
        );
        LocalActorStore::open(&directory.join("identity"))?;
        return Ok(());
    }
    let bytes = source::read(&source_directory.join("actor.json"))?;
    let target = crate::attachment::journal::prepare_directory(directory.join("identity"))?
        .join("actor.json");
    if target.exists() {
        ensure!(
            source::read(&target)? == bytes,
            "managed source actor conflicts with destination"
        );
    } else {
        use std::io::Write;
        let mut candidate =
            tempfile::NamedTempFile::new_in(target.parent().context("identity parent missing")?)?;
        candidate.write_all(&bytes)?;
        candidate.as_file().sync_all()?;
        candidate.persist_noclobber(&target)?;
        std::fs::File::open(target.parent().context("identity parent missing")?)?.sync_all()?;
    }
    Ok(())
}

mod upgrade;
pub use upgrade::{UpgradeArgs, upgrade};
