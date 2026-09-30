//! Private executing-host configuration handoffs preserve policy selection evidence.
use super::*;
use std::{io::Read, path::Path};
pub(crate) fn load(path: Option<&Path>, workspace: &Path) -> Result<Config> {
    load_with_digest(path, workspace, None)
}

pub(crate) fn load_with_digest(
    path: Option<&Path>,
    workspace: &Path,
    expected_digest: Option<&str>,
) -> Result<Config> {
    let Some(path) = path else {
        ensure!(
            expected_digest.is_none(),
            "pinned configuration path missing"
        );
        return Config::load(None);
    };
    crate::session::reject_symlinks(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= 1024 * 1024,
        "configuration file exceeds limit or is not regular"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0
                && metadata.nlink() == 1,
            "launch configuration must be private and owned"
        );
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 1024 * 1024,
        "configuration grew beyond limit"
    );
    if let Some(expected) = expected_digest {
        ensure!(
            expected.len() == 64 && crate::identity_helper::config_digest(&bytes) == expected,
            "reviewed launch configuration changed"
        );
    }
    let mut config = if bytes.iter().copied().find(|b| !b.is_ascii_whitespace()) == Some(b'{') {
        serde_json::from_slice::<crate::launch_config::LaunchConfig>(&bytes)?.resolve(workspace)?
    } else {
        let text = std::str::from_utf8(&bytes)?;
        Config::parse_loaded(text)?
    };
    config.protect_extension_file(path, &metadata)?;
    config.extension_private_files_complete = true;
    Ok(config)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn protected_digest_pins_exact_bytes_before_configuration_resolution() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("config.toml");
        let bytes = toml::to_string(&Config::default()).unwrap().into_bytes();
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let digest = crate::identity_helper::config_digest(&bytes);
        assert!(load_with_digest(Some(&path), fixture.path(), Some(&digest)).is_ok());
        let mut changed = bytes.clone();
        changed.extend_from_slice(b"\n# independently changed after review\n");
        std::fs::write(&path, changed).unwrap();
        assert!(
            load_with_digest(Some(&path), fixture.path(), Some(&digest))
                .unwrap_err()
                .to_string()
                .contains("reviewed launch configuration changed")
        );
        // User installation remains an independent unpinned configuration path.
        assert!(load(Some(&path), fixture.path()).is_ok());
        assert!(load_with_digest(None, fixture.path(), Some(&digest)).is_err());
        assert!(load_with_digest(Some(&path), fixture.path(), Some("invalid")).is_err());
    }

    #[test]
    fn pinned_configuration_still_refuses_symlinks_hardlinks_and_shared_permissions() {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("config.toml");
        let bytes = toml::to_string(&Config::default()).unwrap().into_bytes();
        let digest = crate::identity_helper::config_digest(&bytes);
        std::fs::write(&path, &bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = fixture.path().join("link.toml");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(load_with_digest(Some(&link), fixture.path(), Some(&digest)).is_err());
        let hardlink = fixture.path().join("hardlink.toml");
        std::fs::hard_link(&path, &hardlink).unwrap();
        assert!(load_with_digest(Some(&path), fixture.path(), Some(&digest)).is_err());
        std::fs::remove_file(hardlink).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        assert!(load_with_digest(Some(&path), fixture.path(), Some(&digest)).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(load_with_digest(Some(&path), fixture.path(), Some(&digest)).is_ok());
    }
}
