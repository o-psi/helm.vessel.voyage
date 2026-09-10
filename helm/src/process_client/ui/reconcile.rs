//! Outcome recovery belongs to the connection, not a user key or selected view.
use super::*;

impl App {
    pub(super) fn reconcile_pending(&mut self) {
        self.observe_retired();
        self.refresh_draft_capabilities();
        let now = Instant::now();
        self.command_checks.retain(|(target, id), _| {
            self.views.get(target).is_some_and(|view| {
                view.pending
                    .as_ref()
                    .is_some_and(|pending| pending.command_id == *id)
            })
        });
        self.first_send_checks
            .retain(|id, _| self.new_drafts.contains_key(id));
        // Keep a stable due time for every waiting item. Oldest-due scheduling
        // across both classes prevents unavailable routes starving new voyages.
        let mut ready = Vec::new();
        for (target, view) in &self.views {
            if !self.clients.available(target.route) {
                continue;
            }
            if self.new_drafts.contains_key(&target.session) {
                continue; // The durable first-send handoff still owns recovery.
            }
            if let Some(pending) = &view.pending {
                let due = self
                    .command_checks
                    .entry((*target, pending.command_id))
                    .or_insert(Some(now));
                if let Some(due) = *due
                    && due <= now
                {
                    ready.push((due, Some(*target), pending.command_id));
                }
            }
        }
        for (id, draft) in &self.new_drafts {
            if self.clients.available(draft.route) && !draft.busy && draft.saved.start.is_some() {
                let due = *self.first_send_checks.entry(*id).or_insert(now);
                if due <= now {
                    ready.push((due, None, *id));
                }
            }
        }
        ready.sort_unstable();
        // At most four recovery jobs at once, including requests still awaiting
        // their bounded transport timeout. Never race a check with initial send.
        let busy = self
            .command_checks
            .values()
            .filter(|due| due.is_none())
            .count()
            + self.new_drafts.values().filter(|draft| draft.busy).count();
        for (_, target, id) in ready.into_iter().take(4usize.saturating_sub(busy)) {
            if let Some(target) = target {
                let pending = self.views[&target]
                    .pending
                    .as_ref()
                    .expect("queued pending command");
                // Resolve/Receipt only: never replay the uncertain mutation.
                self.dispatch(target, id, pending.resolution());
            } else {
                self.first_send_checks
                    .insert(id, now + Duration::from_secs(5));
                // Reconnection only observes exact creation/submission identity.
                let status = self.status.clone();
                if let Err(error) = self.recover_draft(id) {
                    if self.active_draft == Some(id) {
                        self.status = format!(
                            "First-send recovery unavailable: {} · draft retained",
                            safe(&error.to_string())
                        );
                    }
                } else {
                    self.status = status;
                }
            }
        }
    }
}
