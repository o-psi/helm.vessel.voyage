//! Human-selected private profile administration and explicit launch flags.
use super::{
    Builtin, Overrides, ProfileDocument,
    selection::{Selection, SelectionRequest},
    store::{Action, ProfileChange, ProfileStore},
};
use crate::Config;
use anyhow::{Context, Result, ensure};
use clap::{Args, Subcommand};
use serde::Serialize;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;
#[derive(Clone, Debug, Default, Args)]
pub struct SelectionArgs {
    /// Private named-profile directory (default: Helm config directory/profiles).
    #[arg(long, global = true)]
    pub policy_directory: Option<PathBuf>,
    /// Explicit launch profile; a saved session never selects one automatically.
    #[arg(long,global=true,requires_all=["policy_revision","policy_digest"])]
    pub policy_profile: Option<String>,
    #[arg(long, global = true, requires = "policy_profile")]
    pub policy_revision: Option<u64>,
    #[arg(long, global = true, requires = "policy_profile")]
    pub policy_digest: Option<String>,
    /// Exact fresh preview digest for a more permissive launch.
    #[arg(long, global = true, requires = "policy_profile")]
    pub policy_confirm: Option<String>,
}
#[derive(Debug, Args)]
pub struct PolicyArgs {
    #[command(subcommand)]
    pub command: PolicyCommand,
}
#[derive(Debug, Subcommand)]
pub enum PolicyCommand {
    /// Persistent operator-private global/workspace preferences.
    Defaults(super::defaults::cli::DefaultsArgs),
    /// List current profiles and deletion tombstones, with bounded pagination.
    List {
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    Inspect {
        name: String,
    },
    /// Create from a shipped immutable preset; this does not activate it.
    Create {
        name: String,
        #[arg(long, default_value = "balanced")]
        preset: String,
        #[arg(long, default_value_t = 0)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    /// Replace the current rules with a strict exported document.
    Edit {
        name: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    Duplicate {
        source: String,
        name: String,
        #[arg(long, default_value_t = 0)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    Delete {
        name: String,
        #[arg(long)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    /// Import rules as an inert new profile; source revision is not authority.
    Import {
        name: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long, default_value_t = 0)]
        expected_revision: u64,
        #[arg(long)]
        operation: Option<Uuid>,
    },
    /// Export names, revision and rules to stdout; never Config or secrets.
    Export {
        name: String,
    },
    /// Resolve exact selected rules against this invocation's Config and ceiling.
    Preview {
        name: String,
        #[arg(long)]
        revision: u64,
        #[arg(long)]
        digest: String,
    },
}
impl PolicyArgs {
    pub fn needs_config(&self) -> bool {
        matches!(
            self.command,
            PolicyCommand::Preview { .. } | PolicyCommand::Defaults(_)
        )
    }
}
fn directory(flags: &SelectionArgs) -> Result<PathBuf> {
    let path = flags
        .policy_directory
        .clone()
        .or_else(|| {
            crate::config::default_config_path()
                .and_then(|p| p.parent().map(|p| p.join("profiles")))
        })
        .context("policy directory must be specified")?;
    ensure!(path.is_absolute(), "policy directory must be absolute");
    Ok(path)
}
// Provision only the default administration path, one pinned no-follow component
// at a time. Selection freshness never calls this initialization helper.
#[cfg(unix)]
pub(super) fn initialize_default_parent(path: &Path) -> Result<()> {
    use cap_fs_ext::DirExt;
    use cap_std::fs::{Dir, DirBuilder, DirBuilderExt};
    use std::path::Component;
    ensure!(
        path.is_absolute() && path.components().count() <= 128,
        "invalid default policy directory"
    );
    let mut current = Dir::open_ambient_dir("/", cap_std::ambient_authority())?;
    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    for component in path.components() {
        let Component::Normal(name) = component else {
            ensure!(
                component == Component::RootDir,
                "invalid default policy ancestor"
            );
            continue;
        };
        current = match current.open_dir_nofollow(name) {
            Ok(next) => next,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match current.create_dir_with(name, &builder) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                    Err(error) => return Err(error.into()),
                }
                current.open_dir_nofollow(name)?
            }
            Err(error) => return Err(error.into()),
        };
    }
    Ok(())
}
/// CLI override fields are copied from the already-validated actual Config.
/// Environment values are intentionally absent from both selection and provenance.
pub fn explicit(config: &Config, assignments: &[String], access: bool) -> Result<Overrides> {
    let keys: Vec<_> = assignments
        .iter()
        .filter_map(|a| a.split_once('=').map(|(k, _)| k.trim()))
        .collect();
    let base = crate::runtime_policy::config_rules(config)?;
    Ok(Overrides {
        access: (access || keys.contains(&"access") || keys.contains(&"approval"))
            .then_some(base.access),
        unattended: keys
            .contains(&"unattended_approval")
            .then_some(base.unattended),
        read_roots: keys.contains(&"allow_read").then_some(base.read_roots),
        write_roots: keys.contains(&"allow_write").then_some(base.write_roots),
        deny_commands: keys
            .contains(&"deny_commands")
            .then_some(base.deny_commands),
        inherit_env: keys.contains(&"inherit_env").then_some(base.inherit_env),
    })
}
impl SelectionArgs {
    pub fn apply(&self, config: &mut Config, workspace: &Path, explicit: Overrides) -> Result<()> {
        if let Some(name) = &self.policy_profile {
            config.policy_profile = Some(Selection::bind(
                config,
                workspace,
                SelectionRequest {
                    directory: directory(self)?,
                    name: name.clone(),
                    revision: self.policy_revision.context("policy revision required")?,
                    digest: self
                        .policy_digest
                        .clone()
                        .context("policy digest required")?,
                    explicit,
                },
                self.policy_confirm.as_deref(),
            )?);
        }
        Ok(())
    }
}
pub(super) fn print(value: &impl Serialize) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    writeln!(out)?;
    Ok(())
}
fn input(path: &Path) -> Result<ProfileDocument> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow::anyhow!("policy input unavailable"))?;
    ensure!(
        file.metadata()?.is_file(),
        "policy input must be a regular file"
    );
    let mut bytes = Vec::new();
    file.take((super::MAX_DOCUMENT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("policy input unavailable"))?;
    Ok(ProfileDocument::decode(&bytes)?)
}
fn shell_word(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn run(
    args: PolicyArgs,
    flags: &SelectionArgs,
    config: Option<&Config>,
    workspace: Option<&Path>,
    explicit: Overrides,
    config_path: Option<&Path>,
) -> Result<()> {
    ensure!(
        flags.policy_profile.is_none(),
        "policy administration cannot also select a launch profile"
    );
    if let PolicyCommand::Defaults(args) = args.command {
        return super::defaults::cli::run(
            args,
            config.context("configuration required")?,
            workspace.context("workspace required")?,
            config_path,
        );
    }
    let path = directory(flags)?;
    #[cfg(unix)]
    if flags.policy_directory.is_none() {
        initialize_default_parent(
            path.parent()
                .context("policy directory requires a parent")?,
        )?;
    }
    let store = ProfileStore::open(&path)?;
    let (name, expected_revision, operation, action) = match args.command {
        PolicyCommand::Defaults(_) => unreachable!(),
        PolicyCommand::List { after, limit } => {
            return print(&store.list(after.as_deref(), limit)?);
        }
        PolicyCommand::Inspect { name } => {
            let snapshot = store.inspect(&name)?.context("policy profile is missing")?;
            return print(&serde_json::json!({"digest":snapshot.digest()?,"profile":snapshot}));
        }
        PolicyCommand::Export { name } => {
            std::io::stdout().write_all(&store.export(&name)?)?;
            return Ok(());
        }
        PolicyCommand::Preview {
            name,
            revision,
            digest,
        } => {
            let config = config.context("policy preview requires configuration")?;
            let workspace = workspace.context("policy preview requires workspace")?;
            let request = SelectionRequest {
                directory: path.clone(),
                name: name.clone(),
                revision,
                digest: digest.clone(),
                explicit,
            };
            let preview = Selection::preview(config, workspace, &request)?;
            let mut flags = format!(
                "--policy-directory {} --policy-profile {} --policy-revision {} --policy-digest {}",
                shell_word(path.to_str().context("policy directory must be UTF-8")?),
                shell_word(&name),
                revision,
                digest
            );
            if preview.requires_confirmation {
                flags.push_str(&format!(" --policy-confirm {}", preview.transition_digest));
            }
            return print(
                &serde_json::json!({"preview":preview,"selection_flags":flags,"instruction":"Repeat the same --config, --set, --access and --workspace options used for this preview, add selection_flags, then run/chat/workflow run/managed submit. Any changed transition requires a fresh preview."}),
            );
        }
        PolicyCommand::Create {
            name,
            preset,
            expected_revision,
            operation,
        } => {
            let preset = match preset.as_str() {
                "restricted" => Builtin::Restricted,
                "balanced" => Builtin::Balanced,
                "autonomous" => Builtin::Autonomous,
                _ => anyhow::bail!("unknown built-in policy preset"),
            };
            (
                name,
                expected_revision,
                operation,
                Action::Create {
                    rules: preset.document().rules,
                },
            )
        }
        PolicyCommand::Edit {
            name,
            input: path,
            expected_revision,
            operation,
        } => {
            let doc = input(&path)?;
            ensure!(
                doc.name == name,
                "edited policy document belongs to another name"
            );
            (
                name,
                expected_revision,
                operation,
                Action::Replace { rules: doc.rules },
            )
        }
        PolicyCommand::Import {
            name,
            input: path,
            expected_revision,
            operation,
        } => {
            let doc = input(&path)?;
            (
                name,
                expected_revision,
                operation,
                Action::Create { rules: doc.rules },
            )
        }
        PolicyCommand::Duplicate {
            source,
            name,
            expected_revision,
            operation,
        } => {
            let rules = store
                .inspect(&source)?
                .context("source policy profile is missing")?
                .document()?
                .rules;
            (name, expected_revision, operation, Action::Create { rules })
        }
        PolicyCommand::Delete {
            name,
            expected_revision,
            operation,
        } => (name, expected_revision, operation, Action::Delete {}),
    };
    let operation_id = operation.unwrap_or_else(Uuid::new_v4);
    // A caller can retry an uncertain operation with its original ID and payload.
    eprintln!("policy operation {operation_id}");
    print(&store.change(&ProfileChange {
        operation_id,
        name,
        expected_revision,
        action,
    })?)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn default_initialization_uses_private_modes_and_never_follows_ancestor_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("new/config/helm");
        initialize_default_parent(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        symlink(&outside, temp.path().join("linked")).unwrap();
        assert!(initialize_default_parent(&temp.path().join("linked/escape")).is_err());
        assert!(!outside.join("escape").exists());
        assert!(initialize_default_parent(&temp.path().join("new/../escape")).is_err());
    }
}
