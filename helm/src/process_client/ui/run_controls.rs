//! Stop is a reviewed exact-run request, never a synonym for detaching Helm.
use super::{
    App, receipts,
    state::{Pending, Target},
};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Clear, Paragraph},
};
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

pub(super) struct StopReview {
    target: Target,
    incarnation: Uuid,
    revision: u64,
    run_id: Uuid,
    error: Option<String>,
    scroll: std::cell::Cell<u16>,
}

impl StopReview {
    fn command(
        &self,
        incarnation: Uuid,
        snapshot: &super::state::Snapshot,
    ) -> Result<VoyageCommand> {
        ensure!(
            incarnation == self.incarnation,
            "Voyage owner changed; reopen Stop to review it"
        );
        ensure!(
            !snapshot.recovery_pending,
            "Voyage recovery pending; no Stop sent"
        );
        ensure!(
            snapshot.revision == self.revision,
            "Voyage changed; reopen Stop to review the current run"
        );
        ensure!(
            snapshot
                .run
                .as_ref()
                .is_some_and(|run| run.run_id == self.run_id
                    && run.active()
                    && run.state != "cancel_requested"),
            "Reviewed run is no longer stoppable; no request sent"
        );
        Ok(VoyageCommand::Cancel {
            command_id: Uuid::new_v4(),
            expected_revision: self.revision,
            expires_at_ms: super::super::frontend::deadline()?,
            run_id: self.run_id,
        })
    }
}

impl App {
    pub(super) fn review_stop(&mut self, target: Target) -> Result<()> {
        ensure!(
            self.clients.available(target.route),
            "Vessel disconnected; reconnect before Stop"
        );
        let view = self.views.get(&target).context("Voyage unavailable")?;
        ensure!(
            view.pending.is_none(),
            "Command outcome pending; resolve its identity before Stop"
        );
        let snapshot = view
            .snapshot
            .as_ref()
            .context("Waiting for canonical snapshot")?;
        ensure!(
            !snapshot.recovery_pending,
            "Voyage recovery pending; observe its original owner first"
        );
        let run = snapshot
            .run
            .as_ref()
            .filter(|run| run.active())
            .context("No active run to stop")?;
        ensure!(
            run.state != "cancel_requested",
            "Stop already requested; cleanup is not yet confirmed"
        );
        self.stop_review = Some(StopReview {
            target,
            incarnation: view.process.incarnation,
            revision: snapshot.revision,
            run_id: run.run_id,
            error: None,
            scroll: Default::default(),
        });
        Ok(())
    }

    fn confirm_stop(&mut self, review: &StopReview) -> Result<()> {
        ensure!(
            self.clients.available(review.target.route),
            "Vessel disconnected; nothing sent"
        );
        let view = self
            .views
            .get_mut(&review.target)
            .context("Voyage unavailable")?;
        ensure!(
            view.pending.is_none(),
            "Another command is pending; nothing sent"
        );
        let snapshot = view.snapshot.as_ref().context("Snapshot unavailable")?;
        let command = review.command(view.process.incarnation, snapshot)?;
        let command_id = match &command {
            VoyageCommand::Cancel { command_id, .. } => *command_id,
            _ => unreachable!(),
        };
        view.pending = Some(Pending {
            account_host: None,
            command_id,
            incarnation: review.incarnation,
            draft: view.draft.text.clone(),
            preserve_draft: true,
            original: Some(Box::new(command.clone())),
            receipt_only: false,
        });
        if let Err(error) = receipts::save(&self.clients[review.target.route], view) {
            view.pending = None;
            return Err(error.context("Cannot retain Stop identity; nothing sent"));
        }
        self.dispatch(review.target, command_id, command);
        self.status =
            "Stop request pending · not yet confirmed · unsent text and images retained".into();
        Ok(())
    }

    pub(super) fn stop_input(&mut self, event: &Event) -> bool {
        let Some(mut review) = self.stop_review.take() else {
            return false;
        };
        if let Event::Key(key) = event
            && key.kind != KeyEventKind::Release
        {
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(key.code, KeyCode::Char('c' | 'q'))
            {
                self.quit = true; // Detach only. Private child input must run before this handler.
                return true;
            }
            match key.code {
                KeyCode::Esc => return true,
                KeyCode::PageUp | KeyCode::Up => {
                    review.scroll.set(review.scroll.get().saturating_sub(3))
                }
                KeyCode::PageDown | KeyCode::Down => {
                    review.scroll.set(review.scroll.get().saturating_add(3))
                }
                KeyCode::Enter => match self.confirm_stop(&review) {
                    Ok(()) => return true,
                    Err(error) => review.error = Some(super::safe(&error.to_string())),
                },
                _ => {}
            }
        }
        self.stop_review = Some(review);
        true
    }

    pub(super) fn draw_stop(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(review) = &self.stop_review else {
            return;
        };
        let width = area.width.min(78);
        let height = area.height.min(16);
        let area = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, area);
        let text = format!(
            "Host: {}\nVoyage: {}\nRun: {}\nEffective: this exact run, after cancellation is admitted. Cleanup may remain pending.\n\nStop requests cancellation; it does not prove cleanup completed. Unsent text and images remain.\n\nEnter Request Stop · Esc Back\nCtrl+C Detach Helm — voyage continues\n{}",
            self.route_label(review.target.route),
            review.target.session,
            review.run_id,
            review.error.as_deref().unwrap_or("")
        );
        let block = super::right_panel::block("Stop · Enter Request · Esc Back · PgUp/PgDn");
        let inner = block.inner(area);
        let text = super::presentation::wrap(ratatui::text::Text::raw(text), inner.width);
        let maximum = text
            .lines
            .len()
            .saturating_sub(inner.height as usize)
            .min(u16::MAX as usize) as u16;
        review.scroll.set(review.scroll.get().min(maximum));
        frame.render_widget(
            Paragraph::new(text)
                .scroll((review.scroll.get(), 0))
                .block(block),
            area,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(run_id: Uuid) -> super::super::state::Snapshot {
        serde_json::from_value(serde_json::json!({
            "session_id": Uuid::new_v4(), "revision": 7, "name": null, "model": "test", "messages": [],
            "run": { "run_id": run_id, "state": "running" }
        }))
        .unwrap()
    }

    #[test]
    fn stop_is_bound_to_reviewed_owner_run_and_revision() {
        let incarnation = Uuid::new_v4();
        let run_id = Uuid::new_v4();
        let target = super::super::state::Target {
            route: super::super::state::Route {
                id: Uuid::new_v4(),
                generation: 1,
            },
            session: Uuid::new_v4(),
        };
        let review = StopReview {
            target,
            incarnation,
            revision: 7,
            run_id,
            error: None,
            scroll: Default::default(),
        };
        let mut snapshot = snapshot(run_id);
        assert!(
            matches!(review.command(incarnation, &snapshot).unwrap(), VoyageCommand::Cancel { run_id: id, expected_revision: 7, .. } if id == run_id)
        );
        assert!(review.command(Uuid::new_v4(), &snapshot).is_err());
        snapshot.revision = 8;
        assert!(review.command(incarnation, &snapshot).is_err());
        snapshot.revision = 7;
        snapshot.run.as_mut().unwrap().run_id = Uuid::new_v4();
        assert!(review.command(incarnation, &snapshot).is_err());
        snapshot.run.as_mut().unwrap().run_id = run_id;
        for state in [
            "completed",
            "failed",
            "cancelled",
            "cancel_requested",
            "interrupted",
        ] {
            snapshot.run.as_mut().unwrap().state = state.into();
            assert!(review.command(incarnation, &snapshot).is_err());
        }
        snapshot.run.as_mut().unwrap().state = "running".into();
        snapshot.recovery_pending = true;
        assert!(review.command(incarnation, &snapshot).is_err());
    }
}
