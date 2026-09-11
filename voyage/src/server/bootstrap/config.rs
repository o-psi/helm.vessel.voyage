//! Private executing-host configuration handoffs preserve policy selection evidence.
use super::*;
use std::{io::Read, path::Path};
pub(crate) fn load(path: Option<&Path>, workspace: &Path) -> Result<Config> {
    let Some(path) = path else {
        return Config::load(None);
    };
    crate::session::reject_symlinks(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
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
