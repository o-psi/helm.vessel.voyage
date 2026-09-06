use anyhow::Result;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

pub(crate) fn persist(
    config: &crate::Config,
    workspace: &Path,
    directory: &Path,
) -> Result<PathBuf> {
    let root = directory.join("launch");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    super::super::local::check_private_directory(&root)?;
    let envelope = voyage_runtime::launch_config::LaunchConfig::capture(config, workspace)?;
    let mut file = tempfile::Builder::new()
        .prefix("launch-")
        .suffix(".json")
        .tempfile_in(&root)?;
    file.write_all(&serde_json::to_vec(&envelope)?)?;
    file.as_file().sync_all()?;
    let (_, path) = file.keep()?;
    #[cfg(unix)]
    std::fs::File::open(root)?.sync_all()?;
    Ok(path)
}
