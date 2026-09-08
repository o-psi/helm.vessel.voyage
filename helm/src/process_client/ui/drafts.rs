//! Interface drafts and uncertain command identities never enter canonical history.
use super::state::{Pending, View};
use crate::process_client::transport::Client;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf};

// Include the immutable public command envelope and escaped draft text.
const MAX_DRAFT_BYTES: usize = 2 * voyage_protocol::vessel::MAX_VESSEL_BODY;

#[derive(Serialize, Deserialize)]
struct Draft {
    text: String,
    pending: Option<Pending>,
    #[serde(default)]
    acknowledged_completion: Option<uuid::Uuid>,
}

fn path(client: &Client, view: &View) -> Result<PathBuf> {
    let root = super::super::cli::default_directory().with_file_name("helm-views");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::create_dir_all(root.parent().expect("view parent"))?;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    super::super::local::check_private_directory(&root)?;
    let identity = serde_json::to_vec(&(
        client.ssh.as_deref(),
        &client.access_file,
        &client.directory,
        view.process.session_id,
    ))?;
    Ok(root.join(format!("{:x}.json", Sha256::digest(identity))))
}

pub fn load(client: &Client, view: &mut View) -> Result<()> {
    let path = path(client, view)?;
    if !path.try_exists()? {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= MAX_DRAFT_BYTES as u64,
        "invalid saved interface draft"
    );
    let draft: Draft = serde_json::from_slice(&std::fs::read(path)?)?;
    view.draft.text = draft.text;
    view.draft.cursor = view.draft.text.len();
    view.pending = draft.pending;
    view.acknowledged_completion = draft.acknowledged_completion;
    Ok(())
}

pub fn save(client: &Client, view: &View) -> Result<()> {
    let path = path(client, view)?;
    let draft = Draft {
        text: view.draft.text.clone(),
        pending: view.pending.clone(),
        acknowledged_completion: view.acknowledged_completion,
    };
    let bytes = serde_json::to_vec(&draft)?;
    ensure!(
        bytes.len() <= MAX_DRAFT_BYTES,
        "saved interface draft exceeds recovery limit"
    );
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().expect("view parent"))?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path)?;
    #[cfg(unix)]
    std::fs::File::open(path.parent().expect("view parent"))?.sync_all()?;
    Ok(())
}
