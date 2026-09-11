//! Only public selection/intent metadata is durable. Private enrollment responses never enter here.
use super::*;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Intent {
    pub host: Uuid,
    pub workspace: PathBuf,
    pub command: VesselCommand,
    pub cancel: Option<Uuid>,
}
#[derive(Default, Serialize, Deserialize)]
pub(super) struct Preferences {
    pub choices: Vec<(Uuid, AccountBinding)>,
    pub enrollment: Option<Intent>,
}
fn path(host: Uuid, workspace: &std::path::Path) -> Result<PathBuf> {
    let root =
        crate::process_client::cli::default_directory().with_file_name("helm-account-choices");
    #[cfg(test)]
    let root = crate::process_client::ui::account_test_support::root("helm-account-choices", root);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::create_dir_all(root.parent().context("account preference parent")?)?;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    crate::process_client::local::check_private_directory(&root)?;
    Ok(root.join(key(host, workspace)?))
}
pub(super) fn load(host: Uuid, workspace: &std::path::Path) -> Result<Preferences> {
    let path = path(host, workspace)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = match options.open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preferences::default()),
        Err(e) => return Err(e.into()),
    };
    private_file(&file)?;
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "Account preferences exceed limit");
    let prefs: Preferences = serde_json::from_slice(&bytes)?;
    ensure!(
        prefs.choices.len() <= 64,
        "Account preference count exceeds limit"
    );
    Ok(prefs)
}
pub(super) fn save(host: Uuid, workspace: &std::path::Path, value: &Preferences) -> Result<()> {
    let path = path(host, workspace)?;
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() < 65536, "Account preference limit reached");
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("preference parent")?)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(&path)?;
    #[cfg(unix)]
    std::fs::File::open(path.parent().context("preference parent")?)?.sync_all()?;
    Ok(())
}

// One visible picker owns a host/workspace preference transaction. A second Helm
// cannot race a new enrollment past the first one's durable intent.
pub(super) fn lock(host: Uuid, workspace: &std::path::Path) -> Result<std::fs::File> {
    let lock = path(host, workspace)?.with_extension("lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(lock)?;
    private_file(&file)?;
    file.try_lock().map_err(|_| anyhow::anyhow!("Another Helm account view owns this host/workspace; close it before changing preferences or enrollment"))?;
    Ok(file)
}

fn private_file(file: &std::fs::File) -> Result<()> {
    let m = file.metadata()?;
    ensure!(
        m.is_file() && m.len() <= 65536,
        "Unsafe account preference file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            m.uid() == unsafe { libc::geteuid() } && m.mode() & 0o077 == 0 && m.nlink() == 1,
            "Account preference ownership/mode/link check failed"
        );
    }
    Ok(())
}

fn key(host: Uuid, workspace: &std::path::Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}.json",
        Sha256::digest(serde_json::to_vec(&(host, workspace))?)
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_key_authenticated_host_and_workspace_not_route_or_label() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        assert_eq!(
            key(a, std::path::Path::new("/work")).unwrap(),
            key(a, std::path::Path::new("/work")).unwrap()
        );
        assert_ne!(
            key(a, std::path::Path::new("/work")).unwrap(),
            key(b, std::path::Path::new("/work")).unwrap()
        );
        assert_ne!(
            key(a, std::path::Path::new("/work")).unwrap(),
            key(a, std::path::Path::new("/other")).unwrap()
        );
    }
}
