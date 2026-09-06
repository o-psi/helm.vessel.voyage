//! Versioned, local declarative prompts. Definitions never grant runtime authority.
use anyhow::{Result, bail, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
};
mod prompt;
pub use prompt::{InputFailure, InputMonitor};
#[cfg(test)]
mod prompt_tests;

pub const MAX_DOCUMENT: usize = 64 * 1024;
const MAX_RENDER: usize = 128 * 1024;
const MAX_FILES: usize = 128;
const MAX_PARAMETERS: usize = 32;
pub const RESERVED: &[&str] = &[
    "policy",
    "policy-directory",
    "policy-profile",
    "policy-revision",
    "policy-digest",
    "policy-confirm",
    "activity",
    "branch",
    "plain",
    "help",
    "exit",
    "quit",
    "run",
    "chat",
    "sessions",
    "models",
    "model",
    "config",
    "doctor",
    "auth",
    "attachment",
    "onboard",
    "local-provider",
    "workflow",
    "voyages",
    "completions",
    "manpage",
    "resume",
    "new",
    "clear",
    "compact",
    "export",
    "name",
    "rename",
    "set",
    "access",
    "provider",
    "workspace",
    "tools",
    "todos",
    "agents",
    "terminals",
    "terminal",
    "verbose",
    "log-format",
    "version",
    "managed",
    "remote-worker",
];
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    pub description: String,
    pub prompt: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
    #[serde(default)]
    pub recommended: Recommendations,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Recommendations {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub access: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    #[serde(rename = "type")]
    pub kind: ParameterType,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub secret: bool,
    pub default: Option<Value>,
    #[serde(default)]
    pub choices: Vec<Value>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub max_length: Option<usize>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterType {
    String,
    Integer,
    Boolean,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invocation {
    pub id: String,
    pub version: String,
    pub digest: String,
    pub scope: Scope,
    pub inputs: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize)]
pub struct Rendered {
    pub prompt: String,
    pub inputs: BTreeMap<String, Value>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    User,
    Repository,
}
#[derive(Clone, Debug, Serialize)]
pub struct Definition {
    pub scope: Scope,
    pub digest: String,
    pub document: Document,
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
pub fn parse(bytes: &[u8]) -> Result<Document> {
    ensure!(
        bytes.len() <= MAX_DOCUMENT,
        "workflow document exceeds 64 KiB"
    );
    let text = std::str::from_utf8(bytes)
        .map_err(|_| anyhow::anyhow!("workflow document must be UTF-8"))?;
    let value: Document = toml::from_str(text).map_err(|_| {
        anyhow::anyhow!("invalid workflow document; inspect schema and field types")
    })?;
    value.validate()?;
    Ok(value)
}
impl Parameter {
    fn decode(&self, text: &str) -> Result<Value> {
        ensure!(text.len() <= 8192, "workflow input exceeds limit");
        let value = match self.kind {
            ParameterType::String => Value::String(text.to_owned()),
            ParameterType::Integer => Value::from(
                text.parse::<i64>()
                    .map_err(|_| anyhow::anyhow!("workflow integer input is invalid"))?,
            ),
            ParameterType::Boolean => Value::Bool(
                text.parse::<bool>()
                    .map_err(|_| anyhow::anyhow!("workflow boolean input must be true or false"))?,
            ),
        };
        ensure!(
            self.accepts(&value),
            "workflow input fails type, bounds or choices"
        );
        Ok(value)
    }
    fn accepts(&self, value: &Value) -> bool {
        let typed = match self.kind {
            ParameterType::String => value
                .as_str()
                .is_some_and(|v| v.len() <= self.max_length.unwrap_or(8192)),
            ParameterType::Integer => value.as_i64().is_some_and(|v| {
                self.minimum.is_none_or(|m| v >= m) && self.maximum.is_none_or(|m| v <= m)
            }),
            ParameterType::Boolean => value.is_boolean(),
        };
        typed && (self.choices.is_empty() || self.choices.contains(value))
    }
}
impl Document {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1,
            "unsupported workflow schema version"
        );
        ensure!(
            identifier(&self.id) && !RESERVED.contains(&self.id.as_str()),
            "invalid or reserved workflow identifier"
        );
        ensure!(
            !self.version.is_empty()
                && self.version.len() <= 64
                && self
                    .version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b)),
            "invalid workflow version"
        );
        ensure!(
            !self.description.trim().is_empty() && self.description.len() <= 2048,
            "workflow description must contain 1..2048 bytes"
        );
        ensure!(
            !self.prompt.trim().is_empty() && self.prompt.len() <= 32768,
            "workflow prompt must contain 1..32768 bytes"
        );
        ensure!(
            self.parameters.len() <= MAX_PARAMETERS,
            "too many workflow parameters"
        );
        for (name, p) in &self.parameters {
            ensure!(identifier(name), "invalid parameter name");
            ensure!(
                p.description.len() <= 2048 && p.choices.len() <= 64,
                "parameter description or choices exceed limit"
            );
            ensure!(
                p.max_length.is_none_or(|n| n > 0 && n <= 8192),
                "parameter string limit must be 1..8192 bytes"
            );
            ensure!(
                p.minimum.zip(p.maximum).is_none_or(|(a, b)| a <= b),
                "invalid integer bounds"
            );
            ensure!(
                !p.secret || (p.default.is_none() && p.choices.is_empty()),
                "secret parameters cannot contain defaults or choices"
            );
            ensure!(
                matches!(p.kind, ParameterType::Integer)
                    || (p.minimum.is_none() && p.maximum.is_none()),
                "integer bounds require integer parameter"
            );
            ensure!(
                matches!(p.kind, ParameterType::String) || p.max_length.is_none(),
                "string bounds require string parameter"
            );
            ensure!(
                p.default.as_ref().is_none_or(|v| p.accepts(v)),
                "invalid workflow parameter default"
            );
            for choice in &p.choices {
                ensure!(p.accepts(choice), "invalid workflow parameter choice");
            }
        }
        for value in [
            &self.recommended.provider,
            &self.recommended.model,
            &self.recommended.access,
        ]
        .into_iter()
        .flatten()
        {
            ensure!(value.len() <= 256, "recommendation exceeds limit");
        }
        self.template(|name| {
            ensure!(
                self.parameters.contains_key(name),
                "template references unknown parameter"
            );
            Ok(String::new())
        })?;
        Ok(())
    }
    fn template(&self, mut value: impl FnMut(&str) -> Result<String>) -> Result<String> {
        let mut output = String::new();
        let mut rest = self.prompt.as_str();
        while let Some(start) = rest.find("{{") {
            ensure!(
                !rest[..start].contains("}}"),
                "unmatched template delimiter"
            );
            output.push_str(&rest[..start]);
            rest = &rest[start + 2..];
            let end = rest
                .find("}}")
                .ok_or_else(|| anyhow::anyhow!("unclosed template parameter"))?;
            let name = &rest[..end];
            ensure!(identifier(name), "invalid template parameter");
            output.push_str(&value(name)?);
            ensure!(
                output.len() <= MAX_RENDER,
                "rendered workflow exceeds limit"
            );
            rest = &rest[end + 2..];
        }
        ensure!(!rest.contains("}}"), "unmatched template delimiter");
        output.push_str(rest);
        ensure!(
            output.len() <= MAX_RENDER,
            "rendered workflow exceeds limit"
        );
        Ok(output)
    }
    pub fn render(&self, supplied: &[(String, String)]) -> Result<Rendered> {
        ensure!(
            !self.parameters.values().any(|p| p.secret),
            "secret parameters require the isolated workflow binding renderer"
        );
        self.validate()?;
        ensure!(supplied.len() <= MAX_PARAMETERS, "too many workflow inputs");
        let mut inputs = BTreeMap::new();
        for (name, text) in supplied {
            let parameter = self
                .parameters
                .get(name)
                .ok_or_else(|| anyhow::anyhow!("unknown workflow input"))?;
            let value = parameter.decode(text)?;
            ensure!(
                inputs.insert(name.clone(), value).is_none(),
                "duplicate workflow input"
            );
        }
        for (name, p) in &self.parameters {
            if !inputs.contains_key(name) {
                if let Some(value) = &p.default {
                    inputs.insert(name.clone(), value.clone());
                } else {
                    ensure!(
                        !p.required,
                        "required workflow input is missing; inspect parameters and supply --input NAME=VALUE"
                    );
                    inputs.insert(name.clone(), Value::Null);
                }
            }
        }
        let prompt = self.template(|name| Ok(serde_json::to_string(&inputs[name])?))?;
        Ok(Rendered { prompt, inputs })
    }
}
impl Definition {
    pub fn authorize(&self, trust: Option<&str>) -> Result<()> {
        ensure!(
            self.scope != Scope::Repository || trust == Some(self.digest.as_str()),
            "repository workflow requires --trust-repository with the exact digest shown by inspect"
        );
        Ok(())
    }
    pub fn invocation(&self, inputs: BTreeMap<String, Value>) -> Invocation {
        Invocation {
            id: self.document.id.clone(),
            version: self.document.version.clone(),
            digest: self.digest.clone(),
            scope: self.scope,
            inputs,
        }
    }
}
// Open every untrusted component relative to a retained directory capability.
fn scope_directory(base: &Path, components: &[&str]) -> Result<Option<Dir>> {
    let mut directory = Dir::open_ambient_dir(base, cap_std::ambient_authority())?;
    for component in components {
        match directory.open_dir_nofollow(component) {
            Ok(next) => directory = next,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => bail!("workflow directory must be a real directory without symlinks"),
        }
    }
    Ok(Some(directory))
}
pub fn discover(workspace: &Path, user_root: Option<&Path>) -> Result<Vec<Definition>> {
    let user = if let Some(root) = user_root {
        if root.exists() {
            Some((root.to_owned(), Vec::new()))
        } else {
            None
        }
    } else {
        crate::config::default_config_path()
            .and_then(|p| p.parent().map(|p| (p.to_owned(), vec!["workflows"])))
            .filter(|(p, _)| p.exists())
    };
    let mut definitions = Vec::new();
    for (scope, base, parts) in user
        .into_iter()
        .map(|(base, parts)| (Scope::User, base, parts))
        .chain(std::iter::once((
            Scope::Repository,
            workspace.to_owned(),
            vec![".helm", "workflows"],
        )))
    {
        let Some(directory) = scope_directory(&base, &parts)? else {
            continue;
        };
        let mut names = Vec::new();
        for entry in directory.entries()?.take(MAX_FILES + 1) {
            names.push(entry?.file_name());
        }
        ensure!(
            names.len() <= MAX_FILES,
            "workflow directory exceeds 128 entries"
        );
        names.sort();
        for name in names {
            let Some(name) = name.to_str().filter(|n| n.ends_with(".toml")) else {
                continue;
            };
            let mut options = OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No).nonblock(true);
            let file = directory
                .open_with(name, &options)
                .map_err(|_| anyhow::anyhow!("workflow input must not be a symlink"))?;
            let metadata = file.metadata()?;
            ensure!(
                metadata.is_file() && metadata.len() <= MAX_DOCUMENT as u64,
                "workflow input must be a bounded regular file"
            );
            let mut bytes = Vec::new();
            file.take(MAX_DOCUMENT as u64 + 1).read_to_end(&mut bytes)?;
            let document = parse(&bytes)?;
            ensure!(
                name == format!("{}.toml", document.id),
                "workflow filename must match its identifier"
            );
            definitions.push(Definition {
                scope,
                digest: hex::encode(Sha256::digest(&bytes)),
                document,
            });
        }
    }
    definitions.sort_by(|a, b| {
        a.document
            .id
            .cmp(&b.document.id)
            .then((a.scope as u8).cmp(&(b.scope as u8)))
    });
    Ok(definitions)
}
pub fn select(definitions: Vec<Definition>, id: &str, scope: Option<Scope>) -> Result<Definition> {
    ensure!(
        identifier(id) && !RESERVED.contains(&id),
        "invalid or reserved workflow identifier"
    );
    definitions
        .into_iter()
        .rfind(|d| d.document.id == id && scope.is_none_or(|s| d.scope == s))
        .ok_or_else(|| anyhow::anyhow!("workflow not found in selected scope"))
}
#[derive(clap::Args)]
pub struct WorkflowArgs {
    /// Directory containing user workflow TOML files; defaults to Helm config/workflows.
    #[arg(long, global = true)]
    pub user_directory: Option<PathBuf>,
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: WorkflowCommand,
}
#[derive(clap::Subcommand)]
pub enum WorkflowCommand {
    List,
    Inspect(Selection),
    Validate(Selection),
    Preview(InputArgs),
    Run(InputArgs),
}
#[derive(clap::Args)]
pub struct Selection {
    pub id: String,
    #[arg(long, value_enum)]
    pub scope: Option<Scope>,
}
#[derive(clap::Args)]
pub struct InputArgs {
    #[command(flatten)]
    pub selection: Selection,
    /// Nonsecret typed values only. String values are rendered as JSON strings.
    #[arg(long = "input", value_name = "NAME=VALUE")]
    pub inputs: Vec<String>,
    /// Explicit transient secret source; pass an environment VARIABLE name, never its value.
    /// Preview validates names without reading the environment. Only one-shot shell calls
    /// opting into the public reference receive the value; their output is suppressed.
    #[arg(long = "secret-env", value_name = "NAME=VARIABLE")]
    pub secret_env: Vec<String>,
    /// Explicit trust in this exact repository workflow's SHA-256 digest.
    #[arg(long)]
    pub trust_repository: Option<String>,
    #[arg(long)]
    pub no_save: bool,
    /// Collect required missing inputs from an attended terminal before preview/run.
    #[arg(long)]
    pub prompt_missing: bool,
    /// Total attended collection deadline, including retries (1..300 seconds).
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u64).range(1..=300))]
    pub input_timeout_seconds: u64,
}
pub struct Prepared {
    pub input_monitor: Option<InputMonitor>,
    pub secrets: secrets::SecretInputs,
    pub prompt: String,
    pub invocation: Invocation,
    pub no_save: bool,
}
pub async fn prepare(args: WorkflowArgs, workspace: &Path) -> Result<Option<Prepared>> {
    let definitions = discover(workspace, args.user_directory.as_deref())?;
    let is_run = matches!(&args.command, WorkflowCommand::Run(_));
    let output = match args.command {
        WorkflowCommand::List => {
            serde_json::json!({"workflows":definitions.iter().map(|d|serde_json::json!({"id":d.document.id,"version":d.document.version,"description":d.document.description,"scope":d.scope,"digest":d.digest})).collect::<Vec<_>>()})
        }
        WorkflowCommand::Inspect(s) => serde_json::to_value(select(definitions, &s.id, s.scope)?)?,
        WorkflowCommand::Validate(s) => {
            let d = select(definitions, &s.id, s.scope)?;
            serde_json::json!({"valid":true,"id":d.document.id,"digest":d.digest})
        }
        WorkflowCommand::Preview(mut input) | WorkflowCommand::Run(mut input) => {
            let d = select(definitions, &input.selection.id, input.selection.scope)?;
            d.authorize(input.trust_repository.as_deref())?;
            let mut collected = Vec::new();
            let mut input_monitor = None;
            if input.prompt_missing {
                ensure!(
                    (1..=300).contains(&input.input_timeout_seconds),
                    "workflow input timeout must be 1..300 seconds"
                );
                let fields = missing_fields(&d, &input, is_run)?;
                if !fields.is_empty() {
                    let entered = prompt::collect(
                        fields,
                        std::time::Duration::from_secs(input.input_timeout_seconds),
                    )
                    .await?;
                    input_monitor = Some(entered.monitor);
                    for (name, value) in entered.values {
                        if d.document.parameters[&name].secret {
                            collected.push((name, value));
                        } else {
                            input.inputs.push(format!("{name}={}", *value));
                        }
                    }
                }
                if !is_run {
                    let sources = parse_inputs(&d, &input)?.sources;
                    for (name, parameter) in &d.document.parameters {
                        if parameter.secret && parameter.required && !sources.contains_key(name) {
                            collected.push((name.clone(), zeroize::Zeroizing::new(String::new())));
                        }
                    }
                }
                let fresh = select(
                    discover(workspace, args.user_directory.as_deref())?,
                    &input.selection.id,
                    input.selection.scope,
                )?;
                ensure!(
                    fresh.scope == d.scope && fresh.digest == d.digest,
                    "workflow definition changed during input collection; inspect and retry"
                );
                fresh.authorize(input.trust_repository.as_deref())?;
            }
            let lookup = |name: &str| {
                std::env::var(name).map_err(|_| {
                    anyhow::anyhow!("workflow secret environment source is missing or not UTF-8")
                })
            };
            let mut prepared = if input.prompt_missing {
                prepare_inputs_collected(&d, input, is_run, lookup, collected)
            } else {
                prepare_inputs(&d, input, is_run, lookup)
            }?;
            prepared.input_monitor = input_monitor;
            return Ok(Some(prepared));
        }
    };
    print_value(&output, args.json)?;
    Ok(None)
}
fn prepare_inputs(
    definition: &Definition,
    input: InputArgs,
    is_run: bool,
    lookup: impl FnMut(&str) -> Result<String>,
) -> Result<Prepared> {
    prepare_inputs_collected(definition, input, is_run, lookup, Vec::new())
}
struct ParsedInputs {
    supplied: Vec<(String, String)>,
    sources: BTreeMap<String, String>,
}
fn parse_inputs(definition: &Definition, input: &InputArgs) -> Result<ParsedInputs> {
    ensure!(
        input.inputs.len() <= MAX_PARAMETERS && input.secret_env.len() <= MAX_PARAMETERS,
        "too many workflow inputs"
    );
    let supplied = input
        .inputs
        .iter()
        .map(|value| {
            value
                .split_once('=')
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .ok_or_else(|| anyhow::anyhow!("workflow input must be NAME=VALUE"))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut sources = BTreeMap::new();
    for reference in &input.secret_env {
        let (name, environment) = reference
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("workflow secret source must be NAME=VARIABLE"))?;
        ensure!(
            definition
                .document
                .parameters
                .get(name)
                .is_some_and(|p| p.secret),
            "unknown workflow secret input"
        );
        ensure!(
            !environment.is_empty()
                && environment.len() <= 128
                && environment
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_alphabetic()
                        || b == b'_'
                        || (i > 0 && b.is_ascii_digit())),
            "invalid workflow secret environment source name"
        );
        ensure!(
            sources
                .insert(name.to_owned(), environment.to_owned())
                .is_none(),
            "duplicate workflow secret input"
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    for (name, text) in &supplied {
        let parameter = definition
            .document
            .parameters
            .get(name)
            .filter(|parameter| !parameter.secret)
            .ok_or_else(|| {
                anyhow::anyhow!("unknown workflow input; secret values require --secret-env")
            })?;
        parameter.decode(text)?;
        ensure!(seen.insert(name), "duplicate workflow input");
    }
    Ok(ParsedInputs { supplied, sources })
}
fn missing_fields(
    definition: &Definition,
    input: &InputArgs,
    is_run: bool,
) -> Result<Vec<prompt::Field>> {
    definition.authorize(input.trust_repository.as_deref())?;
    let ParsedInputs { supplied, sources } = parse_inputs(definition, input)?;
    Ok(definition
        .document
        .parameters
        .iter()
        .filter(|(name, parameter)| {
            parameter.required
                && parameter.default.is_none()
                && if parameter.secret {
                    is_run && !sources.contains_key(*name)
                } else {
                    !supplied.iter().any(|(provided, _)| provided == *name)
                }
        })
        .map(|(name, parameter)| prompt::Field {
            name: name.clone(),
            parameter: parameter.clone(),
        })
        .collect())
}
fn prepare_inputs_collected(
    definition: &Definition,
    input: InputArgs,
    is_run: bool,
    mut lookup: impl FnMut(&str) -> Result<String>,
    mut collected: Vec<(String, zeroize::Zeroizing<String>)>,
) -> Result<Prepared> {
    definition.authorize(input.trust_repository.as_deref())?;
    let ParsedInputs { supplied, sources } = parse_inputs(definition, &input)?;
    let names = sources
        .keys()
        .chain(collected.iter().map(|(name, _)| name))
        .cloned()
        .collect();
    // Validate public inputs and reference completeness before touching private sources.
    let rendered = secrets::render_public(&definition.document, &supplied, &names)?;
    let mut values = Vec::new();
    if is_run {
        // Keep earlier resolved values clearing-on-drop if a later source fails.
        let mut held = std::mem::take(&mut collected);
        for (name, source) in sources {
            held.push((name, zeroize::Zeroizing::new(lookup(&source)?)));
        }
        values = held
            .into_iter()
            .map(|(name, mut value)| (name, std::mem::take(&mut *value)))
            .collect();
    }
    let secrets = secrets::SecretInputs::collect(&definition.document, values)?;
    Ok(Prepared {
        input_monitor: None,
        prompt: rendered.prompt,
        invocation: definition.invocation(rendered.inputs),
        no_save: input.no_save,
        secrets,
    })
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}
pub fn print_value(value: &Value, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string(value)?);
        return Ok(());
    }
    if let Some(workflows) = value.get("workflows").and_then(Value::as_array) {
        if workflows.is_empty() {
            println!("No saved workflows found.");
        }
        for workflow in workflows {
            println!(
                "{}  {}  {}\n  SHA-256 {}",
                workflow["id"].as_str().unwrap_or_default(),
                workflow["version"].as_str().unwrap_or_default(),
                workflow["scope"].as_str().unwrap_or_default(),
                workflow["digest"].as_str().unwrap_or_default()
            );
        }
    } else if let Some(prompt) = value.get("prompt").and_then(Value::as_str) {
        println!(
            "Workflow {} version {}\nSHA-256 {}\n\n{}",
            value["workflow"]["id"].as_str().unwrap_or_default(),
            value["workflow"]["version"].as_str().unwrap_or_default(),
            value["workflow"]["digest"].as_str().unwrap_or_default(),
            safe_text(prompt)
        );
    } else if value.get("valid") == Some(&Value::Bool(true)) {
        println!(
            "Workflow {} is valid.\nSHA-256 {}",
            value["id"].as_str().unwrap_or_default(),
            value["digest"].as_str().unwrap_or_default()
        );
    } else {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    Ok(())
}

#[cfg(test)]
mod secret_tests;
pub mod secrets;

#[cfg(test)]
mod tests;
