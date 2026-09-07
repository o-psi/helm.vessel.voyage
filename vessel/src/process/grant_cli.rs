//! Local account-owner grant administration; secrets go only to private files.
use anyhow::{Result, ensure};
use clap::Args;
use std::{
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::PathBuf,
};
use uuid::Uuid;
use voyage_protocol::process::*;

#[derive(Args)]
pub struct GrantArgs {
    #[arg(long)]
    pub directory: PathBuf,
    #[arg(long)]
    pub output: PathBuf,
    /// Resume the immutable pending request retained alongside the output file.
    #[arg(long)]
    pub resume: bool,
    #[arg(long, required_unless_present = "resume")]
    pub session: Option<Uuid>,
    #[arg(long, required_unless_present = "resume")]
    pub principal: Option<Uuid>,
    #[arg(long, required_unless_present = "resume")]
    pub workspace: Option<PathBuf>,
    #[arg(long, required_unless_present = "resume")]
    pub endpoint: Option<String>,
    #[arg(long, value_delimiter = ',', default_value = "observe")]
    pub rights: Vec<String>,
    #[arg(long, default_value_t = 86400)]
    pub ttl_seconds: u64,
    #[arg(long,requires_all=["machine","machine_epoch"])]
    pub enrollment_database: Option<PathBuf>,
    #[arg(long,requires_all=["enrollment_database","machine_epoch"])]
    pub machine: Option<Uuid>,
    #[arg(long,requires_all=["enrollment_database","machine"])]
    pub machine_epoch: Option<u64>,
}

pub async fn issue(args: GrantArgs) -> Result<()> {
    let parent = args
        .output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("credential parent missing"))?;
    super::registry::private_directory(parent)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(args.output.with_extension("grant-lock"))?;
    let metadata = lock.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0,
        "unsafe grant operation lock"
    );
    lock.try_lock()
        .map_err(|_| anyhow::anyhow!("credential issuance already running"))?;
    let pending = args.output.with_extension("pending-grant.json");
    let request: VesselRequest = if args.resume {
        super::access::store::load(&pending)?
    } else {
        ensure!(!args.output.exists(), "credential output already exists");
        ensure!(
            !pending.exists(),
            "pending grant exists; use --resume with the same output path"
        );
        ensure!(
            args.ttl_seconds > 0 && args.ttl_seconds <= 30 * 86400,
            "grant lifetime must be within 30 days"
        );
        let rights = args
            .rights
            .iter()
            .map(|right| serde_json::from_value(serde_json::Value::String(right.clone())))
            .collect::<std::result::Result<Vec<ProcessRight>, _>>()?;
        let enrollment = match (args.enrollment_database, args.machine, args.machine_epoch) {
            (Some(database_path), Some(machine_id), Some(epoch)) => Some(EnrollmentIdentity {
                database_path: std::fs::canonicalize(database_path)?,
                machine_id,
                epoch,
            }),
            _ => None,
        };
        let request = VesselRequest {
            protocol: voyage_protocol::vessel::VESSEL_API_VERSION,
            command: VesselCommand::Grant {
                command_id: Uuid::new_v4(),
                grant_id: Uuid::new_v4(),
                principal_id: args
                    .principal
                    .ok_or_else(|| anyhow::anyhow!("principal required"))?,
                session_id: args
                    .session
                    .ok_or_else(|| anyhow::anyhow!("session required"))?,
                workspace: std::fs::canonicalize(
                    args.workspace
                        .ok_or_else(|| anyhow::anyhow!("workspace required"))?,
                )?,
                rights,
                expires_at_ms: super::access::store::now()?
                    .checked_add(args.ttl_seconds * 1000)
                    .ok_or_else(|| anyhow::anyhow!("expiry overflow"))?,
                enrollment,
                endpoint: args
                    .endpoint
                    .ok_or_else(|| anyhow::anyhow!("endpoint required"))?,
            },
        };
        super::access::store::save(&pending, &request)?;
        request
    };
    let response = super::exchange::exchange(&args.directory, &request).await?;
    ensure!(
        response.error.is_none(),
        "grant request failed: {}; resume with --resume and the same output path",
        response.error.unwrap_or_default()
    );
    let credential: AccessCredential = serde_json::from_value(response.result)?;
    if args.output.exists() {
        let prior: AccessCredential = super::access::store::load(&args.output)?;
        ensure!(
            serde_json::to_vec(&prior)? == serde_json::to_vec(&credential)?,
            "credential output conflicts with retained grant"
        );
    } else {
        super::access::store::save(&args.output, &credential)?;
    }
    std::fs::remove_file(pending)?;
    std::fs::File::open(parent)?.sync_all()?;
    println!(
        "Private process credential written to {}",
        args.output.display()
    );
    Ok(())
}

pub async fn revoke(
    directory: PathBuf,
    grant_id: Uuid,
    expected_revision: u64,
    command_id: Uuid,
) -> Result<()> {
    let response = super::exchange::exchange(
        &directory,
        &VesselRequest {
            protocol: voyage_protocol::vessel::VESSEL_API_VERSION,
            command: VesselCommand::RevokeGrant {
                command_id,
                grant_id,
                expected_revision,
            },
        },
    )
    .await?;
    ensure!(
        response.error.is_none(),
        "grant revocation failed: {}",
        response.error.unwrap_or_default()
    );
    println!("{}", serde_json::to_string(&response.result)?);
    Ok(())
}
