//! Explicit compatible endpoint setup. No discovery runs during normal startup.
use crate::{Config, config::ProviderKind};
use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{Value, json};
use std::{io::Write, path::PathBuf, time::Duration};

const MAX_BODY: usize = 1024 * 1024;
const MAX_MODELS: usize = 1024;
#[derive(Clone, Copy, Debug, ValueEnum, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    Ollama,
    LmStudio,
    Vllm,
    LlamaCpp,
    Custom,
}
impl Preset {
    pub fn endpoint(self) -> Option<&'static str> {
        match self {
            Self::Ollama => Some("http://127.0.0.1:11434/v1"),
            Self::LmStudio => Some("http://127.0.0.1:1234/v1"),
            Self::Vllm => Some("http://127.0.0.1:8000/v1"),
            Self::LlamaCpp => Some("http://127.0.0.1:8080/v1"),
            Self::Custom => None,
        }
    }
}
const PRESETS: [Preset; 4] = [
    Preset::Ollama,
    Preset::LmStudio,
    Preset::Vllm,
    Preset::LlamaCpp,
];
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum Transport {
    #[default]
    Chat,
    Responses,
}
#[derive(Clone, Debug, Args)]
pub struct EndpointArgs {
    #[arg(value_enum)]
    pub preset: Preset,
    /// API base, including /v1 where required. HTTPS or numeric loopback HTTP.
    #[arg(long)]
    pub endpoint: Option<String>,
    /// Explicit model ID; works when /models is unavailable.
    #[arg(long)]
    pub model: Option<String>,
    /// Read only this environment variable. Omission explicitly selects no auth.
    #[arg(long)]
    pub api_key_env: Option<String>,
    #[arg(long, value_enum, default_value = "chat")]
    pub transport: Transport,
    /// Use max_completion_tokens for Chat endpoints that require the newer field.
    #[arg(long)]
    pub chat_max_completion_tokens: bool,
    /// Total probe deadline, including a tool-free request capped at 16 output tokens.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(1..=120))]
    pub timeout_secs: u64,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show maintained presets without accessing the network.
    Presets,
    /// Discover and validate one endpoint; may generate at most 16 output tokens.
    Probe(EndpointArgs),
    /// Validate then create a new config file; never replace an existing path.
    Setup {
        #[command(flatten)]
        endpoint: EndpointArgs,
        #[arg(long)]
        output: PathBuf,
    },
    /// Explicitly GET /models on four fixed numeric-loopback ports, without credentials.
    Scan,
}

pub fn validate_endpoint(raw: &str) -> Result<String> {
    let url = reqwest::Url::parse(raw).map_err(|_| anyhow::anyhow!("invalid endpoint URL"))?;
    let local = url.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    });
    if !(url.scheme() == "https" || (url.scheme() == "http" && local))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || raw.chars().any(char::is_control)
    {
        bail!(
            "endpoint requires HTTPS or numeric loopback HTTP, without credentials, query or fragment"
        );
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}
fn validate_model(id: &str) -> Result<()> {
    if id.trim().is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
        bail!("model-list failure: invalid model ID");
    }
    Ok(())
}
impl EndpointArgs {
    pub fn resolve(&self) -> Result<Config> {
        let endpoint = validate_endpoint(
            self.endpoint
                .as_deref()
                .or(self.preset.endpoint())
                .unwrap_or(""),
        )?;
        if let Some(name) = &self.api_key_env
            && (name.is_empty()
                || name.len() > 128
                || !name.bytes().enumerate().all(|(i, c)| {
                    c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                }))
        {
            bail!("invalid credential environment variable name");
        }
        if let Some(id) = &self.model {
            validate_model(id)?;
        }
        let config = Config {
            provider: match self.transport {
                Transport::Chat => ProviderKind::OpenaiChat,
                Transport::Responses => ProviderKind::OpenaiResponses,
            },
            base_url: Some(endpoint),
            api_key_required: self.api_key_env.is_some(),
            chat_use_max_tokens: !self.chat_max_completion_tokens,
            api_key_env: self
                .api_key_env
                .clone()
                .unwrap_or_else(|| "OPENAI_API_KEY".into()),
            model: self
                .model
                .clone()
                .unwrap_or_else(|| "manual-model-required".into()),
            ..Config::default()
        };
        config.validate()?;
        Ok(config)
    }
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?)
}
async fn bounded_json(mut response: reqwest::Response) -> Result<Value> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("connection failure while reading response"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            bail!("protocol failure: response exceeds 1 MiB");
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("protocol failure: invalid JSON response"))
}
fn status(response: &reqwest::Response) -> Result<()> {
    let code = response.status();
    if matches!(code.as_u16(), 401 | 403) {
        bail!("authentication failure: endpoint rejected credentials");
    }
    if !code.is_success() {
        bail!("protocol failure: endpoint returned HTTP {}", code.as_u16());
    }
    Ok(())
}
fn authenticate(request: reqwest::RequestBuilder, key: &str) -> reqwest::RequestBuilder {
    if key.is_empty() {
        request
    } else {
        request.bearer_auth(key)
    }
}
async fn models(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
) -> Result<Option<Vec<String>>> {
    let response = authenticate(client.get(format!("{endpoint}/models")), key)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("connection failure: model endpoint unavailable"))?;
    if matches!(response.status().as_u16(), 404 | 405 | 501) {
        return Ok(None);
    }
    status(&response)?;
    let value = bounded_json(response).await?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("model-list failure: missing data array"))?;
    if data.len() > MAX_MODELS {
        bail!("model-list failure: more than 1024 models");
    }
    let mut ids = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("model-list failure: missing model ID"))?;
        validate_model(id)?;
        if !key.is_empty() && id.contains(key) {
            bail!("model-list failure: credential-bearing model ID");
        }
        ids.push(id.to_owned());
    }
    ids.sort();
    ids.dedup();
    Ok(Some(ids))
}
#[derive(Serialize)]
pub struct Report {
    pub endpoint: String,
    pub transport: &'static str,
    pub credential_source: String,
    pub discovery: &'static str,
    pub models: Vec<String>,
    pub selected_model: String,
    pub protocol_validated: bool,
}
pub fn diagnostics(config: &Config) -> Value {
    json!({
        "transport": config.provider_profile().id,
        "endpoint": config.base_url.as_deref().or(match config.provider { ProviderKind::OpenaiChat | ProviderKind::OpenaiResponses => Some("https://api.openai.com/v1"), ProviderKind::Anthropic => Some("https://api.anthropic.com/v1"), _ => None }),
        "credential_source": if matches!(config.provider, ProviderKind::ChatGptOauth | ProviderKind::CodexSubscription) { config.provider_profile().credential.to_owned() } else if config.api_key_required { format!("environment:{}",config.api_key_env) } else { "none".into() },
        "chat_token_limit_parameter": if config.chat_use_max_tokens { "max_tokens" } else { "max_completion_tokens" },
        "discovery": "not_probed; use helm local-provider probe explicitly",
    })
}
async fn probe_inner(args: &EndpointArgs) -> Result<(Config, Report)> {
    let mut config = args.resolve()?;
    let key = config.api_key().map_err(|_| {
        anyhow::anyhow!("authentication failure: configured environment key unavailable")
    })?;
    let endpoint = config.base_url.clone().unwrap();
    if !key.is_empty()
        && (endpoint.contains(&key)
            || config.model.contains(&key)
            || config.api_key_env.contains(&key))
    {
        bail!("credential-bearing configuration metadata is not allowed");
    }
    let client = client()?;
    let discovered = models(&client, &endpoint, &key).await?;
    if args.model.is_none() {
        config.model = discovered
            .as_ref()
            .and_then(|v| v.first())
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!("model-list unavailable or empty: supply --model explicitly")
            })?;
    }
    let (route, mut body) = match args.transport {
        Transport::Chat => (
            "chat/completions",
            json!({"model":config.model,"messages":[{"role":"user","content":"Reply OK."}],"stream":false,"max_tokens":16}),
        ),
        Transport::Responses => (
            "responses",
            json!({"model":config.model,"input":"Reply OK.","stream":false,"max_output_tokens":16,"store":false}),
        ),
    };
    if matches!(args.transport, Transport::Chat) && !config.chat_use_max_tokens {
        let value = body.as_object_mut().unwrap().remove("max_tokens").unwrap();
        body["max_completion_tokens"] = value;
    }
    let response = authenticate(client.post(format!("{endpoint}/{route}")), &key)
        .json(&body)
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("connection failure: protocol endpoint unavailable"))?;
    status(&response)?;
    let value = bounded_json(response).await?;
    let valid = match args.transport {
        Transport::Chat => value
            .get("choices")
            .and_then(Value::as_array)
            .is_some_and(|choices| {
                !choices.is_empty()
                    && choices
                        .iter()
                        .all(|c| c.get("message").is_some_and(Value::is_object))
            }),
        Transport::Responses => {
            value.get("output").is_some_and(Value::is_array)
                && value.get("id").is_some_and(Value::is_string)
        }
    };
    if !valid {
        bail!("protocol failure: incompatible completion response");
    }
    let report = Report {
        endpoint,
        transport: config.provider_profile().id,
        credential_source: if config.api_key_required {
            format!("environment:{}", config.api_key_env)
        } else {
            "none".into()
        },
        discovery: if discovered.is_some() {
            "available"
        } else {
            "unavailable_manual_model"
        },
        models: discovered.unwrap_or_default(),
        selected_model: config.model.clone(),
        protocol_validated: true,
    };
    Ok((config, report))
}
pub async fn probe(args: &EndpointArgs) -> Result<(Config, Report)> {
    tokio::time::timeout(Duration::from_secs(args.timeout_secs), probe_inner(args))
        .await
        .map_err(|_| anyhow::anyhow!("connection timeout: endpoint probe deadline elapsed"))?
}
fn save(config: &Config, output: &std::path::Path) -> Result<()> {
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(toml::to_string_pretty(config)?.as_bytes())?;
    temporary.as_file().sync_all()?;
    temporary.persist_noclobber(output).map_err(|_| anyhow::anyhow!("config publication failed; inspect destination before retrying (existing files are never replaced)"))?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub async fn run(command: Command) -> Result<()> {
    match command {
        Command::Presets => println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"presets": PRESETS.map(|preset|json!({"preset":preset,"endpoint":preset.endpoint(),"transport":"openai-chat","auth":"none unless --api-key-env is supplied"})), "custom":"requires --endpoint"})
            )?
        ),
        Command::Scan => {
            let client = client()?;
            let mut records = Vec::new();
            for preset in PRESETS {
                let endpoint = preset.endpoint().unwrap();
                let result =
                    tokio::time::timeout(Duration::from_secs(2), models(&client, endpoint, ""))
                        .await;
                records.push(match result {
                    Ok(Ok(Some(models))) => json!({"preset":preset,"endpoint":endpoint,"models":models,"discovery":"available","protocol_validated":false}),
                    Ok(Ok(None)) => json!({"preset":preset,"endpoint":endpoint,"discovery":"models_unavailable","protocol_validated":false}),
                    Ok(Err(error)) => json!({"preset":preset,"endpoint":endpoint,"error":error.to_string()}),
                    Err(_) => json!({"preset":preset,"endpoint":endpoint,"error":"connection timeout"}),
                });
            }
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        Command::Probe(args) => {
            println!("{}", serde_json::to_string_pretty(&probe(&args).await?.1)?)
        }
        Command::Setup { endpoint, output } => {
            if output.try_exists()? || output.symlink_metadata().is_ok() {
                bail!("config destination already exists");
            }
            let candidate = endpoint.resolve()?;
            let key = candidate.api_key().map_err(|_| {
                anyhow::anyhow!("authentication failure: configured environment key unavailable")
            })?;
            if !key.is_empty() && output.to_string_lossy().contains(&key) {
                bail!("credential-bearing configuration metadata is not allowed");
            }
            let (config, report) = probe(&endpoint).await?;
            save(&config, &output)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"saved":output,"provider":report}))?
            );
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests;
