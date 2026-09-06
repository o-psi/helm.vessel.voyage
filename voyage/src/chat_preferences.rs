//! Operator-local chat defaults, separate from session history and runtime authority.
use crate::{
    Config,
    policy_profile::{
        Overrides,
        selection::{Selection, SelectionRequest},
    },
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

const LIMIT: u64 = 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Presentation {
    pub verbose: bool,
    pub log_format: String,
    pub plain: bool,
    pub activity: bool,
    pub tool_details: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    request: SelectionRequest,
    confirmation: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct State {
    pub presentation: Presentation,
    pub profile: Option<Profile>,
    pub explicit: Overrides,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    config: Config,
    state: State,
}

fn path() -> std::path::PathBuf {
    crate::config::default_data_dir().join("chat-preferences.json")
}

pub fn load() -> Result<Option<Config>> {
    load_from(&path()).map_err(|_| {
        anyhow::anyhow!(
            "chat preferences are invalid or unavailable; use --config PATH to replace them"
        )
    })
}

fn load_from(path: &Path) -> Result<Option<Config>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "preferences must be a regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "preferences must be private"
        );
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "preferences too large");
    let mut saved: Saved = serde_json::from_slice(&bytes)?;
    ensure!(saved.version == 1, "unsupported preferences version");
    ensure!(
        matches!(
            saved.state.presentation.log_format.as_str(),
            "" | "text" | "json"
        ),
        "invalid log format"
    );
    saved.config.validate()?;
    saved.config.policy_explicit = saved.state.explicit.clone();
    saved.config.chat_preferences = Some(saved.state);
    Ok(Some(saved.config))
}

/// Rebind the selected source against the actual target workspace and current ceiling.
pub fn restore_profile(config: &mut Config, workspace: &Path) -> Result<()> {
    if config.policy_profile.is_none()
        && let Some(mut profile) = config
            .chat_preferences
            .as_ref()
            .and_then(|state| state.profile.clone())
    {
        let changed = profile.request.explicit != config.policy_explicit;
        profile.request.explicit = config.policy_explicit.clone();
        config.policy_profile = Some(Selection::bind(config, workspace, profile.request, (!changed).then_some(profile.confirmation.as_str()))
            .context("remembered policy changed or belongs to another workspace; explicitly reselect with --config and --policy-profile")?);
    }
    Ok(())
}

pub fn remember(config: &Config, model: &str, presentation: Option<Presentation>) -> Result<()> {
    if config.chat_preferences.is_none() {
        return Ok(());
    }
    save_to(&path(), config, model, presentation)
        .map_err(|_| anyhow::anyhow!("could not save chat preferences"))
}

fn save_to(
    path: &Path,
    config: &Config,
    model: &str,
    presentation: Option<Presentation>,
) -> Result<()> {
    let mut config = config.clone();
    config.model = model.to_owned();
    config.validate()?;
    let mut state = config.chat_preferences.take().unwrap_or_default();
    if let Some(presentation) = presentation {
        state.presentation = presentation;
    }
    state.explicit = config.policy_explicit.clone();
    state.profile = config.policy_profile.as_ref().map(|selection| Profile {
        request: selection.request().clone(),
        confirmation: selection.confirmation().into(),
    });
    let bytes = serde_json::to_vec(&Saved {
        version: 1,
        config,
        state,
    })?;
    ensure!(bytes.len() as u64 <= LIMIT, "preferences too large");
    let directory = path.parent().context("preferences directory missing")?;
    std::fs::create_dir_all(directory)?;
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}
