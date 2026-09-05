//! Per-execution workspace instructions, deliberately excluded from saved history.
use std::{
    fs,
    io::{self, Read},
};

use crate::policy::Policy;
use anyhow::{Context, Result, bail};

pub(crate) const MAX_BYTES: u64 = 64 * 1024;

/// Prefer AGENTS.md; use agents.md only when the preferred entry is absent.
/// Resolve through policy before reading, including symlink targets.
pub(crate) fn load(policy: &Policy) -> Result<Option<String>> {
    for name in ["AGENTS.md", "agents.md"] {
        let path = policy.workspace().join(name);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("cannot inspect {}", path.display()));
            }
            Ok(_) => {}
        }
        let resolved = policy
            .resolve_read(&path)
            .with_context(|| format!("cannot load {}", path.display()))?;
        if !fs::metadata(&resolved)?.is_file() {
            bail!(
                "workspace instructions must be a regular file: {}",
                path.display()
            );
        }
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            // Avoid blocking on a raced FIFO or following a replaced final symlink.
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        let file = options
            .open(&resolved)
            .with_context(|| format!("cannot open {}", path.display()))?;
        if !file.metadata()?.is_file() {
            bail!(
                "workspace instructions must be a regular file: {}",
                path.display()
            );
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("cannot read {}", path.display()))?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!(
                "workspace instructions exceed {MAX_BYTES} bytes: {}",
                path.display()
            );
        }
        let text = String::from_utf8(bytes)
            .with_context(|| format!("workspace instructions are not UTF-8: {}", path.display()))?;
        return Ok(Some(text));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn missing_precedence_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(&Config::default(), dir.path().into()).unwrap();
        assert_eq!(load(&policy).unwrap(), None);
        fs::write(dir.path().join("agents.md"), "lower").unwrap();
        assert_eq!(load(&policy).unwrap().as_deref(), Some("lower"));
        fs::write(dir.path().join("AGENTS.md"), "upper").unwrap();
        assert_eq!(load(&policy).unwrap().as_deref(), Some("upper"));
        fs::write(dir.path().join("AGENTS.md"), [255]).unwrap();
        assert!(load(&policy).is_err());
    }

    #[test]
    fn rejects_directories_and_oversize_files() {
        let dir = tempfile::tempdir().unwrap();
        let policy = Policy::new(&Config::default(), dir.path().into()).unwrap();
        let path = dir.path().join("AGENTS.md");
        fs::create_dir(&path).unwrap();
        assert!(load(&policy).is_err());
        fs::remove_dir(&path).unwrap();
        fs::write(path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        assert!(load(&policy).is_err());
    }

    #[test]
    fn utf8_empty_boundary_and_workspace_scope() {
        let parent = tempfile::tempdir().unwrap();
        fs::write(parent.path().join("AGENTS.md"), "ancestor-not-loaded").unwrap();
        let root = parent.path().join("workspace");
        fs::create_dir(&root).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/AGENTS.md"), "nested-not-loaded").unwrap();
        let policy = Policy::new(&Config::default(), root.clone()).unwrap();
        assert_eq!(load(&policy).unwrap(), None);
        let path = root.join("AGENTS.md");
        fs::write(&path, "").unwrap();
        assert_eq!(load(&policy).unwrap().as_deref(), Some(""));
        let content = "雪".repeat(MAX_BYTES as usize / 3) + "x";
        assert_eq!(content.len(), MAX_BYTES as usize);
        fs::write(&path, &content).unwrap();
        assert_eq!(load(&policy).unwrap().as_deref(), Some(content.as_str()));
    }

    #[cfg(unix)]
    #[test]
    fn permitted_symlink_target_and_fifo() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("guidance");
        fs::write(&target, "allowed").unwrap();
        let config = Config {
            allow_read: vec![outside.path().into()],
            ..Config::default()
        };
        let policy = Policy::new(&config, dir.path().into()).unwrap();
        let path = dir.path().join("AGENTS.md");
        symlink(&target, &path).unwrap();
        assert_eq!(load(&policy).unwrap().as_deref(), Some("allowed"));
        fs::remove_file(&path).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let fifo = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(
            load(&policy)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_external_and_dangling_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let policy = Policy::new(&Config::default(), dir.path().into()).unwrap();
        let target = outside.path().join("instructions");
        fs::write(&target, "outside").unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join("AGENTS.md")).unwrap();
        assert!(load(&policy).is_err());
        fs::remove_file(target).unwrap();
        assert!(load(&policy).is_err());
    }
}
