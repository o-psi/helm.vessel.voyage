use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    #[default]
    Openai,
    Anthropic,
    #[serde(rename = "codex-subscription")]
    CodexSubscription,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub provider: ProviderKind,
    pub model: String,
    pub api_key_env: String,
    pub base_url: Option<String>,
    pub system_prompt: String,
    pub max_turns: usize,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub provider_retry_attempts: usize,
    pub provider_retry_initial_ms: u64,
    pub provider_retry_max_ms: u64,
    pub command_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub terminal_max_count: usize,
    pub terminal_max_unread_bytes: usize,
    pub subagent_max_concurrency: usize,
    pub subagent_max_agents: usize,
    pub subagent_event_history: usize,
    pub approval: ApprovalMode,
    pub unattended_approval: UnattendedApprovalMode,
    pub workspace: Option<PathBuf>,
    pub allow_read: Vec<PathBuf>,
    pub allow_write: Vec<PathBuf>,
    pub deny_commands: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub inherit_env: Vec<String>,
    pub redact_values: Vec<String>,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    pub codex_command: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    Always,
    #[default]
    OnRisk,
    Never,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum UnattendedApprovalMode {
    #[default]
    Deny,
    Allow,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: ProviderKind::Openai,
            model: "gpt-5".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            base_url: None,
            system_prompt: include_str!("../prompts/system.md").trim().into(),
            max_turns: 64,
            max_tokens: 8192,
            temperature: None,
            provider_retry_attempts: 4,
            provider_retry_initial_ms: 500,
            provider_retry_max_ms: 8000,
            command_timeout_secs: 120,
            max_output_bytes: 128 * 1024,
            terminal_max_count: 16,
            terminal_max_unread_bytes: 8 * 1024 * 1024,
            subagent_max_concurrency: 4,
            subagent_max_agents: 64,
            subagent_event_history: 2048,
            approval: ApprovalMode::OnRisk,
            unattended_approval: UnattendedApprovalMode::Deny,
            workspace: None,
            allow_read: Vec::new(),
            allow_write: Vec::new(),
            deny_commands: vec!["shutdown".into(), "reboot".into(), "mkfs".into()],
            env: BTreeMap::new(),
            inherit_env: vec!["PATH".into(), "LANG".into(), "LC_ALL".into(), "TERM".into()],
            redact_values: Vec::new(),
            mcp_servers: BTreeMap::new(),
            codex_command: "codex".into(),
        }
    }
}

impl Config {
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        let path = explicit.map(PathBuf::from).or_else(default_config_path);
        let mut config = if let Some(path) = path.filter(|p| p.exists()) {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read config {}", path.display()))?;
            toml::from_str(&text).with_context(|| format!("invalid config {}", path.display()))?
        } else {
            Self::default()
        };
        config.apply_provider_defaults();
        config.validate()?;
        Ok(config)
    }

    pub fn api_key(&self) -> Result<String> {
        env::var(&self.api_key_env).with_context(|| format!("{} is not set", self.api_key_env))
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.command_timeout_secs)
    }

    pub fn resolve_workspace(&self, cli: Option<PathBuf>) -> Result<PathBuf> {
        let raw = cli
            .or_else(|| self.workspace.clone())
            .unwrap_or(env::current_dir()?);
        raw.canonicalize()
            .with_context(|| format!("workspace does not exist: {}", raw.display()))
    }

    fn apply_provider_defaults(&mut self) {
        if self.provider == ProviderKind::Anthropic {
            if self.api_key_env == "OPENAI_API_KEY" {
                self.api_key_env = "ANTHROPIC_API_KEY".into();
            }
            if self.model == "gpt-5" {
                self.model = "claude-sonnet-4-0".into();
            }
        }
    }

    fn validate(&self) -> Result<()> {
        if self.model.trim().is_empty() {
            bail!("model cannot be empty");
        }
        if self.max_turns == 0 {
            bail!("max_turns must be greater than zero");
        }
        if self.max_output_bytes < 1024 {
            bail!("max_output_bytes must be at least 1024");
        }
        if self.terminal_max_count == 0 || self.terminal_max_unread_bytes < 1024 {
            bail!("terminal limits require a positive count and at least 1024 unread bytes");
        }
        if self.subagent_max_concurrency == 0
            || self.subagent_max_agents == 0
            || self.subagent_event_history == 0
        {
            bail!("subagent limits must be greater than zero");
        }
        if self.provider_retry_attempts == 0 {
            bail!("provider_retry_attempts must be greater than zero");
        }
        if self.provider_retry_initial_ms == 0
            || self.provider_retry_max_ms < self.provider_retry_initial_ms
        {
            bail!("provider retry delays must be positive and max must be >= initial");
        }
        if self.provider == ProviderKind::CodexSubscription && self.codex_command.trim().is_empty()
        {
            bail!("codex_command cannot be empty for the codex-subscription provider");
        }
        for name in &self.inherit_env {
            let upper = name.to_ascii_uppercase();
            if ["KEY", "TOKEN", "SECRET", "PASSWORD", "CREDENTIAL"]
                .iter()
                .any(|marker| upper.contains(marker))
            {
                bail!("refusing to inherit secret-like environment variable `{name}`");
            }
        }
        Ok(())
    }
}

pub fn default_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("helm/config.toml"))
}

pub fn default_data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("helm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_secret_environment_inheritance() {
        let mut config = Config::default();
        config.inherit_env.push("DEPLOY_TOKEN".into());
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("DEPLOY_TOKEN")
        );
    }

    #[test]
    fn parses_codex_subscription_without_api_credentials() {
        let config: Config =
            toml::from_str("provider = \"codex-subscription\"\nmodel = \"test\"").unwrap();
        assert_eq!(config.provider, ProviderKind::CodexSubscription);
        assert_eq!(config.codex_command, "codex");
        config.validate().unwrap();
    }
}
