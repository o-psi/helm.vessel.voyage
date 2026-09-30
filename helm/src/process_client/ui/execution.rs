//! Explicit human review; observation never repeats an uncertain approval.
use super::{
    App,
    observe::Update,
    state::{Target, View},
};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::execution_identity::{ExecutionOutcome, IdentityRef, ReviewApproval};
use voyage_protocol::execution_review_control::*;

#[cfg(unix)]
fn retain(target: Target, ids: [Uuid; 3]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    let root =
        crate::process_client::cli::default_directory().with_file_name("helm-execution-reviews");
    std::fs::create_dir_all(
        root.parent()
            .context("execution receipt parent unavailable")?,
    )?;
    match std::fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    };
    crate::process_client::local::check_private_directory(&root)?;
    let name = root.join(format!("{}-{}.json", target.route.id, target.session));
    if let Ok(metadata) = std::fs::symlink_metadata(&name) {
        ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "unsafe execution receipt"
        );
    }
    let temporary = root.join(format!("{}.tmp", Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(&ids)?)?;
    file.sync_all()?;
    std::fs::rename(temporary, &name)?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(unix)]
fn restored(target: Target) -> Result<Option<[Uuid; 3]>> {
    use std::io::Read;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let root =
        crate::process_client::cli::default_directory().with_file_name("helm-execution-reviews");
    if !root.try_exists()? {
        return Ok(None);
    }
    crate::process_client::local::check_private_directory(&root)?;
    let name = root.join(format!("{}-{}.json", target.route.id, target.session));
    let mut file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(name)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.uid() == unsafe { libc::geteuid() }
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && metadata.len() <= 1024,
        "unsafe execution receipt"
    );
    let mut bytes = Vec::new();
    file.take(1025).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024, "execution receipt exceeds bound");
    let ids: [Uuid; 3] = serde_json::from_slice(&bytes)?;
    ensure!(
        ids.iter().all(|id| !id.is_nil()),
        "invalid execution receipt"
    );
    Ok(Some(ids))
}
#[cfg(not(unix))]
fn retain(_target: Target, _ids: [Uuid; 3]) -> Result<()> {
    anyhow::bail!("private execution receipt storage is unavailable on this platform")
}
#[cfg(not(unix))]
fn restored(_target: Target) -> Result<Option<[Uuid; 3]>> {
    Ok(None)
}
impl App {
    pub(super) fn execution_command(&mut self, target: Target, text: &str) -> Result<()> {
        ensure!(
            self.clients.available(target.route),
            "Reconnect to observe the retained execution review"
        );
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        let view = self
            .views
            .get_mut(&target)
            .context("execution target unavailable")?;
        if view.execution_pending.is_none() {
            view.execution_pending = restored(target)?;
        }
        let words = text.split_whitespace().skip(1).collect::<Vec<_>>();
        let operation = match words.as_slice() {
            [] | ["status"] => ExecutionOperation::Status {
                session_id: target.session,
            },
            ["identities"] => ExecutionOperation::Inventory,
            ["prepare", id, revision] => {
                ensure!(
                    view.execution_pending.is_none(),
                    "Check or cancel the retained review before creating another"
                );
                let identity = IdentityRef {
                    id: id.parse()?,
                    revision: revision.parse()?,
                };
                let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
                retain(target, ids)?;
                view.execution_pending = Some(ids);
                ExecutionOperation::Prepare {
                    review_id: ids[0],
                    command_id: ids[1],
                    session_id: ids[2],
                    workspace: view.process.workspace.clone(),
                    identity,
                }
            }
            ["transition", id, revision, "stop-source"] => {
                ensure!(
                    view.execution_pending.is_none(),
                    "Check or cancel the retained review before creating another"
                );
                let identity = IdentityRef {
                    id: id.parse()?,
                    revision: revision.parse()?,
                };
                let ids = [Uuid::new_v4(), Uuid::new_v4(), target.session];
                retain(target, ids)?;
                view.execution_pending = Some(ids);
                ExecutionOperation::PrepareTransition {
                    review_id: ids[0],
                    command_id: ids[1],
                    session_id: target.session,
                    source_incarnation: view.process.incarnation,
                    identity,
                    stop_source: true,
                }
            }
            ["check"] => ExecutionOperation::Review {
                review_id: view.execution_pending.context("No retained review")?[0],
            },
            ["review", id] => ExecutionOperation::Review {
                review_id: id.parse()?,
            },
            ["reconcile"] => {
                let saved = view
                    .execution_review
                    .as_ref()
                    .context("Read retained transition review first")?;
                ensure!(
                    saved.review.facts.change
                        == voyage_protocol::execution_identity::ExecutionChange::Transition
                        && matches!(
                            saved.receipt.outcome,
                            ExecutionOutcome::Launching | ExecutionOutcome::Unconfirmed { .. }
                        ),
                    "Only unresolved transition metadata can be reconciled"
                );
                ExecutionOperation::ReconcileTransition {
                    review_id: saved.review.review_id,
                    command_id: Uuid::new_v4(),
                    digest: saved.review.digest.clone(),
                }
            }
            [action @ ("approve" | "cancel" | "revoke")] => {
                let saved = view
                    .execution_review
                    .as_ref()
                    .context("Read the exact prepared review first")?;
                let approval = ReviewApproval {
                    review_id: saved.review.review_id,
                    command_id: saved.review.command_id,
                    digest: saved.review.digest.clone(),
                };
                if *action == "approve" {
                    ensure!(
                        !view.execution_uncertain
                            && saved.receipt.outcome == ExecutionOutcome::AwaitingApproval
                            && chrono::Utc::now().timestamp_millis()
                                < i64::try_from(saved.review.expires_at_ms)?,
                        "Review is stale or an approval outcome is uncertain; /execution check first"
                    );
                    view.execution_uncertain = true;
                    ExecutionOperation::Approve { approval }
                } else {
                    ExecutionOperation::Control {
                        control: ExecutionReviewControl {
                            command_id: Uuid::new_v4(),
                            review_id: approval.review_id,
                            digest: approval.digest,
                            action: if *action == "cancel" {
                                ExecutionReviewControlAction::Cancel
                            } else {
                                ExecutionReviewControlAction::Revoke
                            },
                        },
                    }
                }
            }
            _ => anyhow::bail!(
                "/execution identities | prepare ID REVISION | transition ID REVISION stop-source | check | reconcile | approve | cancel | revoke | review REVIEW_UUID"
            ),
        };
        let incarnation = view.process.incarnation;
        self.status =
            "Execution request captured; retained IDs observe one exact operation.".into();
        tokio::spawn(async move {
            let result = client
                .execution(operation)
                .await
                .map_err(|error| error.to_string());
            let _ = sender
                .send(Update::Execution {
                    target,
                    incarnation,
                    result,
                })
                .await;
        });
        Ok(())
    }
    pub(super) fn execution_arrived(
        &mut self,
        target: Target,
        incarnation: Uuid,
        result: Result<serde_json::Value, String>,
    ) {
        let Some(view) = self
            .views
            .get_mut(&target)
            .filter(|view| view.process.incarnation == incarnation)
        else {
            return;
        };
        match result {
            Err(error) => {
                view.execution_uncertain = true;
                view.error = Some(super::safe(&error));
                self.status="Execution outcome unavailable; /execution check observes and never repeats approval.".into();
            }
            Ok(value) => {
                if let Ok(saved) = serde_json::from_value::<SavedExecutionReview>(value.clone()) {
                    let ids = [
                        saved.review.review_id,
                        saved.review.command_id,
                        saved.review.facts.session_id,
                    ];
                    if retain(target, ids).is_err() {
                        view.execution_uncertain = true;
                        view.error = Some(
                            "Execution receipt could not be retained; keep this view open.".into(),
                        );
                        return;
                    }
                    view.execution_pending = Some(ids);
                    view.execution_uncertain = false;
                    if matches!(
                        saved.receipt.outcome,
                        ExecutionOutcome::Cancelled | ExecutionOutcome::Refused { .. }
                    ) {
                        if forget(target).is_ok() {
                            view.execution_pending = None;
                        }
                    }
                    view.panel = Some(super::safe(&format!(
                        "EXECUTION REVIEW\nState: {:?}\nVoyage: {}\nWorkspace: {}\nAccount: {}\nReview: {}\nExpires: {}\n\nAdministrator execution can alter host files, processes, credentials and Vessel. Namespaces/mounts still constrain actual authority; root can alter local receipts.\n\n/execution approve explicitly authorizes this exact voyage/account/workspace until revoked.\n/execution cancel cancels only pending review; /execution revoke fences authorization, without claiming cleanup.\n/execution check observes the retained receipt. Ready is a launch observation, not a completed run. /use VOYAGE_UUID opens the new voyage.\nCurrent draft remains in its original voyage.",
                        saved.receipt.outcome,
                        saved.review.facts.session_id,
                        saved.review.facts.workspace.display(),
                        saved.review.facts.account.account_id,
                        saved.review.review_id,
                        saved.review.expires_at_ms
                    )));
                    view.execution_review = Some(saved);
                } else {
                    view.panel = Some(super::safe(&format!(
                        "Execution observation\n{}\n\n/execution identities lists configured references. Administrator prepare requires the exact ID/revision, explicit host provisioning and separately enrolled fresh owner connection.\nUnknown/last launch evidence does not mean the process is live.",
                        serde_json::to_string_pretty(&value).unwrap_or_default()
                    )));
                }
                view.scroll = 0;
                self.status =
                    "Execution overview ready. Esc returns to the retained conversation/draft."
                        .into();
            }
        }
    }
}

#[cfg(unix)]
fn forget(target: Target) -> Result<()> {
    let root =
        crate::process_client::cli::default_directory().with_file_name("helm-execution-reviews");
    crate::process_client::local::check_private_directory(&root)?;
    std::fs::remove_file(root.join(format!("{}-{}.json", target.route.id, target.session)))?;
    std::fs::File::open(root)?.sync_all()?;
    Ok(())
}
#[cfg(not(unix))]
fn forget(_target: Target) -> Result<()> {
    Ok(())
}

#[cfg(test)]
#[path = "execution_boundary_tests.rs"]
mod boundary_tests;
