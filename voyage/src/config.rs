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
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderAccess {
    NativePublicApi,
    NativeChatgptOauth,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: &'static str,
    pub access: ProviderAccess,
    pub credential: &'static str,
    pub billing: &'static str,
}

impl ProviderKind {
    pub fn profile(&self) -> ProviderProfile {
        match self {
            Self::OpenaiResponses => ProviderProfile {
                id: "openai-responses",
                access: ProviderAccess::NativePublicApi,
                credential: "OPENAI_API_KEY (or api_key_env override)",
                billing: "OpenAI API usage is billed separately from ChatGPT subscriptions",
            },
            Self::OpenaiChat => ProviderProfile {
                id: "openai-chat",
                access: ProviderAccess::NativePublicApi,
                credential: "API key named by api_key_env",
                billing: "Billing is determined by the configured OpenAI-compatible endpoint",
            },
            Self::ChatGptOauth => ProviderProfile {
                id: "chatgpt-oauth",
                access: ProviderAccess::NativeChatgptOauth,
                credential: "Vessel-managed ChatGPT OAuth tokens; run `vessel auth login`",
                billing: "Uses the authenticated ChatGPT subscription and its plan limits",
            },
            Self::Anthropic => ProviderProfile {
                id: "anthropic",
                access: ProviderAccess::NativePublicApi,
                credential: "ANTHROPIC_API_KEY (or api_key_env override)",
                billing: "Anthropic API usage is billed by Anthropic",
            },
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Explicit process-local sharing; configuration files cannot enable it.
    #[serde(skip)]
    pub browser: Option<std::sync::Arc<crate::browser::BrowserBroker>>,
    /// Native Vessel coordination routes; credential contents never enter model context.
    pub vessel: crate::tools::VesselSettings,
    /// Authenticated process identity, populated by the supervisor bootstrap only.
    #[serde(skip)]
    pub vessel_context: Option<crate::tools::VesselContext>,
    pub sandbox: crate::sandbox::Settings,
    /// Deny-only source provenance retained in private launch settings. These
    /// entries never grant file access or execution; display projections omit them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extension_private_files: Vec<crate::extensions::PrivateFile>,
    #[serde(skip_serializing_if = "is_false")]
    pub extension_private_files_complete: bool,
    /// Process-local access updates; never persisted or delegated as authority.
    #[serde(skip)]
    pub live_access: Option<std::sync::Arc<crate::policy::LiveAccess>>,
    /// Admitted run's current grant check, never restored from serialized settings.
    #[serde(skip)]
    pub provider_authority: Option<std::sync::Arc<dyn crate::policy::ExecutionAuthority>>,
    /// Runtime-owned artifact storage; never accepted from configuration.
    #[serde(skip)]
    pub artifact_scope: Option<crate::artifacts::Scope>,
    /// Locally accepted participant endpoints; credential contents never enter model context.
    pub participants: Vec<voyage_protocol::process::ParticipantEndpoint>,
    /// Chat-only preference recording; never carried to workers or runtime config files.
    #[serde(default, rename = "_chat_preferences", skip_serializing)]
    pub chat_preferences: Option<crate::chat_preferences::State>,
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
    /// Frozen executing-host identity; absent preserves legacy credential behavior.
    pub account: Option<voyage_protocol::accounts::AccountBinding>,
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
    /// Omit for the provider default; explicit values are transport-validated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// Requested service tier, not a claim of account entitlement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
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
    /// Retired command deny-list. Retained only for saved-document compatibility; never enforced.
    #[doc(hidden)]
    #[serde(default, rename = "deny_commands")]
    pub legacy_deny_commands: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub inherit_env: Vec<String>,
    pub redact_values: Vec<String>,
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerConfig {
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub bearer_token_env: Option<String>,
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
    Sandbox,
    Vessel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigOverrideSpec {
    pub key: &'static str,
    pub description: &'static str,
    pub kind: ConfigValueKind,
}

pub const CONFIG_OVERRIDE_SPECS: &[ConfigOverrideSpec] = &[
    ConfigOverrideSpec {
        key: "vessel",
        description: "Native Vessel coordination settings and named routes",
        kind: ConfigValueKind::Vessel,
    },
    ConfigOverrideSpec {
        key: "sandbox",
        description: "Explicit Linux process isolation configuration",
        kind: ConfigValueKind::Sandbox,
    },
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
        key: "reasoning_effort",
        description: "Reasoning effort override (omit for provider default)",
        kind: ConfigValueKind::Text,
    },
    ConfigOverrideSpec {
        key: "service_tier",
        description: "Service tier override (subject to endpoint and account support)",
        kind: ConfigValueKind::Text,
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
            vessel: Default::default(),
            vessel_context: None,
            browser: None,
            sandbox: Default::default(),
            participants: Vec::new(),
            chat_preferences: None,
            github_enabled: false,
            provider: ProviderKind::OpenaiResponses,
            account: None,
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
            reasoning_effort: None,
            service_tier: None,
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
            legacy_deny_commands: Vec::new(),
            env: BTreeMap::new(),
            inherit_env: vec!["PATH".into(), "LANG".into(), "LC_ALL".into(), "TERM".into()],
            redact_values: Vec::new(),
            live_access: None,
            provider_authority: None,
            artifact_scope: None,
            extension_private_files: Vec::new(),
            extension_private_files_complete: false,
            policy_profile: None,
            policy_defaults: None,
            policy_explicit: Default::default(),
            mcp_servers: BTreeMap::new(),
        }
    }
}

impl Config {
    /// A display-only snapshot. Never use this to persist or rebuild a runtime:
    /// placeholders deliberately cannot restore the concealed bindings.
    pub fn diagnostic_toml(&self) -> Result<String> {
        let mut displayed = self.clone();
        displayed.extension_private_files.clear();
        displayed.extension_private_files_complete = false;
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
        for server in displayed.mcp_servers.values_mut() {
            if server.url.is_some() {
                server.url = Some("[REDACTED]".into());
            }
        }
        // Redact structurally before serialization so escaping, short values,
        // and empty bindings cannot evade the diagnostic boundary.
        let mut document = toml::Value::try_from(&displayed)?;
        if let Some(table) = document.as_table_mut() {
            table.remove("deny_commands");
            if let Some(explicit) = table
                .get_mut("policy_explicit")
                .and_then(toml::Value::as_table_mut)
            {
                explicit.remove("deny_commands");
            }
        }
        toml::to_string_pretty(&document)
            .map_err(|_| anyhow::anyhow!("could not serialize concealed configuration"))
    }

    pub(crate) fn parse_loaded(text: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(text)?;
        config.apply_provider_defaults();
        config.validate()?;
        Ok(config)
    }
    pub(crate) fn protect_extension_file(
        &mut self,
        path: &Path,
        metadata: &std::fs::Metadata,
    ) -> Result<()> {
        let source = crate::extensions::PrivateFile::capture(path, metadata)?;
        if !self.extension_private_files.contains(&source) {
            anyhow::ensure!(
                self.extension_private_files.len() < 256,
                "private configuration provenance capacity reached"
            );
            self.extension_private_files.push(source);
        }
        Ok(())
    }
    pub fn load(explicit: Option<&Path>) -> Result<Self> {
        use std::io::Read;
        let path = explicit.map(PathBuf::from).or_else(default_config_path);
        let mut config = if let Some(path) = path.filter(|p| p.exists()) {
            let file = fs::File::open(&path)
                .with_context(|| format!("failed to read config {}", path.display()))?;
            let metadata = file.metadata()?;
            let mut text = String::new();
            file.take(1024 * 1024 + 1).read_to_string(&mut text)?;
            anyhow::ensure!(text.len() <= 1024 * 1024, "configuration exceeds 1 MiB");
            let mut config: Self = toml::from_str(&text)
                .with_context(|| format!("invalid config {}", path.display()))?;
            config.protect_extension_file(&path, &metadata)?;
            config
        } else {
            Self::default()
        };
        config.extension_private_files_complete = true;
        config.apply_provider_defaults();
        config.validate()?;
        Ok(config)
    }

    /// Apply an explicit, exact host selection. Never refresh a saved generation.
    /// Executing-host migration only. Never call this on a remote Helm draft.
    /// Existing OAuth caches require the explicit old-writers-stopped CLI boundary;
    /// once migrated their original identity is retained, including tombstones.
    pub fn materialize_legacy_account(&mut self) -> Result<()> {
        if self.account.is_some() {
            return Ok(());
        }
        use voyage_protocol::accounts::Transport;
        let binding = match self.provider {
            ProviderKind::ChatGptOauth => Some(crate::accounts::Registry::legacy_store_binding()?
                .map(|(_, binding)| binding)
                .ok_or_else(|| anyhow::anyhow!("Select a named ChatGPT account, or explicitly migrate the retained legacy login after stopping old credential writers: vessel auth accounts migrate-legacy --old-writers-stopped"))?),
            _ if !self.api_key_required => None,
            _ => {
                let (transport, endpoint) = match self.provider {
                    ProviderKind::OpenaiResponses => {
                        (Transport::OpenaiResponses, "https://api.openai.com/v1")
                    }
                    ProviderKind::OpenaiChat => {
                        (Transport::OpenaiChat, "https://api.openai.com/v1")
                    }
                    ProviderKind::Anthropic => {
                        (Transport::Anthropic, "https://api.anthropic.com/v1")
                    }
                    ProviderKind::ChatGptOauth => unreachable!(),
                };
                Some(
                    crate::accounts::Registry::default_host()?.migrate_legacy_api(
                        self.base_url.clone().unwrap_or_else(|| endpoint.into()),
                        transport,
                        self.api_key_env.clone(),
                    )?,
                )
            }
        };
        self.account = binding;
        Ok(())
    }

    pub fn select_account(
        &mut self,
        binding: voyage_protocol::accounts::AccountBinding,
    ) -> Result<()> {
        use voyage_protocol::accounts::Transport;
        let registry = crate::accounts::Registry::default_host()?;
        registry.validate_binding(&binding)?;
        let connection = registry.connection(binding.connection_id)?;
        self.provider = match binding.transport {
            Transport::OpenaiResponses => ProviderKind::OpenaiResponses,
            Transport::OpenaiChat => ProviderKind::OpenaiChat,
            Transport::ChatgptOauth => ProviderKind::ChatGptOauth,
            Transport::Anthropic => ProviderKind::Anthropic,
        };
        if self.provider == ProviderKind::ChatGptOauth {
            self.chatgpt_base_url = Some(connection.endpoint);
        } else {
            self.base_url = Some(connection.endpoint);
        }
        self.account = Some(binding);
        // Explicit named authentication is not an anonymous compatible endpoint.
        self.api_key_required = true;
        self.validate_account()
    }
    pub fn validate_account(&self) -> Result<()> {
        if let Some(binding) = &self.account {
            use voyage_protocol::accounts::Transport;
            let transport = match self.provider {
                ProviderKind::OpenaiResponses => Transport::OpenaiResponses,
                ProviderKind::OpenaiChat => Transport::OpenaiChat,
                ProviderKind::ChatGptOauth => Transport::ChatgptOauth,
                ProviderKind::Anthropic => Transport::Anthropic,
            };
            anyhow::ensure!(binding.transport == transport, "account transport mismatch");
            let registry = crate::accounts::Registry::default_host()?;
            registry.validate_binding(binding)?;
            let connection = registry.connection(binding.connection_id)?;
            let endpoint = match self.provider {
                ProviderKind::ChatGptOauth => self
                    .chatgpt_base_url
                    .as_deref()
                    .unwrap_or("https://chatgpt.com/backend-api/codex"),
                ProviderKind::Anthropic => self
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.anthropic.com/v1"),
                _ => self
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1"),
            };
            anyhow::ensure!(endpoint == connection.endpoint, "account endpoint mismatch");
        }
        Ok(())
    }

    pub fn api_key(&self) -> Result<String> {
        if let Some(binding) = &self.account {
            self.validate_account()?;
            return crate::accounts::Registry::default_host()?.resolve_api_key(binding);
        }
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

    /// Returns a secret only for transports whose authentication the native runtime owns.
    pub fn api_key_for_redaction(&self) -> Option<String> {
        (self.api_key_required && !matches!(self.provider, ProviderKind::ChatGptOauth))
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
            }
        }
        self.apply_provider_defaults();
    }

    pub fn resolve_workspace(&self, cli: Option<PathBuf>) -> Result<PathBuf> {
        let raw = cli
            .or_else(|| self.workspace.clone())
            .map(Ok)
            .unwrap_or_else(env::current_dir)?;
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
        updated.vessel_context = self.vessel_context.clone();
        updated.browser = self.browser.clone();
        updated.live_access = self.live_access.clone();
        updated.provider_authority = self.provider_authority.clone();
        updated.artifact_scope = self.artifact_scope.clone();
        updated.chat_preferences = self.chat_preferences.clone();
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
        anyhow::ensure!(
            self.extension_private_files.len() <= 256
                && self
                    .extension_private_files
                    .iter()
                    .all(|file| file.path.is_absolute()),
            "invalid private configuration provenance"
        );
        for server in self.mcp_servers.values() {
            if let Some(url) = &server.url {
                crate::tools::mcp::validate_http_endpoint(url)?;
                if !server.command.is_empty() || !server.args.is_empty() {
                    bail!("MCP HTTP server cannot also configure a stdio command or arguments");
                }
            } else if server.command.trim().is_empty() || server.bearer_token_env.is_some() {
                bail!("MCP stdio requires command; bearer_token_env requires url");
            }
            if server.bearer_token_env.as_ref().is_some_and(|name| {
                name.is_empty()
                    || !name.bytes().enumerate().all(|(i, b)| {
                        b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit())
                    })
            }) {
                bail!("MCP bearer_token_env must be an environment variable name");
            }
        }
        self.vessel.validate()?;
        self.sandbox.validate()?;
        crate::provider::validate_inference_settings(self)?;
        crate::provider::validate_config_endpoints(self)?;
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
