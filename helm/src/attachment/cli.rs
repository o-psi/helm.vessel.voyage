//! Explicit enrollment administration, separate from provider login and execution.
pub mod prompt;
use super::client::{ClientError, EnrollmentClient, Inspection, Status, validate_origin};
use clap::{Args, Subcommand};
use std::{
    io::{self, IsTerminal, Read},
    path::PathBuf,
    time::Duration,
};
use uuid::Uuid;
#[derive(Args)]
pub struct AttachmentArgs {
    /// Dedicated enrollment directory; defaults to Helm's data directory/attachment.
    #[arg(long, global = true)]
    pub directory: Option<PathBuf>,
    /// Vessel HTTPS origin. Required for new enrollment; existing origins cannot change.
    #[arg(long, global = true)]
    pub origin: Option<String>,
    /// Permit HTTP only for a literal loopback address (development).
    #[arg(long, global = true)]
    pub allow_insecure_loopback: bool,
    #[command(subcommand)]
    pub command: AttachmentCommand,
}
#[derive(Subcommand)]
pub enum AttachmentCommand {
    /// Enroll a new identity; no worker or remote execution is started.
    Enroll {
        #[arg(long)]
        invitation_id: Uuid,
        /// Read the invitation key from a pipe/file instead of the hidden terminal prompt.
        #[arg(long)]
        invitation_key_stdin: bool,
    },
    /// Inspect local status without creating or changing enrollment.
    Status,
    /// Resume the original pending transaction (enrollment requires its invitation key).
    Resume {
        /// Use a pipe/file instead of the hidden prompt for a pending enrollment.
        #[arg(long)]
        invitation_key_stdin: bool,
    },
    /// Rotate the enrolled key using a durable recoverable transaction.
    Rotate,
    /// Request confirmed server revocation; failures retain the pending transaction.
    Revoke,
    /// Disable locally offline; this does not request or confirm server revocation.
    Detach,
}
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("invalid attachment arguments; use helm attachment --help")]
    Arguments,
    #[error(
        "invitation key requires a terminal or explicit non-terminal stdin: exactly 43 base64url characters"
    )]
    Input,
    #[error("attachment input cancelled or timed out")]
    Cancelled,
    #[error(transparent)]
    Client(#[from] ClientError),
}
struct Secret(Vec<u8>);
impl Drop for Secret {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}
impl Secret {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).expect("validated ASCII invitation")
    }
}
fn parse_secret(mut bytes: Vec<u8>) -> Result<Secret, CliError> {
    if bytes.ends_with(b"\r\n") {
        bytes.truncate(bytes.len() - 2);
    } else if bytes.ends_with(b"\n") {
        bytes.pop();
    }
    if bytes.len() != 43
        || !bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
    {
        bytes.fill(0);
        return Err(CliError::Input);
    }
    Ok(Secret(bytes))
}
async fn invitation() -> Result<Secret, CliError> {
    if io::stdin().is_terminal() {
        return Err(CliError::Input);
    }
    let task = tokio::task::spawn_blocking(|| {
        let mut bytes = Vec::new();
        io::stdin()
            .take(46)
            .read_to_end(&mut bytes)
            .map_err(|_| CliError::Input)?;
        parse_secret(bytes)
    });
    match tokio::time::timeout(Duration::from_secs(30), task).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(CliError::Input),
        // A pipe read may be blocked in an OS thread. Exit the dedicated CLI
        // process rather than hanging runtime shutdown waiting for its producer.
        Err(_) => {
            eprintln!("attachment input timed out");
            std::process::exit(1)
        }
    }
}
fn print_status(info: Option<Inspection>, detached: bool) -> Result<(), CliError> {
    let mut value = match info {
        Some(info) => serde_json::to_value(info).map_err(|_| CliError::Arguments)?,
        None => serde_json::json!({"status":"unenrolled"}),
    };
    if detached {
        value["notice"] = if value["status"] == "revoked" {
            serde_json::json!("server revocation already confirmed; local state unchanged")
        } else {
            serde_json::json!("disabled locally; server revocation is not confirmed by detach")
        };
    }
    println!(
        "{}",
        serde_json::to_string(&value).map_err(|_| CliError::Arguments)?
    );
    Ok(())
}
pub async fn run(args: AttachmentArgs) -> Result<(), CliError> {
    run_with_prompt(args, prompt::PromptControl::default()).await
}
pub async fn run_with_prompt(
    args: AttachmentArgs,
    control: prompt::PromptControl,
) -> Result<(), CliError> {
    let explicit_directory = args.directory.is_some();
    let directory = args
        .directory
        .unwrap_or_else(|| crate::config::default_data_dir().join("attachment"));
    let existing = EnrollmentClient::inspect(&directory)?;
    let requested = args
        .origin
        .as_deref()
        .map(|origin| validate_origin(origin, args.allow_insecure_loopback))
        .transpose()?;
    if let (Some(info), Some(origin)) = (&existing, &requested)
        && &info.origin != origin
    {
        return Err(ClientError::Conflict.into());
    }
    if matches!(args.command, AttachmentCommand::Status) {
        return print_status(existing, false);
    }
    if let AttachmentCommand::Enroll {
        invitation_id,
        invitation_key_stdin,
    } = args.command
    {
        let origin = requested.ok_or(CliError::Arguments)?;
        if invitation_id.is_nil()
            || existing
                .as_ref()
                .is_some_and(|i| i.status != Status::Unenrolled)
        {
            return Err(ClientError::Conflict.into());
        }
        let secret = if invitation_key_stdin {
            invitation().await?
        } else {
            prompt::read(control.clone()).await?
        };
        if !explicit_directory {
            let parent = directory.parent().ok_or(CliError::Arguments)?;
            crate::session::reject_symlinks(parent).map_err(|_| ClientError::Storage)?;
            std::fs::create_dir_all(parent).map_err(|_| ClientError::Storage)?;
        }
        let mut client = if existing.is_some() {
            EnrollmentClient::open_existing(&directory, &origin, args.allow_insecure_loopback)?
        } else {
            EnrollmentClient::open(&directory, &origin, args.allow_insecure_loopback)?
        };
        client.enroll(invitation_id, secret.as_str()).await?;
        return print_status(Some(client.inspection()), false);
    }
    let info = existing.ok_or(ClientError::Conflict)?;
    let detached = matches!(args.command, AttachmentCommand::Detach);
    // Offline detach may open an already-bound loopback identity without granting
    // permission for an HTTP request. No network method is called on this path.
    let mut client = EnrollmentClient::open_existing(
        &directory,
        &info.origin,
        args.allow_insecure_loopback || detached,
    )?;
    match args.command {
        AttachmentCommand::Resume {
            invitation_key_stdin,
        } => {
            if info.status != Status::Enrolling && invitation_key_stdin {
                return Err(CliError::Arguments);
            }
            let secret = if info.status == Status::Enrolling {
                Some(if invitation_key_stdin {
                    invitation().await?
                } else {
                    prompt::read(control.clone()).await?
                })
            } else {
                None
            };
            client.resume(secret.as_ref().map(Secret::as_str)).await?;
        }
        AttachmentCommand::Rotate => {
            client.rotate().await?;
        }
        AttachmentCommand::Revoke => {
            client.revoke().await?;
        }
        AttachmentCommand::Detach => client.detach()?,
        _ => return Err(CliError::Arguments),
    }
    print_status(Some(client.inspection()), detached)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_secret_grammar() {
        for ending in ["", "\n", "\r\n"] {
            assert!(parse_secret(format!("{}{ending}", "A".repeat(43)).into_bytes()).is_ok());
        }
        for input in [
            "A".repeat(42),
            "A".repeat(44),
            format!("{} ", "A".repeat(42)),
            format!("{}\0", "A".repeat(42)),
            format!("{}\n\n", "A".repeat(43)),
            "🦀".repeat(43),
        ] {
            assert!(parse_secret(input.into_bytes()).is_err());
        }
    }
}
