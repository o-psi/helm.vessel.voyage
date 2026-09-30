//! Fixed, bounded text observation by the executing Voyage, without an agent run.
use crate::policy::Policy;
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::OpenOptions,
    io::Read,
    path::{Component, Path},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;

const MAX_BYTES: usize = 64 * 1024;
static READS: OnceLock<Arc<Semaphore>> = OnceLock::new();

pub(super) async fn read(policy: &Policy, relative: &str) -> Result<Value> {
    ensure!(
        relative.len() <= 4096,
        "Invalid workspace-relative file path"
    );
    let slots = READS.get_or_init(|| Arc::new(Semaphore::new(4)));
    let policy = policy.clone();
    let relative = relative.to_owned();
    bounded_read(slots, Duration::from_secs(4), move || {
        observe(&policy, &relative)
    })
    .await
}

async fn bounded_read(
    slots: &Arc<Semaphore>,
    deadline: Duration,
    observer: impl FnOnce() -> Result<Value> + Send + 'static,
) -> Result<Value> {
    let permit = slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("Workspace file observations are busy"))?;
    // A host filesystem call may outlive the response deadline. Its slot stays
    // owned until the actual file/task closes, including timeout/disconnect.
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        observer()
    });
    tokio::time::timeout(deadline, task)
        .await
        .map_err(|_| {
            anyhow::anyhow!("Workspace file observation timed out; its read may still be pending")
        })?
        .map_err(|_| anyhow::anyhow!("Workspace file observation unavailable"))?
}

fn observe(policy: &Policy, relative: &str) -> Result<Value> {
    let path = Path::new(relative);
    let normalized: std::path::PathBuf = path.components().collect();
    ensure!(
        !relative.is_empty()
            && relative.len() <= 4096
            && !relative.contains('\\')
            && !relative.chars().any(char::is_control)
            && normalized.as_os_str() == path.as_os_str()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "Invalid workspace-relative file path"
    );
    let root = policy.resolve_read(policy.workspace())?;
    let mut candidate = root.clone();
    for part in path.components() {
        candidate.push(part);
        ensure!(
            !std::fs::symlink_metadata(&candidate)?
                .file_type()
                .is_symlink(),
            "Linked files are unavailable to workspace preview"
        );
    }
    let resolved = policy.resolve_read(&candidate)?;
    ensure!(resolved.starts_with(&root), "File is outside the workspace");
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(&resolved)?;
    let before = file.metadata()?;
    ensure!(before.is_file(), "Only regular text files can be previewed");
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let opened = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        ensure!(
            opened == resolved && policy.resolve_read(&opened)? == resolved,
            "Workspace file identity changed"
        );
    }
    let mut file = file;
    let mut bytes = Vec::new();
    (&mut file)
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    let current = std::fs::metadata(&resolved)?;
    stable_metadata(&before, &after, &current)?;
    ensure!(
        policy.resolve_read(&candidate)? == resolved,
        "Workspace file path changed"
    );
    policy.check_current()?;
    let truncated = bytes.len() > MAX_BYTES;
    bytes.truncate(MAX_BYTES);
    ensure!(!bytes.contains(&0), "Binary file preview is unavailable");
    if truncated {
        while std::str::from_utf8(&bytes).is_err_and(|error| error.error_len().is_none()) {
            bytes.pop();
        }
    }
    let text =
        std::str::from_utf8(&bytes).map_err(|_| anyhow::anyhow!("File is not UTF-8 text"))?;
    Ok(json!({"path":relative,"text":text,"truncated":truncated,
        "observed_bytes":bytes.len(),"file_bytes":after.len(),
        "preview_sha256":hex::encode(Sha256::digest(&bytes)),
        "observed_at_ms":std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64}))
}

fn stable_metadata(
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
    current: &std::fs::Metadata,
) -> Result<()> {
    ensure!(
        before.len() == after.len()
            && after.len() == current.len()
            && before.modified()? == after.modified()?
            && after.modified()? == current.modified()?,
        "Workspace file changed during observation"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let identity = |m: &std::fs::Metadata| (m.dev(), m.ino(), m.ctime(), m.ctime_nsec());
        ensure!(
            identity(before) == identity(after) && identity(after) == identity(current),
            "Workspace file identity or metadata changed"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[cfg(unix)]
    #[test]
    fn same_size_rewrite_with_restored_mtime_and_inode_substitution_refuse() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        fs::write(&path, "before\n").unwrap();
        let file = OpenOptions::new().read(true).open(&path).unwrap();
        let before = file.metadata().unwrap();
        fs::write(&path, "after!\n").unwrap();
        file.set_times(std::fs::FileTimes::new().set_modified(before.modified().unwrap()))
            .unwrap();
        let after = file.metadata().unwrap();
        assert_eq!(before.len(), after.len());
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        assert!(stable_metadata(&before, &after, &fs::metadata(&path).unwrap()).is_err());
        let stable = file.metadata().unwrap();
        fs::rename(&path, root.path().join("retained")).unwrap();
        fs::write(&path, "after!\n").unwrap();
        assert!(
            stable_metadata(
                &stable,
                &file.metadata().unwrap(),
                &fs::metadata(path).unwrap()
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn timeout_retains_read_capacity_until_actual_worker_cleanup() {
        let slots = Arc::new(Semaphore::new(1));
        let (release, wait) = std::sync::mpsc::channel();
        let result = bounded_read(&slots, Duration::from_millis(20), move || {
            wait.recv()?;
            Ok(json!({"late":"discarded"}))
        })
        .await;
        assert!(result.unwrap_err().to_string().contains("still be pending"));
        assert_eq!(slots.available_permits(), 0);
        let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = effects.clone();
        let refused = bounded_read(&slots, Duration::from_secs(1), move || {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(json!("unexpected"))
        })
        .await;
        assert!(refused.unwrap_err().to_string().contains("busy"));
        assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 0);
        release.send(()).unwrap();
        let reclaimed = tokio::time::timeout(Duration::from_secs(1), slots.acquire())
            .await
            .unwrap()
            .unwrap();
        drop(reclaimed);
        assert_eq!(
            bounded_read(&slots, Duration::from_secs(1), || Ok(json!("fresh")))
                .await
                .unwrap(),
            json!("fresh")
        );
    }

    #[tokio::test]
    async fn reads_current_text_without_a_repository_or_file_mutation() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("docs")).unwrap();
        let path = root.path().join("docs/note.md");
        fs::write(&path, "# Current document\n").unwrap();
        let policy = Policy::new(&crate::Config::default(), root.path().into()).unwrap();
        let value = read(&policy, "docs/note.md").await.unwrap();
        assert_eq!(value["text"], "# Current document\n");
        assert_eq!(value["path"], "docs/note.md");
        assert_eq!(value["truncated"], false);
        assert_eq!(value["observed_bytes"], 19);
        assert_eq!(
            value["preview_sha256"],
            hex::encode(Sha256::digest(b"# Current document\n"))
        );
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            value["text"].as_str().unwrap()
        );
        assert!(!root.path().join(".git").exists());
    }

    #[tokio::test]
    async fn bounds_unicode_preview_and_distinguishes_binary_or_missing_files() {
        let root = tempfile::tempdir().unwrap();
        let policy = Policy::new(&crate::Config::default(), root.path().into()).unwrap();
        fs::write(root.path().join("large.txt"), "€".repeat(MAX_BYTES)).unwrap();
        let value = read(&policy, "large.txt").await.unwrap();
        assert_eq!(value["truncated"], true);
        assert_eq!(value["file_bytes"], MAX_BYTES * 3);
        assert_eq!(value["text"].as_str().unwrap().len(), MAX_BYTES / 3 * 3);
        fs::write(root.path().join("binary"), b"a\0b").unwrap();
        fs::write(root.path().join("invalid"), [255, 254]).unwrap();
        for path in ["binary", "invalid", "absent"] {
            assert!(read(&policy, path).await.is_err());
        }
    }

    #[tokio::test]
    async fn refuses_path_escape_and_non_regular_files() {
        let root = tempfile::tempdir().unwrap();
        let policy = Policy::new(&crate::Config::default(), root.path().into()).unwrap();
        for path in [
            "",
            ".",
            "..",
            "../outside",
            "/etc/passwd",
            "docs/../file",
            "a\\b",
            "a\nb",
            "a//b",
            "a/./b",
            "file/",
        ] {
            assert!(read(&policy, path).await.is_err(), "{path:?}");
        }
        fs::create_dir(root.path().join("folder")).unwrap();
        assert!(read(&policy, "folder").await.is_err());
        #[cfg(unix)]
        {
            fs::write(root.path().join("real"), "private").unwrap();
            std::os::unix::fs::symlink(root.path().join("real"), root.path().join("linked"))
                .unwrap();
            assert!(read(&policy, "linked").await.is_err());
            std::os::unix::fs::symlink(root.path(), root.path().join("linked-dir")).unwrap();
            assert!(read(&policy, "linked-dir/real").await.is_err());
        }
    }
}
