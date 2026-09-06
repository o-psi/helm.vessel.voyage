//! Interface logging must not corrupt the full-screen display.
use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
pub(crate) fn command_uses_full_screen_tui(
    cli: &Cli,
    stdin_terminal: bool,
    stdout_terminal: bool,
    term: Option<&str>,
) -> bool {
    stdin_terminal
        && stdout_terminal
        && term.is_some_and(|term| !term.is_empty() && term != "dumb")
        && matches!(
            &cli.command,
            None | Some(Command::Chat { plain: false, .. })
        )
}

pub(crate) fn tui_log_file() -> Result<File> {
    let directory = helm::config::default_data_dir().join("logs");
    std::fs::create_dir_all(&directory).with_context(|| {
        format!(
            "failed to create Helm log directory {}",
            directory.display()
        )
    })?;
    let path = directory.join("helm.log");
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .with_context(|| format!("failed to open Helm TUI log {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to secure Helm TUI log {}", path.display()))?;
    }
    Ok(file)
}
