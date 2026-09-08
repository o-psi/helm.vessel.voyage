use super::*;
use anyhow::ensure;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
};

fn root() -> Result<PathBuf> {
    let root = super::super::super::cli::default_directory().with_file_name("helm-new-drafts");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::create_dir_all(root.parent().context("draft parent")?)?;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    }
    super::super::super::local::check_private_directory(&root)?;
    Ok(root)
}

pub(super) fn route(client: &Client) -> Result<String> {
    Ok(format!("connection:{}", client.id()))
}

// Exact explicit legacy route only; never infer authority from an endpoint.
// Old SSH's non-null reserved slot remains unsupported and preserved.
fn legacy_route(client: &Client) -> Result<Option<String>> {
    if client
        .managed()
        .is_some_and(|connection| connection.legacy_route.is_none())
    {
        return Ok(None);
    }
    let legacy = client.legacy_route();
    Ok(Some(serde_json::to_string(&(
        &legacy.directory,
        Option::<&str>::None,
        &legacy.access_file,
    ))?))
}

fn lock(root: &Path, id: Uuid) -> Result<Option<File>> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(root.join(format!("{id}.lock")))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn save(saved: &Saved) -> Result<()> {
    saved.validate_identity()?;
    let root = root()?;
    let mut file = tempfile::NamedTempFile::new_in(&root)?;
    let bytes = serde_json::to_vec(saved)?;
    ensure!(
        bytes.len() <= 2 * voyage_protocol::vessel::MAX_VESSEL_BODY,
        "new-voyage draft exceeds private storage limit"
    );
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(root.join(format!("{}.json", saved.id)))?;
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    Ok(())
}

pub(super) fn create(saved: Saved, route: Route) -> Result<Draft> {
    let lock = lock(&root()?, saved.id)?.context("draft is open in another Helm")?;
    // Even an empty workspace choice is a durable Helm draft, not a voyage.
    save(&saved)?;
    Ok(Draft {
        saved,
        route,
        composer: Default::default(),
        busy: false,
        _lock: lock,
    })
}

pub(super) fn recover<'a>(
    clients: impl Iterator<Item = &'a Client>,
) -> Result<BTreeMap<Uuid, Draft>> {
    let root = root()?;
    let routes = clients
        .map(|client| {
            Ok((
                Route {
                    id: client.id(),
                    generation: client.generation(),
                },
                route(client)?,
                legacy_route(client)?,
                client
                    .managed()
                    .and_then(|c| c.legacy_route.as_ref())
                    .map(|legacy| format!("connection:{}", legacy.id())),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut drafts = BTreeMap::new();
    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<Uuid>().ok())
        else {
            continue;
        };
        let Some(lock) = lock(&root, id)? else {
            continue;
        };
        let bytes = super::super::drafts::read_private(
            &path,
            2 * voyage_protocol::vessel::MAX_VESSEL_BODY,
        )?;
        let mut saved: Saved = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow::anyhow!("Invalid saved new-voyage draft; original file preserved")
        })?;
        ensure!(saved.id == id, "draft identity mismatch");
        saved.validate_identity()?;
        if saved.finished {
            continue;
        }
        super::super::attachments::validate_set(&saved.images)?;
        ensure!(
            drafts.len() < 4096,
            "too many local drafts to recover; archive draft files before continuing"
        );
        let matching: Vec<_> = routes
            .iter()
            .filter(|(_, stable, legacy, startup)| {
                &saved.route == stable
                    || legacy.as_ref() == Some(&saved.route)
                    || startup.as_ref() == Some(&saved.route)
            })
            .collect();
        // Ambiguous aliases must never select whichever route is listed first.
        let [matched] = matching.as_slice() else {
            continue;
        };
        let (route, stable, _, _) = *matched;
        let route = *route;
        if saved.route != *stable {
            // Migration changes ONLY the route key under the original lock.
            // Original command IDs, envelopes, expiry and receipts are untouched.
            // Retain byte-exact pre-migration input alongside the stable record.
            let backup = root.join(format!("{id}.legacy"));
            if !backup.try_exists()? {
                let mut original = tempfile::NamedTempFile::new_in(&root)?;
                original.write_all(&bytes)?;
                original.as_file().sync_all()?;
                original.persist_noclobber(backup)?;
            }
            saved.route = stable.clone();
            save(&saved)?;
        }
        let composer = super::super::attachments::restore_draft(
            saved.text.clone(),
            saved.markers.clone(),
            &saved.images,
        )?;
        saved.text = composer.text.clone();
        saved.markers = (!saved.images.is_empty()).then(|| composer.markers.clone());
        drafts.insert(
            id,
            Draft {
                saved,
                route,
                composer,
                busy: false,
                _lock: lock,
            },
        );
    }
    Ok(drafts)
}

/// Read a transition saved before an asynchronous completion was discarded.
pub(super) fn reload(id: Uuid) -> Result<Option<Saved>> {
    let path = root()?.join(format!("{id}.json"));
    if !path.try_exists()? {
        return Ok(None);
    }
    let bytes =
        super::super::drafts::read_private(&path, 2 * voyage_protocol::vessel::MAX_VESSEL_BODY)?;
    let saved: Saved = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("Invalid saved new-voyage draft; original file preserved"))?;
    ensure!(saved.id == id, "Saved draft identity mismatch");
    saved.validate_identity()?;
    super::super::attachments::validate_set(&saved.images)?;
    Ok(Some(saved))
}
