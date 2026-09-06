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
