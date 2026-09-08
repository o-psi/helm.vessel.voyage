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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<super::attachments::Image>,
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
        // Retain the reserved null slot so existing local/HTTPS view hashes stay stable.
        Option::<&str>::None,
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
    let bytes = read_private(&path, MAX_DRAFT_BYTES)?;
    let draft: Draft = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid saved interface draft; original file preserved"))?;
    super::attachments::validate_set(&draft.images)?;
    view.images = draft.images;
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
        images: view.images.clone(),
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

/// Pin and bound the opened private file, not a prior pathname observation.
#[cfg(unix)]
pub(super) fn read_private(path: &std::path::Path, limit: usize) -> Result<Vec<u8>> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| anyhow::anyhow!("Cannot open private image draft"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.len() <= limit as u64,
        "Invalid private image draft file"
    );
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "Private image draft exceeds limit");
    Ok(bytes)
}
#[cfg(not(unix))]
pub(super) fn read_private(_path: &std::path::Path, _limit: usize) -> Result<Vec<u8>> {
    anyhow::bail!("Private image draft storage unsupported on this platform")
}

#[cfg(test)]
mod attachment_tests {
    use super::*;
    #[test]
    fn attachment_old_view_draft_roundtrip_omits_empty_images() {
        let old = r#"{"text":" keep\n","pending":null,"acknowledged_completion":null}"#;
        let draft: Draft = serde_json::from_str(old).unwrap();
        assert!(draft.images.is_empty());
        assert_eq!(draft.text, " keep\n");
        assert_eq!(serde_json::to_string(&draft).unwrap(), old);
    }
}
