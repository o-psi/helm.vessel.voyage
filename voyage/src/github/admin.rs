//! Attended, local-only recovery. No transport or model adapter is available here.
use super::{operator::CommandResult, store::Owner};
use crate::tools::{ApprovalOutcome, InteractionMode, ToolContext};
use anyhow::{Result, ensure};
use serde_json::Value;
use uuid::Uuid;

#[derive(Clone, Debug, clap::Subcommand)]
pub enum Command {
    List {
        #[arg(long, default_value_t = 0)]
        offset: u32,
    },
    Inspect {
        id: Uuid,
    },
    Cancel {
        id: Uuid,
        digest: String,
    },
    Dispose {
        id: Uuid,
        digest: String,
        note: String,
    },
    Forget {
        id: Uuid,
        digest: String,
    },
    /// Export bounded private audit evidence as JSON before explicit maintenance.
    Audit,
    /// Delete only the audit snapshot identified by its exported digest.
    ClearAudit {
        digest: String,
    },
}

fn current(context: &ToolContext, write: bool) -> Result<()> {
    context.policy.check_current()?;
    ensure!(
        !context.cancellation.is_cancelled(),
        "GitHub administration cancelled"
    );
    ensure!(
        context.interaction == InteractionMode::Attended,
        "GitHub administration requires an attended local operator"
    );
    ensure!(
        !write || context.policy.access_mode() != crate::config::AccessMode::ReadOnly,
        "GitHub administration is denied in read-only mode"
    );
    Ok(())
}

async fn database<T: Send + 'static>(
    context: &ToolContext,
    directory: &std::path::Path,
    write: bool,
    operation: impl FnOnce(&mut super::store::Store) -> Result<T> + Send + 'static,
) -> Result<T> {
    current(context, write)?;
    let cloned = context.clone();
    let value = super::service::database_at(directory.to_owned(), move |store| {
        current(&cloned, write)?;
        operation(store)
    })
    .await?;
    current(context, write)?;
    Ok(value)
}

pub(super) async fn execute(
    context: ToolContext,
    command: Command,
    scoped: Option<Owner>,
) -> Result<CommandResult> {
    execute_inner(
        context,
        command,
        scoped,
        super::store::Store::default_path(),
    )
    .await
}

async fn execute_inner(
    context: ToolContext,
    command: Command,
    scoped: Option<Owner>,
    directory: std::path::PathBuf,
) -> Result<CommandResult> {
    current(&context, false)?;
    let inspect = |id| {
        let scoped = scoped.clone();
        move |store: &mut super::store::Store| {
            if let Some(owner) = scoped {
                store.inspect(id, &owner)
            } else {
                store.admin_inspect(id)
            }
        }
    };
    let value = match command {
        Command::List { offset } => {
            database(&context, &directory, false, move |store| {
                Ok(serde_json::to_value(store.admin_list(offset)?)?)
            })
            .await?
        }
        Command::Inspect { id } => {
            serde_json::to_value(database(&context, &directory, false, inspect(id)).await?)?
        }
        Command::Audit => {
            database(&context, &directory, false, move |store| {
                Ok(serde_json::to_value(store.audit()?)?)
            })
            .await?
        }
        Command::ClearAudit { digest } => {
            let snapshot = database(&context, &directory, false, |store| store.audit()).await?;
            ensure!(
                snapshot.digest == digest,
                "GitHub audit changed; export its current snapshot"
            );
            confirm(&context, &serde_json::json!({"action":"clear local audit","digest":digest,
                "entries":snapshot.entries.len(),"notice":"Keep the exported private evidence. This permanently removes local audit evidence; it never authorizes another remote send."})).await?;
            let removed = database(&context, &directory, true, move |store| {
                store.clear_audit(&digest)
            })
            .await?;
            serde_json::json!({"removed_audit_entries":removed})
        }
        command => {
            let (id, digest) = match &command {
                Command::Cancel { id, digest }
                | Command::Dispose { id, digest, .. }
                | Command::Forget { id, digest } => (*id, digest.clone()),
                _ => unreachable!(),
            };
            let operation = database(&context, &directory, false, inspect(id)).await?;
            ensure!(
                operation.digest == digest,
                "GitHub operation changed; inspect the exact current record"
            );
            match &command {
                Command::Cancel { .. } => ensure!(
                    operation.state == super::store::State::Prepared,
                    "only prepared operations can be cancelled"
                ),
                Command::Dispose { .. } => ensure!(
                    operation.state == super::store::State::Sending,
                    "only uncertain operations need disposition"
                ),
                Command::Forget { .. } => ensure!(
                    matches!(
                        operation.state,
                        super::store::State::Published
                            | super::store::State::Cancelled
                            | super::store::State::Disposed
                    ),
                    "only terminal records can be forgotten"
                ),
                _ => unreachable!(),
            }
            let note = match &command {
                Command::Dispose { note, .. } => Some(note),
                _ => None,
            };
            confirm(&context,&serde_json::json!({"operation":operation,"disposition":note,
                "action":match command { Command::Cancel { .. } => "cancel prepared record",Command::Dispose { .. } => "record uncertain disposition",_=>"forget terminal record after atomic audit" },
                "notice":"Local maintenance does not establish that a request was unsent and never authorizes repetition."})).await?;
            database(&context, &directory, true, move |store| match command {
                Command::Cancel { .. } => {
                    Ok(serde_json::to_value(store.cancel_exact(&operation)?)?)
                }
                Command::Dispose { note, .. } => Ok(serde_json::to_value(
                    store.dispose_exact(&operation, note)?,
                )?),
                Command::Forget { .. } => {
                    store.forget_exact(&operation)?;
                    Ok(serde_json::json!({"forgotten":id,"audit_recorded":true}))
                }
                _ => unreachable!(),
            })
            .await?
        }
    };
    current(&context, false)?;
    Ok(CommandResult {
        display: serde_json::to_string_pretty(&super::redact_value(&value, &context.redactor))?,
        reference: None,
        feedback: None,
    })
}

async fn confirm(context: &ToolContext, value: &Value) -> Result<()> {
    current(context, true)?;
    ensure!(
        !super::value_has_secret(value, &context.redactor),
        "GitHub exact administration preview contains a configured secret"
    );
    let preview = super::service::json_preview(value)?;
    ensure!(
        !context.redactor.contains_secret(&preview),
        "GitHub exact administration preview contains a configured secret"
    );
    let request = context.approval("github.dispose", "local GitHub journal", preview);
    let decision = tokio::select! {
        biased;
        _ = context.cancellation.cancelled() => anyhow::bail!("GitHub administration cancelled"),
        decision = tokio::time::timeout(std::time::Duration::from_secs(900),context.approver.approve(&request)) => decision.unwrap_or(ApprovalOutcome::Expired),
    };
    decision.require_approved()?;
    current(context, true)
}
