//! Deny-only configuration-source provenance, never execution authority.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivateFile {
    pub path: PathBuf,
    pub device: Option<u64>,
    pub inode: Option<u64>,
}
impl PrivateFile {
    pub(crate) fn capture(path: &Path, metadata: &std::fs::Metadata) -> Result<Self> {
        let path = std::path::absolute(path)?;
        ensure!(
            metadata.is_file(),
            "configuration provenance requires a regular file"
        );
        #[cfg(unix)]
        let (device, inode) = {
            use std::os::unix::fs::MetadataExt;
            (Some(metadata.dev()), Some(metadata.ino()))
        };
        #[cfg(not(unix))]
        let (device, inode) = (None, None);
        Ok(Self {
            path,
            device,
            inode,
        })
    }
    pub(crate) fn matches(&self, path: &Path, metadata: &cap_std::fs::Metadata) -> bool {
        if path == self.path {
            return true;
        }
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;
            self.device == Some(metadata.dev()) && self.inode == Some(metadata.ino())
        }
        #[cfg(not(unix))]
        {
            let _ = metadata;
            false
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn explicit_source_is_retained_privately_and_matches_renamed_identity() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("explicit-private-config.toml");
        std::fs::write(&source, "model = \"fixture\"\napi_key_required = false\n")?;
        let config = crate::Config::load(Some(&source))?;
        assert!(config.extension_private_files_complete);
        assert_eq!(config.extension_private_files.len(), 1);
        let restored: crate::Config = serde_json::from_slice(&serde_json::to_vec(&config)?)?;
        assert_eq!(
            restored.extension_private_files,
            config.extension_private_files
        );
        assert!(
            !config
                .diagnostic_toml()?
                .contains("explicit-private-config")
        );
        let moved = temp.path().join("renamed.toml");
        std::fs::rename(&source, &moved)?;
        let dir = cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority())?;
        let file = dir.open("renamed.toml")?;
        assert!(restored.extension_private_files[0].matches(&moved, &file.metadata()?));
        Ok(())
    }
}
