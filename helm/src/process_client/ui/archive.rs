//! Archive navigation never deletes history or treats an unavailable owner as stopped.
use super::{
    App, drafts,
    observe::Update,
    state::{Pending, Target},
};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, ProcessState, VesselCommand, VoyageCommand};

impl App {
    pub(super) fn show_archives(&mut self, archived: bool) {
        self.archives = archived;
        self.selected = self.ordered_targets().first().copied();
        self.status = if archived {
            "Archived voyages · Select with Tab, then /restore. F5 returns to current voyages."
        } else {
            "Current voyages · F5 opens archives. /archive puts the selected voyage away."
        }
        .into();
    }

    pub(super) fn restore_archive(&mut self, target: Target, preserve_draft: bool) -> Result<()> {
        let view = self
            .views
            .get_mut(&target)
            .context("select an archived voyage")?;
        ensure!(
            view.pending.is_none(),
            "resolve the pending command first with /receipt"
        );
        ensure!(
            view.process.state == ProcessState::Stopped && view.process.archive.is_some(),
            "archive cleanup is not yet confirmed; wait for the stopped owner"
        );
        let command_id = Uuid::new_v4();
        let incarnation = view.process.incarnation;
        view.pending = Some(Pending {
            command_id,
            incarnation,
            original: None,
            receipt_only: true,
            draft: view.draft.text.clone(),
            preserve_draft,
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error);
        }
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        self.status = "Restoring voyage…".into();
        tokio::spawn(async move {
            let result: Result<_> = async {
                let process: ProcessInfo = serde_json::from_value(
                    client
                        .request(VesselCommand::Restart {
                            command_id,
                            session_id: target.session,
                            incarnation,
                        })
                        .await?,
                )?;
                let _ = sender
                    .send(Update::Created {
                        route: target.route,
                        result: Ok(process.clone()),
                    })
                    .await;
                let snapshot = client
                    .voyage(target.session, process.incarnation, VoyageCommand::Snapshot)
                    .await?;
                client
                    .voyage(
                        target.session,
                        process.incarnation,
                        VoyageCommand::Archive {
                            command_id,
                            expected_revision: snapshot["revision"]
                                .as_u64()
                                .context("missing revision")?,
                            expires_at_ms: crate::process_client::frontend::deadline()?,
                            archived: false,
                        },
                    )
                    .await
            }
            .await;
            let refused = result.as_ref().err().is_some_and(|e| {
                e.downcast_ref::<crate::process_client::transport::Refusal>()
                    .is_some()
            });
            let _ = sender
                .send(Update::Command {
                    target,
                    command_id,
                    refused,
                    result: result.map_err(|e| e.to_string()),
                })
                .await;
        });
        Ok(())
    }
}
