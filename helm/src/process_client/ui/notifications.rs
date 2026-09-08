//! Local result notification lifecycle, independent of executor suspension.
use super::{App, drafts, state::View};
use uuid::Uuid;
use voyage_protocol::process::ProcessState;

impl View {
    pub(super) fn terminal_completion(&self) -> Option<Uuid> {
        let run = self.snapshot.as_ref()?.run.as_ref()?;
        matches!(
            run.state.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted"
        )
        .then_some(run.run_id)
    }

    pub(super) fn completion_unread(&self) -> bool {
        self.terminal_completion().is_some_and(|run| {
            self.acknowledged_completion != Some(run) && self.viewed_completion.get() != Some(run)
        })
    }

    pub(super) fn sidebar_suspended(&self) -> bool {
        self.process.state == ProcessState::Suspended
            && !self.archived()
            && self.error.is_none()
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.decisions.is_empty()
                    && snapshot.pending_cleanup_run.is_none()
                    && (snapshot.run.is_none()
                        || self
                            .terminal_completion()
                            .is_some_and(|run| self.acknowledged_completion == Some(run)))
            })
    }

    pub(super) fn mark_completion_viewed(&self) {
        if matches!(
            self.process.state,
            ProcessState::Live | ProcessState::Suspended
        ) && self.error.is_none()
            && !self.archived()
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.decisions.is_empty() && snapshot.pending_cleanup_run.is_none()
            })
        {
            self.viewed_completion.set(self.terminal_completion());
        }
    }
}

impl App {
    /// Central leave detection covers keyboard, mouse, draft and archive navigation.
    /// Rendering (not merely selection) records which exact result was visited.
    pub(super) fn acknowledge_departed_completions(&mut self) {
        for (target, view) in &mut self.views {
            if self.active_draft.is_none() && self.selected == Some(*target) {
                continue;
            }
            let Some(viewed) = view.viewed_completion.take() else {
                continue;
            };
            // A newer result arriving between display and departure stays unread.
            if view.terminal_completion() != Some(viewed)
                || view.acknowledged_completion == Some(viewed)
            {
                continue;
            }
            view.acknowledged_completion = Some(viewed);
            view.unread = false;
            if let Err(error) = drafts::save(&self.clients[target.route], view) {
                self.status =
                    format!("Result reviewed, but saving its acknowledgement failed: {error}");
            }
        }
    }
}
