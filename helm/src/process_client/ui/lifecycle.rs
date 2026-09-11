use super::{
    App, drafts,
    observe::Update,
    state::{Pending, Target},
};
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::vessel::VesselCommand;

impl App {
    pub(super) fn branch(
        &mut self,
        target: Target,
        name: Option<String>,
        preserve_draft: bool,
    ) -> Result<()> {
        ensure!(
            self.clients.available(target.route),
            "Vessel disconnected; reconnect before changing remote work"
        );
        let view = self
            .views
            .get_mut(&target)
            .context("selected voyage unavailable")?;
        ensure!(
            view.pending.is_none(),
            "resolve pending command before branching"
        );
        let snapshot = view
            .snapshot
            .as_ref()
            .context("waiting for canonical snapshot")?;
        let expected_revision = snapshot.revision;
        let incarnation = view.process.incarnation;
        let command_id = Uuid::new_v4();
        let branch_id = Uuid::new_v4();
        let expires_at_ms = super::super::frontend::deadline()?;
        view.pending = Some(Pending {
            account_host: None,
            command_id,
            incarnation,
            original: None,
            receipt_only: true,
            draft: view.draft.text.clone(),
            preserve_draft,
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error.context("cannot persist branch identity; nothing sent"));
        }
        self.status = "Creating a separate voyage...".into();
        let client = self.clients[target.route].clone();
        let sender = self.sender.clone();
        self.command_checks.insert((target, command_id), None);
        let job = tokio::spawn(async move {
            let result = client
                .request(VesselCommand::Branch {
                    command_id,
                    session_id: target.session,
                    incarnation,
                    expected_revision,
                    expires_at_ms,
                    branch_id,
                    name,
                })
                .await;
            let refused = result.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<super::super::transport::Refusal>()
                    .is_some()
            });
            match result {
                Ok(value) => {
                    let _=sender.send(Update::Command {target,command_id,refused:false,result:Ok(serde_json::json!({"command_id":command_id,"branch_id":branch_id,"status":"accepted"}))}).await;
                    let result = serde_json::from_value(value).map_err(|error| error.to_string());
                    let _ = sender
                        .send(Update::Created {
                            route: target.route,
                            result,
                        })
                        .await;
                }
                Err(error) => {
                    let _ = sender
                        .send(Update::Command {
                            target,
                            command_id,
                            refused,
                            result: Err(error.to_string()),
                        })
                        .await;
                }
            }
        });
        self.route_tasks
            .entry(target.route.id)
            .or_default()
            .push(job);
        Ok(())
    }
}
