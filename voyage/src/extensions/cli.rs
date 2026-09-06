use super::{
    Archive,
    catalog::{Catalog, Scope},
    store,
};
use anyhow::Result;
use clap::{Args, Subcommand};
use std::{io::Write, path::PathBuf};
#[derive(Args)]
pub struct ExtensionArgs {
    #[arg(long, value_enum, default_value = "user", global = true)]
    pub scope: Scope,
    #[command(subcommand)]
    pub command: ExtensionCommand,
}
#[derive(Subcommand)]
pub enum ExtensionCommand {
    /// Validate a directory and write a new bounded .helmpkg archive.
    Pack {
        directory: PathBuf,
        output: PathBuf,
    },
    /// Install a local archive or directory, inactive until explicitly enabled.
    Install {
        source: PathBuf,
    },
    /// Replace exact installed bytes; activation is always cleared first.
    Update {
        id: String,
        source: PathBuf,
        #[arg(long)]
        expected: String,
    },
    /// Fetch from a configured, digest-pinned HTTPS index; installation is inactive.
    Fetch {
        index: PathBuf,
        id: String,
        #[arg(long)]
        expected: Option<String>,
    },
    List,
    /// List current activation bindings, including externally removed packages.
    Grants,
    /// Revoke an exact binding without needing the original package/workspace.
    RevokeGrant {
        binding: String,
        #[arg(long)]
        expected: String,
    },
    Inspect {
        id: String,
    },
    /// Print a JSON string containing one exact UTF-8 resource (no terminal controls).
    Resource {
        id: String,
        path: String,
    },
    /// Authorize these exact bytes as untrusted model context on the next run.
    Enable {
        id: String,
        #[arg(long)]
        expected: String,
    },
    Disable {
        id: String,
        #[arg(long)]
        expected: String,
    },
    Remove {
        id: String,
        #[arg(long)]
        expected: String,
    },
}
fn source(path: &std::path::Path) -> Result<Vec<u8>> {
    if path.is_dir() {
        store::pack(path)
    } else {
        let bytes = store::local(path)?;
        Archive::parse(&bytes)?;
        Ok(bytes)
    }
}
pub async fn run(args: ExtensionArgs, workspace: Option<PathBuf>) -> Result<()> {
    let root = workspace.unwrap_or(std::env::current_dir()?);
    let catalog = Catalog::new(&root, &crate::config::default_data_dir())?;
    match args.command {
        ExtensionCommand::Pack { directory, output } => {
            let bytes = store::pack(&directory)?;
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(output)?;
            f.write_all(&bytes)?;
            f.sync_all()?;
            println!("{}", super::digest(&bytes));
        }
        ExtensionCommand::Install { source: path } => {
            let bytes = source(&path)?;
            let archive = Archive::parse(&bytes)?;
            catalog.mutate(args.scope, &archive.manifest.id, None, Some(&bytes), None)?;
            println!("{}", super::digest(&bytes));
        }
        ExtensionCommand::Update {
            id,
            source: path,
            expected,
        } => {
            let bytes = source(&path)?;
            catalog.mutate(args.scope, &id, Some(&expected), Some(&bytes), None)?;
            println!("{}", super::digest(&bytes));
        }
        ExtensionCommand::Fetch {
            index,
            id,
            expected,
        } => {
            let bytes = super::index::acquire(&index, &id).await?;
            catalog.mutate(args.scope, &id, expected.as_deref(), Some(&bytes), None)?;
            println!("{}", super::digest(&bytes));
        }
        ExtensionCommand::Grants => println!(
            "{}",
            serde_json::to_string_pretty(&catalog.activation_records()?)?
        ),
        ExtensionCommand::RevokeGrant { binding, expected } => {
            catalog.revoke_grant(&binding, &expected)?
        }
        ExtensionCommand::List => println!("{}", serde_json::to_string_pretty(&catalog.list()?)?),
        ExtensionCommand::Inspect { id } => println!(
            "{}",
            serde_json::to_string_pretty(&catalog.inspect(args.scope, &id)?)?
        ),
        ExtensionCommand::Resource { id, path } => println!(
            "{}",
            serde_json::to_string(&catalog.resource(args.scope, &id, &path)?)?
        ),
        ExtensionCommand::Enable { id, expected } => {
            catalog.mutate(args.scope, &id, Some(&expected), None, Some(true))?
        }
        ExtensionCommand::Disable { id, expected } => {
            catalog.mutate(args.scope, &id, Some(&expected), None, Some(false))?
        }
        ExtensionCommand::Remove { id, expected } => {
            catalog.mutate(args.scope, &id, Some(&expected), None, None)?
        }
    }
    Ok(())
}
