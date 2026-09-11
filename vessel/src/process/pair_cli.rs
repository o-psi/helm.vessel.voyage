//! Local account-owner connection administration. No secret is printed or sent
//! through a model-visible command envelope; invitation output is private.
use super::{access::store, pairing, registry};
use anyhow::{Result, ensure};
use clap::Args;
use std::{
    fs::{File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};
use uuid::Uuid;
use voyage_protocol::process::{ApprovedWorkspace, ProcessRight};

#[derive(Args)]
pub struct PairInviteArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub endpoint: String,
    /// Intended Helm principal UUID; the code alone cannot change this binding.
    #[arg(long)]
    pub principal: Uuid,
    /// Owner-approved canonical directory. Repeat for each permitted workspace.
    #[arg(long, required = true)]
    pub workspace: Vec<PathBuf>,
    /// New private file (never stdout). Its parent must be owner-private.
    #[arg(long)]
    pub output: PathBuf,
    /// Explicit scope; destructive actions and terminals are not granted by default.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "catalogue,create,observe,history,execute"
    )]
    pub rights: Vec<String>,
    #[arg(long, value_delimiter = ',')]
    pub accounts: Vec<Uuid>,
    #[arg(long, value_delimiter = ',')]
    pub enrollment_connections: Vec<Uuid>,
    #[arg(long, default_value_t = 600)]
    pub ttl_seconds: u64,
}
#[derive(Args)]
pub struct RevokeConnectionArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub grant: Uuid,
    #[arg(long)]
    pub expected_revision: u64,
    /// Retain and reuse this UUID for retries of this exact revocation.
    #[arg(long)]
    pub command_id: Uuid,
}

#[derive(Args)]
pub struct ListConnectionsArgs {
    /// Existing private state owned by the executing account; never initialized.
    #[arg(long)]
    pub directory: PathBuf,
}

pub fn list(args: ListConnectionsArgs) -> Result<()> {
    let result = pairing::inventory(&args.directory)?;
    let output = serde_json::to_string(&result)?;
    ensure!(
        output.len() <= 16 * 1024 * 1024,
        "connection inventory exceeds limit"
    );
    println!("{output}");
    Ok(())
}

pub fn invite(args: PairInviteArgs) -> Result<()> {
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    registry::private_directory(parent)?;
    // Different CLI processes must not issue competing invitations for one output.
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(args.output.with_extension("pair-lock"))?;
    let m = lock.metadata()?;
    ensure!(
        m.is_file()
            && m.nlink() == 1
            && m.uid() == unsafe { libc::geteuid() }
            && m.mode() & 0o077 == 0,
        "unsafe invitation output lock"
    );
    lock.try_lock()
        .map_err(|_| anyhow::anyhow!("invitation output is busy"))?;
    ensure!(
        std::fs::symlink_metadata(&args.output)
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "invitation output already exists or is inaccessible"
    );
    let rights = args
        .rights
        .iter()
        .map(|r| serde_json::from_value::<ProcessRight>(serde_json::Value::String(r.clone())))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let workspaces = args
        .workspace
        .iter()
        .map(|path| {
            let path = std::fs::canonicalize(path)?;
            ensure!(path.is_dir(), "workspace must be a directory");
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Workspace".into());
            Ok(ApprovedWorkspace {
                id: Uuid::new_v4(),
                name,
                path,
                // Configuration presence is not proof of provider readiness.
                provider_ready: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let invitation = pairing::invite(
        &args.directory,
        &args.endpoint,
        args.principal,
        workspaces,
        rights,
        args.accounts,
        args.enrollment_connections,
        args.ttl_seconds,
    )?;
    // Publish complete bytes without ever replacing a concurrently created output.
    // If interrupted before publication, the orphaned short-lived invitation expires;
    // no grant has been minted and running this command again cannot broaden it.
    let temporary = parent.join(format!(".pair-output-{}", Uuid::new_v4()));
    store::save(&temporary, &invitation)?;
    let published = std::fs::hard_link(&temporary, &args.output);
    let removed = std::fs::remove_file(&temporary);
    File::open(parent)?.sync_all()?;
    published?;
    removed?;
    println!(
        "Private pairing invitation written to {}",
        args.output.display()
    );
    Ok(())
}

pub fn revoke(args: RevokeConnectionArgs) -> Result<()> {
    let result = pairing::revoke(
        &args.directory,
        args.grant,
        args.expected_revision,
        args.command_id,
    )?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}

#[derive(Args)]
pub struct ConnectionAuditArgs {
    /// Existing account-private Vessel directory. Never creates or repairs state.
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long, default_value_t = 64)]
    pub limit: usize,
    /// Exact next_cursor returned by the preceding page; retain the same limit.
    #[arg(long)]
    pub cursor: Option<String>,
}
pub fn audit(args: ConnectionAuditArgs) -> Result<()> {
    let value = pairing::connection_audit(&args.directory, args.limit, args.cursor.as_deref())?;
    let output = serde_json::to_string(&value)?;
    ensure!(
        output.len() <= 512 * 1024,
        "connection audit output exceeds limit"
    );
    println!("{output}");
    Ok(())
}
