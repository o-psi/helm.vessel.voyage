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
    let identity = serde_json::to_vec(&("connection-v1", client.id(), view.process.session_id))?;
    Ok(root.join(format!("{:x}.json", Sha256::digest(identity))))
}

fn migrate_legacy(client: &Client, view: &View, destination: &std::path::Path) -> Result<()> {
    if client
        .managed()
        .is_some_and(|connection| connection.legacy_route.is_none())
    {
        return Ok(());
    }
    let legacy = client.legacy_route();
    let identity = serde_json::to_vec(&(
        Option::<&str>::None,
        &legacy.access_file,
        &legacy.directory,
        view.process.session_id,
    ))?;
    let root = destination.parent().expect("view parent");
    let tuple_source = root.join(format!("{:x}.json", Sha256::digest(identity)));
    // Explicit first import may follow a prior launch that already migrated the
    // path-derived startup UUID. Prefer that newer stable file, never an endpoint.
    let startup_source = client
        .managed()
        .and_then(|c| c.legacy_route.as_ref())
        .map(|legacy| {
            serde_json::to_vec(&("connection-v1", legacy.id(), view.process.session_id))
                .map(|identity| root.join(format!("{:x}.json", Sha256::digest(identity))))
        })
        .transpose()?;
    let source = match startup_source {
        Some(path) if path.try_exists()? => path,
        _ => tuple_source,
    };
    if !source.try_exists()? {
        return Ok(());
    }
    // Claim the exact old identity once. Do not let an imported copy or another
    // credential with a similar endpoint silently inherit uncertain commands.
    let claim_path = source.with_extension("migration");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut claim = options.open(&claim_path)?;
    claim.try_lock().map_err(|_| {
        anyhow::anyhow!("Legacy draft migration is open in another Helm; original retained")
    })?;
    let binding = read_private(&claim_path, 128)?;
    let owner = client.id().to_string();
    if binding.is_empty() {
        claim.write_all(owner.as_bytes())?;
        claim.sync_all()?;
        #[cfg(unix)]
        std::fs::File::open(root)?.sync_all()?;
    } else {
        ensure!(
            binding == owner.as_bytes(),
            "Legacy draft already belongs to another immutable connection; original retained"
        );
    }
    if destination.try_exists()? {
        return Ok(());
    }
    let bytes = read_private(&source, MAX_DRAFT_BYTES)?;
    // Validate before copying, but preserve the exact original envelope bytes.
    let draft: Draft = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid legacy interface draft; original retained"))?;
    super::attachments::validate_set(&draft.images)?;
    let mut temporary = tempfile::NamedTempFile::new_in(root)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(destination)?;
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}

pub fn load(client: &Client, view: &mut View) -> Result<()> {
    let path = path(client, view)?;
    if !path.try_exists()? {
        migrate_legacy(client, view, &path)?;
        if !path.try_exists()? {
            return Ok(());
        }
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
