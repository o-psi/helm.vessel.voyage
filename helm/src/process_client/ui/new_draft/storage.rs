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
    Ok(serde_json::to_string(&(
        &client.directory,
        &client.ssh,
        &client.access_file,
    ))?)
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

pub(super) fn create(saved: Saved, route: usize) -> Result<Draft> {
    let lock = lock(&root()?, saved.id)?.context("draft is open in another Helm")?;
    if !saved.text.is_empty() || !saved.images.is_empty() {
        save(&saved)?;
    }
    Ok(Draft {
        saved,
        route,
        composer: Default::default(),
        busy: false,
        _lock: lock,
    })
}

pub(super) fn recover(clients: &[Client]) -> Result<BTreeMap<Uuid, Draft>> {
    let root = root()?;
    let routes = clients.iter().map(route).collect::<Result<Vec<_>>>()?;
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
        let metadata = std::fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() <= (2 * voyage_protocol::vessel::MAX_VESSEL_BODY) as u64,
            "invalid new-voyage draft file"
        );
        let saved: Saved = serde_json::from_slice(&std::fs::read(path)?)?;
        ensure!(saved.id == id, "draft identity mismatch");
        if saved.finished {
            continue;
        }
        super::super::attachments::validate_set(&saved.images)?;
        ensure!(
            drafts.len() < 4096,
            "too many local drafts to recover; archive draft files before continuing"
        );
        let Some(route) = routes.iter().position(|r| r == &saved.route) else {
            continue;
        };
        let mut composer = composer::Composer::default();
        composer.text = saved.text.clone();
        composer.cursor = composer.text.len();
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
