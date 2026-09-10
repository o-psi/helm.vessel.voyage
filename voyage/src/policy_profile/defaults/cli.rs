//! Operator defaults commands. Configuration publication is create-only.
use super::*;
use crate::Config;
use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
use uuid::Uuid;
const MAX_CONFIG: usize = 1024 * 1024;
#[derive(Debug, Args)]
pub struct DefaultsArgs {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Args)]
pub struct ScopeArgs {
    #[arg(long, conflicts_with = "project", required_unless_present = "project")]
    global: bool,
    /// Use this invocation's actual --workspace as a private project key.
    #[arg(long, conflicts_with = "global")]
    project: bool,
}
impl ScopeArgs {
    fn scope(&self, workspace: &Path) -> DefaultScope {
        if self.global {
            DefaultScope::Global {}
        } else {
            DefaultScope::Workspace {
                workspace: workspace.into(),
            }
        }
    }
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize private inert preferences; prints the explicit source anchor.
    Init {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        store_id: Option<Uuid>,
    },
    /// Copy source config verbatim to a new file with only the defaults anchor added.
    Enable {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        store_id: Uuid,
        #[arg(long)]
        output: PathBuf,
    },
    List {
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Inspect {
        #[command(flatten)]
        scope: ScopeArgs,
    },
    Set {
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        profile_directory: PathBuf,
        name: String,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        digest: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    Clear {
        #[command(flatten)]
        scope: ScopeArgs,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    Preview {},
    Activate {
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
        #[arg(long)]
        confirm: String,
    },
}
fn print(value: &impl Serialize) -> Result<()> {
    super::super::cli::print(value)
}
fn activation_arguments(
    config: &Path,
    workspace: &Path,
    explicit: &crate::policy_profile::Overrides,
    revision: u64,
    confirmation: &str,
    operation: Uuid,
) -> Result<Vec<String>> {
    let mut args = vec![
        "helm".into(),
        "--config".into(),
        config
            .to_str()
            .context("configuration path must be UTF-8")?
            .into(),
        "--workspace".into(),
        workspace
            .to_str()
            .context("workspace must be UTF-8")?
            .into(),
    ];
    let values = serde_json::to_value(explicit)?;
    for (field, key) in [
        ("access", "access"),
        ("unattended", "unattended_approval"),
        ("read_roots", "allow_read"),
        ("write_roots", "allow_write"),
        ("inherit_env", "inherit_env"),
        ("github_enabled", "github_enabled"),
    ] {
        if let Some(value) = values.get(field).filter(|v| !v.is_null()) {
            args.extend([
                "--set".into(),
                format!("{key}={}", serde_json::to_string(value)?),
            ]);
        }
    }
    args.extend([
        "policy".into(),
        "defaults".into(),
        "activate".into(),
        "--expected-revision".into(),
        revision.to_string(),
        "--confirm".into(),
        confirmation.into(),
        "--operation".into(),
        operation.to_string(),
    ]);
    Ok(args)
}
fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
/// Bounded administrative parsing; errors never carry TOML source excerpts.
pub fn load_config(explicit: Option<&Path>) -> Result<Config> {
    let default = crate::config::default_config_path();
    let path = explicit.or(default.as_deref().filter(|p| p.exists()));
    let bytes = match path {
        Some(path) => read_config(path)?,
        None => Vec::new(),
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow::anyhow!("configuration input is invalid"))?;
    Config::parse_loaded(text).map_err(|_| anyhow::anyhow!("configuration input is invalid"))
}
fn read_config(path: &Path) -> Result<Vec<u8>> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow::anyhow!("configuration input unavailable"))?;
    let metadata = file
        .metadata()
        .map_err(|_| anyhow::anyhow!("configuration input unavailable"))?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_CONFIG as u64,
        "configuration input must be a bounded regular file"
    );
    let mut bytes = Vec::new();
    file.take((MAX_CONFIG + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("configuration input unavailable"))?;
    ensure!(
        bytes.len() <= MAX_CONFIG,
        "configuration input exceeds limit"
    );
    Ok(bytes)
}
fn enable(source: Option<&Path>, output: &Path, anchor: &DefaultsSource) -> Result<()> {
    DefaultsStore::open_existing(anchor)?;
    let bytes = match source {
        Some(path) => read_config(path)?,
        None => Vec::new(),
    };
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| anyhow::anyhow!("configuration input is invalid"))?;
    let config: Config =
        toml::from_str(text).map_err(|_| anyhow::anyhow!("configuration input is invalid"))?;
    config
        .validate()
        .map_err(|_| anyhow::anyhow!("configuration input is invalid"))?;
    let mut intended = bytes.clone();
    if let Some(existing) = config.policy_defaults {
        ensure!(
            existing == *anchor,
            "configuration already selects another defaults source; choose an explicit new source configuration"
        );
    } else {
        #[derive(Serialize)]
        struct Anchor<'a> {
            policy_defaults: &'a DefaultsSource,
        }
        intended.extend_from_slice(b"\n");
        intended.extend_from_slice(
            toml::to_string(&Anchor {
                policy_defaults: anchor,
            })?
            .as_bytes(),
        );
    }
    ensure!(
        intended.len() <= MAX_CONFIG,
        "configuration plus defaults anchor exceeds limit"
    );
    // Validate appended table boundaries without ever formatting source diagnostics.
    let parsed: Config = toml::from_str(std::str::from_utf8(&intended).unwrap())
        .map_err(|_| anyhow::anyhow!("configuration anchor could not be appended safely"))?;
    ensure!(
        parsed.policy_defaults.as_ref() == Some(anchor),
        "configuration anchor is invalid"
    );
    match output.symlink_metadata() {
        Ok(_) => {
            ensure!(
                read_config(output)? == intended,
                "configuration output differs; choose a new output path"
            );
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => anyhow::bail!("configuration output unavailable"),
    }
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = output
        .file_name()
        .context("configuration output requires a filename")?;
    let directory = cap_std::fs::Dir::open_ambient_dir(parent, cap_std::ambient_authority())
        .map_err(|_| anyhow::anyhow!("configuration output unavailable"))?;
    crate::file_publication::Publication::prepare(directory, Path::new(name))?.publish(&intended)
}
pub(crate) fn run(
    args: DefaultsArgs,
    config: &Config,
    workspace: &Path,
    config_path: Option<&Path>,
) -> Result<()> {
    match args.command {
        Command::Init {
            directory,
            store_id,
        } => {
            let anchor = DefaultsSource {
                directory,
                store_id: store_id.unwrap_or_else(Uuid::new_v4),
            };
            DefaultsStore::create(&anchor)?;
            print(&anchor)
        }
        Command::Enable {
            directory,
            store_id,
            output,
        } => {
            let anchor = DefaultsSource {
                directory,
                store_id,
            };
            let default = crate::config::default_config_path();
            let source = if let Some(explicit) = config_path {
                Some(explicit)
            } else {
                default.as_deref().filter(|p| p.exists())
            };
            enable(source, &output, &anchor)?;
            print(
                &serde_json::json!({"output":output,"policy_defaults":anchor,"launch":"pass --config with this output path; source configuration was preserved"}),
            )
        }
        command => {
            let anchor = config.policy_defaults.as_ref().context(
                "policy defaults are not enabled; create an anchored configuration first",
            )?;
            let store = DefaultsStore::open_existing(anchor)?;
            match command {
                Command::List { after, limit } => print(&store.list(after.as_deref(), limit)?),
                Command::Inspect { scope } => print(&store.inspect(&DefaultKey::Preference {
                    scope: scope.scope(workspace),
                })?),
                Command::Set {
                    scope,
                    profile_directory,
                    name,
                    revision,
                    digest,
                    expected_revision,
                    operation,
                } => {
                    let snapshot = crate::policy_profile::store::ProfileStore::open_existing(
                        &profile_directory,
                    )?
                    .inspect(&name)?
                    .context("policy profile is missing")?;
                    ensure!(
                        snapshot.rules.is_some()
                            && snapshot.revision == revision
                            && snapshot.digest()? == digest,
                        "policy profile changed; inspect it again"
                    );
                    let profile = ProfileRef {
                        directory: profile_directory.canonicalize()?,
                        name,
                        revision,
                        digest,
                        snapshot,
                    };
                    print(&store.change(&DefaultsChange {
                        operation_id: operation.unwrap_or_else(Uuid::new_v4),
                        key: DefaultKey::Preference {
                            scope: scope.scope(workspace),
                        },
                        expected_revision,
                        value: DefaultValue::Preference {
                            profile: Some(profile),
                        },
                    })?)
                }
                Command::Clear {
                    scope,
                    expected_revision,
                    operation,
                } => print(&store.change(&DefaultsChange {
                    operation_id: operation.unwrap_or_else(Uuid::new_v4),
                    key: DefaultKey::Preference {
                        scope: scope.scope(workspace),
                    },
                    expected_revision,
                    value: DefaultValue::Preference { profile: None },
                })?),
                Command::Preview {} => {
                    let preview = super::preview(config, workspace)?;
                    let activation = store.inspect(&DefaultKey::Activation {
                        workspace: workspace.into(),
                    })?;
                    let revision = activation.map_or(0, |s| s.revision);
                    let default = crate::config::default_config_path();
                    let path = config_path
                        .or(default.as_deref())
                        .context("configuration path required")?
                        .canonicalize()?;
                    let operation = Uuid::new_v4();
                    let arguments = activation_arguments(
                        &path,
                        workspace,
                        &config.policy_explicit,
                        revision,
                        &preview.transition_digest,
                        operation,
                    )?;
                    let command = arguments
                        .iter()
                        .map(|s| shell_word(s))
                        .collect::<Vec<_>>()
                        .join(" ");
                    print(
                        &serde_json::json!({"preview":preview,"activation_expected_revision":revision,"activation_operation":operation,"activation_arguments":arguments,"activation_command":command}),
                    )
                }
                Command::Activate {
                    expected_revision,
                    operation,
                    confirm,
                } => {
                    let receipt = super::activate(
                        config,
                        workspace,
                        operation.unwrap_or_else(Uuid::new_v4),
                        expected_revision,
                        &confirm,
                    )?;
                    let current =
                        super::preview(config, workspace).is_ok_and(|p| p.activation_current);
                    print(&serde_json::json!({"receipt":receipt,"activation_current":current}))
                }
                Command::Init { .. } | Command::Enable { .. } => unreachable!(),
            }
        }
    }
}
