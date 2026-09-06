use super::*;
use std::{io::Read, path::Path};
#[derive(clap::Args)]
pub struct ImportPlanArgs {
    #[arg(long)]
    pub source_directory: PathBuf,
    #[arg(long)]
    pub session: Uuid,
}
pub fn import_plan(args: ImportPlanArgs) -> Result<serde_json::Value> {
    let bytes = read(&args.source_directory.join(format!("{}.json", args.session)))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value["format"] == "helm.session-transfer" {
        ensure!(value["version"] == 1, "unsupported transfer marker version");
        let provenance: crate::attachment::migration::Provenance =
            serde_json::from_value(value["provenance"].clone())?;
        ensure!(
            provenance.session_id == args.session
                && provenance.source
                    == args
                        .source_directory
                        .join(format!("{}.json", args.session))
                        .canonicalize()?,
            "transfer marker source mismatch"
        );
        ensure!(
            provenance.backup
                == provenance
                    .destination
                    .join("imports")
                    .join(format!("{}.json", provenance.transfer_id)),
            "transfer backup path mismatch"
        );
        let original = read(&provenance.backup)?;
        ensure!(
            hex::encode(Sha256::digest(&original)) == provenance.source_sha256,
            "retained original hash mismatch"
        );
        return Ok(
            serde_json::json!({"session_id":args.session,"transferred":true,"transfer_id":provenance.transfer_id,"expected_revision":provenance.source_revision,"source_sha256":provenance.source_sha256,"workspace":provenance.workspace,"source_directory":args.source_directory,"destination_directory":provenance.destination.parent()}),
        );
    }
    let session: Session = serde_json::from_value(value).context("source is not a session")?;
    ensure!(session.id == args.session, "source identity mismatch");
    Ok(
        serde_json::json!({"session_id":session.id,"expected_revision":session.revision,"source_sha256":hex::encode(Sha256::digest(&bytes)),"workspace":session.workspace,"source_directory":args.source_directory}),
    )
}
pub(super) fn original(
    source: &Path,
    session: Uuid,
    destination: &Path,
    transfer: Uuid,
) -> Result<Session> {
    let bytes = read(&source.join(format!("{session}.json")))?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    if value["format"] == "helm.session-transfer" {
        Ok(serde_json::from_slice(&read(
            &destination.join("imports").join(format!("{transfer}.json")),
        )?)?)
    } else {
        Ok(serde_json::from_value(value)?)
    }
}
pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    crate::session::reject_symlinks(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 16 * 1024 * 1024,
        "invalid import source size/type"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() == 1,
            "source must be private and owned"
        );
    }
    let mut bytes = Vec::new();
    file.take(16 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 16 * 1024 * 1024, "source grew beyond limit");
    Ok(bytes)
}
