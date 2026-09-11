use super::super::{
    App, drafts,
    state::{Pending, Target},
};
use super::now_ms;
use anyhow::{Context, Result, ensure};
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

impl App {
    pub(super) fn respond_to_interaction(
        &mut self,
        target: Target,
        decision_id: Uuid,
        response: serde_json::Value,
    ) -> Result<()> {
        ensure!(
            self.clients.available(target.route),
            "Vessel unavailable · Ctrl+G to manage / retry; no response sent"
        );
        let view = self
            .views
            .get_mut(&target)
            .context("reviewed voyage unavailable")?;
        ensure!(
            view.pending.is_none(),
            "Waiting for delivery confirmation; Helm checks automatically"
        );
        let snapshot = view
            .snapshot
            .as_ref()
            .context("waiting for current snapshot")?;
        let decision = snapshot
            .decisions
            .iter()
            .find(|decision| decision.decision_id == decision_id)
            .context("reviewed request is no longer pending")?;
        super::validate_response(&decision.request, &response)?;
        ensure!(
            decision.incarnation == view.process.incarnation,
            "request belongs to an earlier runtime"
        );
        ensure!(
            decision.expires_at_ms > now_ms(),
            "request expired; nothing sent"
        );
        let command_id = Uuid::new_v4();
        let command = VoyageCommand::Respond {
            command_id,
            expected_revision: snapshot.revision,
            expires_at_ms: decision.expires_at_ms.min(now_ms().saturating_add(60_000)),
            run_id: decision.run_id,
            decision_id,
            response,
        };
        view.pending = Some(Pending {
            account_host: None,
            command_id,
            original: Some(Box::new(command.clone())),
            receipt_only: false,
            incarnation: view.process.incarnation,
            draft: String::new(),
            preserve_draft: true,
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error.context("cannot persist response identity; nothing sent"));
        }
        self.dispatch(target, command_id, command);
        Ok(())
    }
}
