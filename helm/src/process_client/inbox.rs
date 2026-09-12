//! Explicit metadata-only notification operations; no decision response or navigation.
use super::transport::Client;
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use serde_json::Value;
use std::{io::Read, path::PathBuf, time::Duration};
use uuid::Uuid;
use voyage_protocol::{
    notifications::{Destination, NotificationOperation, ReceiptState},
    vessel::VesselCommand,
};

#[derive(Subcommand)]
pub enum InboxCommand {
    /// Configure an immutable destination from a typed JSON file (no message content).
    Configure {
        file: PathBuf,
        #[arg(long)]
        command_id: Uuid,
    },
    /// Accept storage/subscription as the configured recipient (separate from source consent).
    Accept {
        destination: Uuid,
        #[arg(long)]
        command_id: Uuid,
    },
    /// List notification destinations visible to this authenticated connection.
    Destinations,
    /// Revoke a destination. Its identity cannot be reused.
    Revoke {
        destination: Uuid,
        #[arg(long)]
        command_id: Uuid,
    },
    /// Publish a synthetic test, never a fabricated successful run.
    Test {
        destination: Uuid,
        #[arg(long)]
        command_id: Uuid,
    },
    /// Read a bounded page. Reading alone does not mark notifications seen.
    List {
        destination: Uuid,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Poll read-only pages for a bounded duration; any error stops the watch.
    Watch {
        destination: Uuid,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u64).range(1..=3600))]
        seconds: u64,
    },
    /// Fetch fresh owner metadata for a notification, without opening or responding.
    #[command(alias = "open")]
    Inspect { destination: Uuid, event: Uuid },
    /// Record seen state for this destination only.
    Seen { destination: Uuid, event: Uuid },
    /// Dismiss locally; does not resolve the owner's request or cancel its run.
    Dismiss { destination: Uuid, event: Uuid },
}

fn load_destination(path: &std::path::Path) -> Result<Destination> {
    let file = std::fs::File::open(path).context("cannot open destination file")?;
    ensure!(
        file.metadata()?.is_file(),
        "destination must be a regular JSON file"
    );
    let mut bytes = Vec::new();
    file.take(65_537).read_to_end(&mut bytes)?;
    decode_destination(&bytes)
}

fn decode_destination(bytes: &[u8]) -> Result<Destination> {
    ensure!(bytes.len() <= 65_536, "destination JSON exceeds 64 KiB");
    // Never echo the supplied document, values, or serde diagnostic (which may quote input).
    serde_json::from_slice(bytes).map_err(|_| anyhow::anyhow!("invalid typed destination JSON"))
}

pub(super) async fn request(client: &Client, operation: NotificationOperation) -> Result<Value> {
    tokio::time::timeout(
        Duration::from_secs(10),
        client.request(VesselCommand::Notifications { operation }),
    )
    .await
    .context("notification request timed out; mutation outcome may be unknown; do not change its command identity")?
}

pub(super) async fn execute(client: &Client, command: InboxCommand) -> Result<Value> {
    let operation = match command {
        InboxCommand::Configure { file, command_id } => NotificationOperation::Configure {
            command_id,
            destination: load_destination(&file)?,
        },
        InboxCommand::Accept {
            destination,
            command_id,
        } => NotificationOperation::Accept {
            command_id,
            destination_id: destination,
        },
        InboxCommand::Destinations => NotificationOperation::Destinations,
        InboxCommand::Revoke {
            destination,
            command_id,
        } => NotificationOperation::Revoke {
            command_id,
            destination_id: destination,
        },
        InboxCommand::Test {
            destination,
            command_id,
        } => NotificationOperation::Test {
            command_id,
            destination_id: destination,
        },
        InboxCommand::List {
            destination,
            after,
            limit,
        } => {
            ensure!((1..=100).contains(&limit), "limit must be 1..100");
            NotificationOperation::Inbox {
                destination_id: destination,
                after,
                limit,
            }
        }
        InboxCommand::Inspect { destination, event } => NotificationOperation::Open {
            destination_id: destination,
            event_id: event,
        },
        InboxCommand::Seen { destination, event } => NotificationOperation::Receipt {
            destination_id: destination,
            event_id: event,
            state: ReceiptState::Seen,
        },
        InboxCommand::Dismiss { destination, event } => NotificationOperation::Receipt {
            destination_id: destination,
            event_id: event,
            state: ReceiptState::Dismissed,
        },
        InboxCommand::Watch { .. } => anyhow::bail!("watch requires the streaming CLI dispatcher"),
    };
    request(client, operation).await
}

/// Validate the server's cursor instead of spinning on a malformed/repeated page.
fn next_cursor(value: &Value, after: u64) -> Result<u64> {
    let next = value["next_after"]
        .as_u64()
        .context("inbox page has no cursor")?;
    ensure!(next >= after, "inbox cursor moved backwards");
    ensure!(
        value["has_more"].is_boolean(),
        "inbox page has no continuation flag"
    );
    ensure!(
        value["has_more"] != true || next > after,
        "inbox cursor did not advance"
    );
    Ok(next)
}

pub(super) async fn watch(
    client: &Client,
    destination: Uuid,
    mut after: u64,
    seconds: u64,
) -> Result<()> {
    ensure!(
        (1..=3600).contains(&seconds),
        "watch duration must be 1..3600 seconds"
    );
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    loop {
        let value = tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep_until(deadline) => return Ok(()),
            value = request(client, NotificationOperation::Inbox {
                destination_id: destination, after, limit: 100,
            }) => value?,
        };
        after = next_cursor(&value["page"], after)?;
        // JSON encoding escapes control characters. One response envelope per line.
        println!("{}", serde_json::to_string(&value)?);
        tokio::select! {
            _ = tokio::signal::ctrl_c() => return Ok(()),
            _ = tokio::time::sleep_until(deadline) => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(1)) => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn destination_input_is_bounded_and_diagnostics_do_not_echo_content() {
        let secret = br#"{"summary":"do-not-echo-secret"}"#;
        let error = decode_destination(secret).unwrap_err().to_string();
        assert!(!error.contains("do-not-echo-secret"));
        assert!(decode_destination(&vec![b' '; 65_537]).is_err());
    }

    #[test]
    fn watch_rejects_missing_backwards_and_stuck_cursors() {
        for value in [
            json!({}),
            json!({"next_after": 2,"has_more": false}),
            json!({"next_after": 3,"has_more": true}),
            json!({"next_after": 4}),
        ] {
            assert!(next_cursor(&value, 3).is_err());
        }
        assert_eq!(
            next_cursor(&json!({"next_after": 3,"has_more": false}), 3).unwrap(),
            3
        );
        assert_eq!(
            next_cursor(&json!({"next_after": 4,"has_more": true}), 3).unwrap(),
            4
        );
    }

    #[test]
    fn cli_bounds_duration_and_page_size_and_requires_mutation_identity() {
        #[derive(clap::Parser)]
        struct Args {
            #[command(subcommand)]
            command: InboxCommand,
        }
        use clap::Parser;
        let id = Uuid::nil().to_string();
        for args in [
            vec!["inbox", "watch", &id, "--seconds", "0"],
            vec!["inbox", "watch", &id, "--seconds", "3601"],
            vec!["inbox", "list", &id, "--limit", "101"],
            vec!["inbox", "revoke", &id],
            vec!["inbox", "test", &id],
        ] {
            assert!(Args::try_parse_from(args).is_err());
        }
        assert!(Args::try_parse_from(["inbox", "inspect", &id, &id]).is_ok());
    }
}
