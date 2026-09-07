use super::*;

impl App {
    pub(super) fn update(&mut self, update: Update) {
        match update {
            Update::Terminals {
                target,
                incarnation,
                result,
            } => {
                if let Some(view) = self
                    .views
                    .get_mut(&target)
                    .filter(|v| v.process.incarnation == incarnation)
                {
                    view.terminals.update(result);
                }
            }
            Update::Control {
                target,
                incarnation,
                result,
            } => {
                if let Some(view) = self
                    .views
                    .get_mut(&target)
                    .filter(|view| view.process.incarnation == incarnation)
                {
                    match result {
                        Ok(value) => {
                            view.panel = Some(safe(&value));
                            view.scroll = 0;
                        }
                        Err(error) => view.error = Some(safe(&error)),
                    }
                }
            }
            Update::Catalogue { route, processes } => {
                for process in processes.into_iter().take(256) {
                    let target = Target {
                        route,
                        session: process.session_id,
                    };
                    if let Some(view) = self.views.get_mut(&target) {
                        if view.process.incarnation != process.incarnation {
                            view.snapshot = None;
                            view.terminals = Default::default();
                            view.rendered.take();
                            view.observed = None;
                            view.error = Some(
                                "The voyage reconnected. Check any unconfirmed action before trying again."
                                    .into(),
                            );
                        }
                        view.process = process;
                    } else {
                        let mut view = View::new(process);
                        if let Err(error) = drafts::load(&self.clients[route], &mut view) {
                            view.error = Some(format!("Draft recovery: {error}"));
                        }
                        self.views.insert(target, view);
                    }
                    if self.selected.is_none() {
                        self.selected = Some(target);
                    }
                }
            }
            Update::Snapshot {
                target,
                incarnation,
                result,
            } => {
                let Some(view) = self
                    .views
                    .get_mut(&target)
                    .filter(|v| v.process.incarnation == incarnation)
                else {
                    return;
                };
                match *result {
                    Ok(snapshot) if snapshot.session_id == target.session => {
                        if view
                            .snapshot
                            .as_ref()
                            .is_some_and(|old| old.revision > snapshot.revision)
                        {
                            return;
                        }
                        let changed = view
                            .snapshot
                            .as_ref()
                            .is_none_or(|old| old.revision != snapshot.revision);
                        view.unread |= changed && self.selected != Some(target);
                        if view.snapshot.as_ref() != Some(&snapshot) {
                            view.rendered.take();
                        }
                        view.snapshot = Some(snapshot);
                        view.observed = Some(Instant::now());
                        view.error = None;
                    }
                    Ok(_) => view.error = Some("Snapshot identity mismatch".into()),
                    Err(error) => view.error = Some(error),
                }
            }
            Update::RouteError { route, error } => {
                self.status = format!("{} unavailable: {}", self.route_label(route), safe(&error));
                for (target, view) in &mut self.views {
                    if target.route == route {
                        view.error =
                            Some("Connection unavailable; runtime state is unknown".into());
                    }
                }
            }
            Update::Created { route, result } => match result {
                Ok(process) => {
                    let target = Target {
                        route,
                        session: process.session_id,
                    };
                    self.views
                        .entry(target)
                        .or_insert_with(|| View::new(process));
                    self.selected = Some(target);
                    self.status = format!("New voyage ready on {}", self.route_label(route));
                }
                Err(error) => self.status = safe(&error),
            },
            Update::Command {
                target,
                command_id,
                refused,
                result,
            } => {
                let Some(view) = self.views.get_mut(&target) else {
                    return;
                };
                let Some(pending) = view.pending.as_ref().filter(|p| p.command_id == command_id)
                else {
                    return;
                };
                match result {
                    Ok(value) => {
                        if value.get("status").and_then(|status| status.as_str()) == Some("unknown")
                        {
                            self.status =
                                "Not confirmed yet. Your draft is saved. Press F4 to check again."
                                    .into();
                            return;
                        }
                        if value
                            .get("command_id")
                            .and_then(|id| id.as_str())
                            .is_some_and(|id| id != command_id.to_string())
                        {
                            self.status = "The response could not be matched. Your draft is saved; press F4 to check.".into();
                            return;
                        }
                        let rejected = value.get("status").and_then(|status| status.as_str())
                            == Some("rejected")
                            || (value["status"] == "transferred"
                                && matches!(
                                    value["original_status"].as_str(),
                                    Some("rejected" | "not_admitted")
                                ));
                        if !rejected
                            && !pending.preserve_draft
                            && !pending.draft.trim_start().starts_with('/')
                        {
                            view.history.record(&pending.draft);
                        }
                        if !rejected && !pending.preserve_draft && view.draft.text == pending.draft
                        {
                            view.draft.take();
                        }
                        view.pending = None;
                        self.status = super::presentation::receipt(&value);
                    }
                    Err(error) => {
                        self.status = format!("{} · draft retained", safe(&error));
                        if refused {
                            view.pending = None;
                        }
                        // A runtime/Vessel error can follow durable admission. Only
                        // a positive receipt resolves this command's identity.
                    }
                }
                if let Err(error) = drafts::save(&self.clients[target.route], view) {
                    self.status = format!("Draft persistence failed: {error}");
                }
            }
        }
    }
}
