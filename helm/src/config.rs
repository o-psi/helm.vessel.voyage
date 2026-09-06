use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProviderKind {
    #[default]
    #[serde(rename = "openai-responses")]
    OpenaiResponses,
    #[serde(rename = "openai-chat", alias = "openai", alias = "openai-compatible")]
    OpenaiChat,
    #[serde(rename = "chatgpt-oauth")]
    ChatGptOauth,
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "codex-compatibility", alias = "codex-subscription")]
    CodexSubscription,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccess {
    NativePublicApi,
    NativeChatgptOauth,
    ExternalCompatibilityBridge,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub access: ProviderAccess,
    pub credential: &'static str,
    pub billing: &'static str,
    pub compatibility_bridge: bool,
}

impl ProviderKind {
    pub fn profile(&self) -> ProviderProfile {
        match self {
            Self::OpenaiResponses => ProviderProfile {
                id: "openai-responses",
                access: ProviderAccess::NativePublicApi,
                credential: "OPENAI_API_KEY (or api_key_env override)",
                billing: "OpenAI API usage is billed separately from ChatGPT subscriptions",
                compatibility_bridge: false,
            },
            Self::OpenaiChat => ProviderProfile {
                id: "openai-chat",
                access: ProviderAccess::NativePublicApi,
                credential: "API key named by api_key_env",
                billing: "Billing is determined by the configured OpenAI-compatible endpoint",
                compatibility_bridge: false,
            },
            Self::ChatGptOauth => ProviderProfile {
                id: "chatgpt-oauth",
                access: ProviderAccess::NativeChatgptOauth,
                credential: "Helm-managed ChatGPT OAuth tokens; run `helm auth login`",
                billing: "Uses the authenticated ChatGPT subscription and its plan limits",
                compatibility_bridge: false,
            },
            Self::Anthropic => ProviderProfile {
                id: "anthropic",
                access: ProviderAccess::NativePublicApi,
                credential: "ANTHROPIC_API_KEY (or api_key_env override)",
                billing: "Anthropic API usage is billed by Anthropic",
                compatibility_bridge: false,
            },
            Self::CodexSubscription => ProviderProfile {
                id: "codex-compatibility",
                access: ProviderAccess::ExternalCompatibilityBridge,
                credential: "this legacy bridge delegates authentication to `codex login`; Helm does not read an API key",
                billing: "This bridge uses the account and entitlement selected by the external Codex CLI",
                compatibility_bridge: true,
            },
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Explicit operator-private defaults source, freshly resolved at root startup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_defaults: Option<crate::policy_profile::defaults::DefaultsSource>,
    /// Invocation policy overrides; never restored as authority from a saved session.
    #[serde(skip)]
    pub policy_explicit: crate::policy_profile::Overrides,
    /// Explicit invocation authority; never persisted or restored from sessions.
    #[serde(skip)]
    pub policy_profile: Option<crate::policy_profile::selection::Selection>,
    /// Explicit GitHub capability; the named credential never enters child environment.
    pub github_enabled: bool,
    pub provider: ProviderKind,
    pub model: String,
    pub api_key_env: String,
    /// Explicit opt-out only for a configured native compatible endpoint.
    pub api_key_required: bool,
    /// Use the compatible Chat max_tokens field instead of max_completion_tokens.
    pub chat_use_max_tokens: bool,
    pub base_url: Option<String>,
    /// Explicit override for the experimental ChatGPT subscription backend.
    /// Kept separate from `base_url` so provider switching cannot redirect OAuth tokens.
    pub chatgpt_base_url: Option<String>,
    pub system_prompt: String,
    pub max_tokens: u32,
    pub context_window: usize,
    pub temperature: Option<f32>,
    pub provider_retry_attempts: usize,
    pub provider_retry_initial_ms: u64,
    pub provider_retry_max_ms: u64,
    pub command_timeout_secs: u64,
    pub max_output_bytes: usize,
    pub terminal_max_count: usize,
    pub terminal_max_unread_bytes: usize,
    pub subagent_max_concurrency: usize,
    pub subagent_event_history: usize,
    /// User-facing authority level. When omitted, the legacy `approval` setting
    /// is translated for backwards compatibility.
    pub access: Option<AccessMode>,
    /// Deprecated compatibility setting. Prefer `access`.
    #[serde(skip_serializing_if = "approval_is_default")]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigValueKind {
    Bool,
    Provider,
    Model,
    EnvironmentName,
    Url,
    Text,
    PositiveInteger,
    NonNegativeInteger,
    Temperature,
    Access,
    Approval,
    UnattendedApproval,
    Path,
    PathList,
    StringList,
    StringMap,
    McpServers,
    Executable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigOverrideSpec {
    pub key: &'static str,
    pub description: &'static str,
    pub kind: ConfigValueKind,
}

pub const CONFIG_OVERRIDE_SPECS: &[ConfigOverrideSpec] = &[
    ConfigOverrideSpec {
        key: "github_enabled",
        description: "Enable the dedicated GitHub capability using HELM_GITHUB_TOKEN",
        kind: ConfigValueKind::Bool,
    },
    ConfigOverrideSpec {
        key: "provider",
        description: "Provider transport",
        kind: ConfigValueKind::Provider,
    },
    ConfigOverrideSpec {
        key: "model",
        description: "Default model",
        kind: ConfigValueKind::Model,
    },
    ConfigOverrideSpec {
        key: "chat_use_max_tokens",
        description: "Use the compatible Chat max_tokens parameter",
        kind: ConfigValueKind::Bool,
    },
    ConfigOverrideSpec {
        key: "api_key_required",
        description: "Require the named API key (false explicitly disables authentication)",
        kind: ConfigValueKind::Bool,
    },
    ConfigOverrideSpec {
        key: "api_key_env",
        description: "API-key environment variable",
        kind: ConfigValueKind::EnvironmentName,
    },
    ConfigOverrideSpec {
        key: "base_url",
        description: "Provider API base URL",
        kind: ConfigValueKind::Url,
    },
    ConfigOverrideSpec {
        key: "chatgpt_base_url",
        description: "ChatGPT API base URL",
        kind: ConfigValueKind::Url,
    },
    ConfigOverrideSpec {
        key: "system_prompt",
        description: "Agent system guidance",
        kind: ConfigValueKind::Text,
    },
    ConfigOverrideSpec {
        key: "context_window",
        description: "Optional context limit (0 disables local token gating)",
        kind: ConfigValueKind::NonNegativeInteger,
    },
    ConfigOverrideSpec {
        key: "max_tokens",
        description: "Optional response token limit (0 uses provider defaults)",
        kind: ConfigValueKind::NonNegativeInteger,
    },
    ConfigOverrideSpec {
        key: "temperature",
        description: "Sampling temperature",
        kind: ConfigValueKind::Temperature,
    },
    ConfigOverrideSpec {
        key: "provider_retry_attempts",
        description: "Provider retry attempts",
        kind: ConfigValueKind::NonNegativeInteger,
    },
    ConfigOverrideSpec {
        key: "provider_retry_initial_ms",
        description: "Initial provider retry delay",
        kind: ConfigValueKind::NonNegativeInteger,
    },
    ConfigOverrideSpec {
        key: "provider_retry_max_ms",
        description: "Maximum provider retry delay",
        kind: ConfigValueKind::NonNegativeInteger,
    },
    ConfigOverrideSpec {
        key: "command_timeout_secs",
        description: "Shell command timeout",
        kind: ConfigValueKind::PositiveInteger,
    },
    ConfigOverrideSpec {
        key: "max_output_bytes",
        description: "Maximum captured tool output",
        kind: ConfigValueKind::PositiveInteger,
    },
    ConfigOverrideSpec {
        key: "terminal_max_count",
        description: "Maximum persistent terminals",
        kind: ConfigValueKind::PositiveInteger,
    },
    ConfigOverrideSpec {
        key: "terminal_max_unread_bytes",
        description: "Maximum unread terminal output",
        kind: ConfigValueKind::PositiveInteger,
    },
    ConfigOverrideSpec {
        key: "subagent_max_concurrency",
        description: "Concurrent subagent limit",
        kind: ConfigValueKind::PositiveInteger,
    },
    ConfigOverrideSpec {
        key: "subagent_event_history",
        description: "Retained subagent events",
        kind: ConfigValueKind::PositiveInteger,
    },
    ConfigOverrideSpec {
        key: "access",
        description: "Default access mode",
        kind: ConfigValueKind::Access,
    },
    ConfigOverrideSpec {
        key: "approval",
        description: "Legacy approval policy",
        kind: ConfigValueKind::Approval,
    },
    ConfigOverrideSpec {
        key: "unattended_approval",
        description: "Unattended approval policy",
        kind: ConfigValueKind::UnattendedApproval,
    },
    ConfigOverrideSpec {
        key: "workspace",
        description: "Default workspace",
        kind: ConfigValueKind::Path,
    },
    ConfigOverrideSpec {
        key: "allow_read",
        description: "Additional readable roots",
        kind: ConfigValueKind::PathList,
    },
    ConfigOverrideSpec {
        key: "allow_write",
        description: "Additional writable roots",
        kind: ConfigValueKind::PathList,
    },
    ConfigOverrideSpec {
        key: "deny_commands",
        description: "Blocked command names",
        kind: ConfigValueKind::StringList,
    },
    ConfigOverrideSpec {
        key: "env",
        description: "Tool environment map",
        kind: ConfigValueKind::StringMap,
    },
    ConfigOverrideSpec {
        key: "inherit_env",
        description: "Inherited environment names",
        kind: ConfigValueKind::StringList,
    },
    ConfigOverrideSpec {
        key: "redact_values",
        description: "Additional values to redact",
        kind: ConfigValueKind::StringList,
    },
    ConfigOverrideSpec {
        key: "mcp_servers",
        description: "MCP server configuration",
        kind: ConfigValueKind::McpServers,
    },
    ConfigOverrideSpec {
        key: "codex_command",
        description: "Compatibility bridge executable",
        kind: ConfigValueKind::Executable,
    },
];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    Always,
    #[default]
    OnRisk,
    Never,
}

fn approval_is_default(mode: &ApprovalMode) -> bool {
    *mode == ApprovalMode::OnRisk
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum AccessMode {
    ReadOnly,
    #[default]
    Approval,
    Unrestricted,
}

impl std::fmt::Display for AccessMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::ReadOnly => "read-only",
            Self::Approval => "approval",
            Self::Unrestricted => "unrestricted",
        })
    }
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
            github_enabled: false,
            provider: ProviderKind::OpenaiResponses,
            model: "gpt-5".into(),
            api_key_env: "OPENAI_API_KEY".into(),
            api_key_required: true,
            chat_use_max_tokens: false,
            base_url: None,
            chatgpt_base_url: None,
            system_prompt: include_str!("../prompts/system.md").trim().into(),
            max_tokens: 0,
            context_window: 0,
            temperature: None,
            provider_retry_attempts: 4,
            provider_retry_initial_ms: 500,
            provider_retry_max_ms: 8000,
            command_timeout_secs: 120,
            max_output_bytes: 128 * 1024,
            terminal_max_count: 16,
            terminal_max_unread_bytes: 8 * 1024 * 1024,
            subagent_max_concurrency: crate::subagent::default_concurrency(),
            subagent_event_history: 2048,
            access: None,
            approval: ApprovalMode::OnRisk,
            unattended_approval: UnattendedApprovalMode::Deny,
            workspace: None,
            allow_read: Vec::new(),
            allow_write: Vec::new(),
            deny_commands: vec!["shutdown".into(), "reboot".into(), "mkfs".into()],
            env: BTreeMap::new(),
            inherit_env: vec!["PATH".into(), "LANG".into(), "LC_ALL".into(), "TERM".into()],
            redact_values: Vec::new(),
            policy_profile: None,
            policy_defaults: None,
            policy_explicit: Default::default(),
            mcp_servers: BTreeMap::new(),
            codex_command: "codex".into(),
        }
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn context_override_roundtrips_and_invalid_reserves_are_rejected() {
        let mut config = Config::default();
        assert_eq!(config.context_window, 0);
        assert_eq!(config.max_tokens, 0);
        config.apply_override("context_window", "16384").unwrap();
        config.apply_override("max_tokens", "8192").unwrap();
        let restored: Config = toml::from_str(&toml::to_string(&config).unwrap()).unwrap();
        assert_eq!(restored.context_window, 16384);
        assert_eq!(restored.max_tokens, 8192);
        for value in ["8192", "-1"] {
            assert!(config.apply_override("context_window", value).is_err());
            assert_eq!(config.context_window, 16384);
        }
        assert!(config.apply_override("max_tokens", "16384").is_err());
        assert_eq!(config.max_tokens, 8192);
        config.apply_override("context_window", "0").unwrap();
        config.apply_override("max_tokens", "32768").unwrap();
        config.apply_override("max_tokens", "0").unwrap();
        config.apply_override("context_window", "1").unwrap();
        assert!(config.apply_override("max_tokens", "-1").is_err());
        config.apply_override("context_window", "0").unwrap();
        config.validate().unwrap();
    }
}

impl Config {
    /// A display-only snapshot. Never use this to persist or rebuild a runtime:
    /// placeholders deliberately cannot restore the concealed bindings.
    pub fn diagnostic_toml(&self) -> Result<String> {
        let mut displayed = self.clone();
        displayed.access = Some(self.access_mode());
        for value in displayed
            .env
            .values_mut()
            .chain(displayed.redact_values.iter_mut())
            .chain(
                displayed
                    .mcp_servers
                    .values_mut()
                    .flat_map(|server| server.env.values_mut()),
            )
        {
            *value = "[REDACTED]".into();
        }
        // Redact structurally before serialization so escaping, short values,
        // and empty bindings cannot evade the diagnostic boundary.
        toml::to_string_pretty(&displayed)
            .map_err(|_| anyhow::anyhow!("could not serialize concealed configuration"))
    }

    pub(crate) fn parse_loaded(text: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(text)?;
        config.apply_provider_defaults();
        config.validate()?;
        Ok(config)
    }
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
        if !self.api_key_required {
            self.validate()?;
            return Ok(String::new());
        }
        let key = env::var(&self.api_key_env)
            .with_context(|| format!("{} is not set", self.api_key_env))?;
        if key.trim().is_empty() {
            bail!("configured API key is empty");
        }
        Ok(key)
    }

    /// Returns a secret only for transports whose authentication Helm owns.
    pub fn api_key_for_redaction(&self) -> Option<String> {
        (self.api_key_required
            && !matches!(
                self.provider,
                ProviderKind::CodexSubscription | ProviderKind::ChatGptOauth
            ))
        .then(|| self.api_key().ok())
        .flatten()
    }

    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.command_timeout_secs)
    }

    /// Resolve the new access model, translating old configuration files.
    pub fn access_mode(&self) -> AccessMode {
        self.access.unwrap_or(match self.approval {
            ApprovalMode::Never => AccessMode::Unrestricted,
            ApprovalMode::Always | ApprovalMode::OnRisk => AccessMode::Approval,
        })
    }

    pub fn provider_profile(&self) -> ProviderProfile {
        let mut profile = self.provider.profile();
        if !self.api_key_required {
            profile.credential = "none (explicit no-auth endpoint)";
        }
        if self.base_url.is_some()
            && matches!(
                self.provider,
                ProviderKind::OpenaiChat | ProviderKind::OpenaiResponses
            )
        {
            profile.billing = "Billing is determined by the configured endpoint; no ChatGPT subscription credentials are used";
        }
        profile
    }

    pub fn select_provider(&mut self, provider: ProviderKind) {
        let previous = self.provider.clone();
        self.provider = provider;
        if previous != self.provider {
            self.api_key_required = true;
            match self.provider {
                ProviderKind::OpenaiResponses | ProviderKind::OpenaiChat => {
                    if self.api_key_env == "ANTHROPIC_API_KEY" {
                        self.api_key_env = "OPENAI_API_KEY".into();
                    }
                    if self.model == "claude-sonnet-4-0" {
                        self.model = "gpt-5".into();
                    }
                }
                ProviderKind::Anthropic => {}
                ProviderKind::ChatGptOauth => {}
                ProviderKind::CodexSubscription => {}
            }
        }
        self.apply_provider_defaults();
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

    pub fn apply_override(&mut self, key: &str, raw_value: &str) -> Result<()> {
        let segments = key
            .split('.')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if segments.is_empty() || raw_value.trim().is_empty() {
            bail!("configuration overrides require KEY and VALUE");
        }
        let mut document = toml::Value::try_from(self.clone())?;
        let root = document
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("configuration root is not a table"))?;
        if !CONFIG_OVERRIDE_SPECS
            .iter()
            .any(|spec| spec.key == segments[0])
        {
            bail!("unknown configuration key `{}`", segments[0]);
        }
        let parsed = toml::from_str::<toml::Value>(&format!("value = {raw_value}"))
            .ok()
            .and_then(|mut value| value.as_table_mut()?.remove("value"))
            .unwrap_or_else(|| toml::Value::String(raw_value.to_owned()));
        let mut table = root;
        for segment in &segments[..segments.len() - 1] {
            let value = table
                .entry((*segment).to_owned())
                .or_insert_with(|| toml::Value::Table(Default::default()));
            table = value
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("configuration key `{segment}` is not a table"))?;
        }
        table.insert(segments[segments.len() - 1].to_owned(), parsed);
        let mut updated: Self = document.try_into()?;
        // Serialization deliberately drops launch authority; in-process edits must
        // retain it so the next rebuild checks the same profile and transition.
        updated.policy_profile = self.policy_profile.clone();
        updated.policy_explicit = self.policy_explicit.clone();
        if updated.provider != self.provider {
            updated.api_key_required = true;
        }
        updated.apply_provider_defaults();
        updated.validate()?;
        *self = updated;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if !self.api_key_required {
            if !matches!(
                self.provider,
                ProviderKind::OpenaiChat | ProviderKind::OpenaiResponses
            ) {
                bail!("no-auth mode requires a native OpenAI-compatible transport");
            }
            crate::local_provider::validate_endpoint(self.base_url.as_deref().unwrap_or(""))?;
        }
        if self.context_window > 0
            && self.max_tokens > 0
            && self.max_tokens as usize >= self.context_window
        {
            bail!("explicit context_window must exceed explicit max_tokens");
        }
        if self.model.trim().is_empty() {
            bail!("model cannot be empty");
        }
        if self.max_output_bytes < 1024 {
            bail!("max_output_bytes must be at least 1024");
        }
        if self.terminal_max_count == 0 || self.terminal_max_unread_bytes < 1024 {
            bail!("terminal limits require a positive count and at least 1024 unread bytes");
        }
        if self.subagent_max_concurrency == 0
            || self.subagent_max_concurrency > tokio::sync::Semaphore::MAX_PERMITS
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
            bail!("codex_command cannot be empty for the codex-compatibility provider");
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
    fn github_opt_in_preserves_secret_inheritance_refusal() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config.toml");
        std::fs::write(&path, "github_enabled = true\n").unwrap();
        let mut config = Config::load(Some(&path)).unwrap();
        assert!(config.github_enabled);
        assert!(config.validate().is_ok());
        config.inherit_env.push("HELM_GITHUB_TOKEN".into());
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("refusing to inherit")
        );
        assert!(!Config::default().github_enabled);
    }

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
    fn validated_runtime_overrides_support_scalars_and_nested_maps() {
        let mut config = Config::default();
        config.apply_override("max_tokens", "32").unwrap();
        config.apply_override("access", "unrestricted").unwrap();
        config.apply_override("env.HELM_TEST", "enabled").unwrap();
        assert_eq!(config.max_tokens, 32);
        assert_eq!(config.access, Some(AccessMode::Unrestricted));
        assert_eq!(
            config.env.get("HELM_TEST").map(String::as_str),
            Some("enabled")
        );
        assert!(config.apply_override("not_a_setting", "true").is_err());
        assert!(config.apply_override("max_turns", "0").is_err());
    }

    #[test]
    fn legacy_turn_limit_is_ignored_and_not_exposed() {
        for value in [0, 1, 64, 256] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("config.toml");
            std::fs::write(&path, format!("max_turns = {value}\nmax_tokens = 1234\n")).unwrap();
            let mut config = Config::load(Some(&path)).unwrap();
            assert_eq!(config.max_tokens, 1234);
            assert!(!toml::to_string(&config).unwrap().contains("max_turns"));
            assert!(
                config
                    .apply_override("max_turns", &value.to_string())
                    .is_err()
            );
        }
        assert!(
            !CONFIG_OVERRIDE_SPECS
                .iter()
                .any(|spec| spec.key == "max_turns")
        );
    }

    #[test]
    fn subagent_defaults_and_legacy_capacity_migrate() {
        for cpus in [0, 1, 2, 3, 4, 16, 127] {
            assert_eq!(
                crate::subagent::concurrency_for_cpus(cpus),
                (cpus / 2).max(1)
            );
        }
        let mut config: Config = toml::from_str("subagent_max_agents = 1\n").unwrap();
        assert_eq!(
            config.subagent_max_concurrency,
            crate::subagent::default_concurrency()
        );
        assert!(
            !toml::to_string(&config)
                .unwrap()
                .contains("subagent_max_agents")
        );
        assert!(config.apply_override("subagent_max_agents", "1").is_err());
        config
            .apply_override("subagent_max_concurrency", "3")
            .unwrap();
        assert_eq!(config.subagent_max_concurrency, 3);
        assert!(
            config
                .apply_override("subagent_max_concurrency", "0")
                .is_err()
        );
    }

    #[test]
    fn runtime_override_schema_has_unique_keys() {
        let keys = CONFIG_OVERRIDE_SPECS
            .iter()
            .map(|spec| spec.key)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(keys.len(), CONFIG_OVERRIDE_SPECS.len());
        assert!(keys.contains("provider"));
        assert!(keys.contains("mcp_servers"));
        assert!(keys.contains("codex_command"));
        let document = toml::Value::try_from(Config::default()).unwrap();
        for key in document.as_table().unwrap().keys() {
            assert!(
                keys.contains(key.as_str()),
                "schema omitted config key {key}"
            );
        }
    }

    #[test]
    fn migrates_legacy_codex_subscription_name() {
        let config: Config =
            toml::from_str("provider = \"codex-subscription\"\nmodel = \"test\"").unwrap();
        assert_eq!(config.provider, ProviderKind::CodexSubscription);
        assert_eq!(config.codex_command, "codex");
        config.validate().unwrap();
        let serialized = toml::to_string(&config).unwrap();
        assert!(serialized.contains("provider = \"codex-compatibility\""));
        assert!(!serialized.contains("codex-subscription"));
    }

    #[test]
    fn preserves_legacy_openai_as_chat_completions() {
        let config: Config = toml::from_str("provider = \"openai\"").unwrap();
        assert_eq!(config.provider, ProviderKind::OpenaiChat);
        assert!(
            toml::to_string(&config)
                .unwrap()
                .contains("provider = \"openai-chat\"")
        );
        assert_eq!(
            config.provider_profile().access,
            ProviderAccess::NativePublicApi
        );
    }

    #[test]
    fn defaults_to_native_responses_without_migrating_legacy_configs() {
        assert_eq!(Config::default().provider, ProviderKind::OpenaiResponses);
        let responses: Config = toml::from_str("provider = \"openai-responses\"").unwrap();
        assert_eq!(responses.provider, ProviderKind::OpenaiResponses);
        let compatible: Config = toml::from_str("provider = \"openai-compatible\"").unwrap();
        assert_eq!(compatible.provider, ProviderKind::OpenaiChat);
        let oauth: Config = toml::from_str("provider = \"chatgpt-oauth\"").unwrap();
        assert_eq!(oauth.provider, ProviderKind::ChatGptOauth);
    }

    #[test]
    fn resolves_access_and_translates_legacy_approval_settings() {
        assert_eq!(Config::default().access_mode(), AccessMode::Approval);
        let always: Config = toml::from_str("approval = \"always\"").unwrap();
        assert_eq!(always.access_mode(), AccessMode::Approval);
        let never: Config = toml::from_str("approval = \"never\"").unwrap();
        assert_eq!(never.access_mode(), AccessMode::Unrestricted);
        let explicit: Config =
            toml::from_str("access = \"read-only\"\napproval = \"never\"").unwrap();
        assert_eq!(explicit.access_mode(), AccessMode::ReadOnly);
    }

    #[test]
    fn compatibility_profile_is_truthful_about_credentials_and_billing() {
        let profile = ProviderKind::CodexSubscription.profile();
        assert_eq!(profile.access, ProviderAccess::ExternalCompatibilityBridge);
        assert!(profile.compatibility_bridge);
        assert!(profile.credential.contains("codex login"));
        assert!(profile.billing.contains("Codex CLI"));
        let config = Config {
            provider: ProviderKind::CodexSubscription,
            api_key_env: "A_VARIABLE_THE_BRIDGE_MUST_NOT_READ".into(),
            ..Config::default()
        };
        assert_eq!(config.api_key_for_redaction(), None);
    }
}
