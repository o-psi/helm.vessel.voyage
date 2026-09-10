use super::*;

impl App {
    pub(super) fn update(&mut self, update: Update) {
        let route = match &update {
            Update::Live { target, .. }
            | Update::History { target, .. }
            | Update::Completion { target, .. }
            | Update::Terminals { target, .. }
            | Update::Control { target, .. }
            | Update::Snapshot { target, .. }
            | Update::Command { target, .. } => Some(target.route),
            Update::FirstSend { route, .. }
            | Update::Catalogue { route, .. }
            | Update::RouteError { route, .. }
            | Update::RouteUnavailable { route, .. }
            | Update::Created { route, .. } => Some(*route),
            Update::InferenceModels { route, .. } => *route,
            _ => None,
        };
        if route.is_some_and(|route| !self.clients.current(route)) {
            return;
        }
        let background = match &update {
            Update::Command { target, .. } => {
                self.active_draft.is_some() || self.selected != Some(*target)
            }
            Update::FirstSend { saved, .. } => self.active_draft != Some(saved.id),
            _ => false,
        };
        let status = background.then(|| self.status.clone());
        self.apply_update(update);
        if let Some(status) = status {
            self.status = status;
        }
    }

    fn apply_update(&mut self, update: Update) {
        for view in self.views.values() {
            view.transcript.borrow_mut().dirty = true;
        }
        match update {
            Update::Coordination {
                origin,
                request,
                result,
            } => self.coordination_arrived(origin, request, result),
            Update::Vessels(event) => self.vessel_update(event),
            Update::DraftInferenceModels {
                id,
                provider,
                context,
                generation,
                models,
            } => self.draft_inference_models(id, provider, context, generation, models),
            Update::InferenceModels {
                route: _,
                id,
                context,
                generation,
                result,
            } => self.inference_models(id, context, generation, result),
            Update::FirstSend {
                route: _,
                saved,
                result,
            } => self.first_send_update(*saved, result),
            Update::Live {
                target,
                incarnation,
                run,
                offset,
                total,
                result,
            } => {
                if let Some(view) = self
                    .views
                    .get(&target)
                    .filter(|v| v.process.incarnation == incarnation)
                {
                    view.transcript.borrow_mut().live_loading = false;
                }
                if let Some(view) = self
                    .views
                    .get_mut(&target)
                    .filter(|v| v.process.incarnation == incarnation)
                    && view
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.run.as_ref())
                        .is_some_and(|r| {
                            r.run_id == run
                                && r.live_text_offset == Some(offset)
                                && r.partial_text_bytes == total
                        })
                {
                    let mut state = view.transcript.borrow_mut();
                    match result {
                        Ok(text) => state.live = Some((run, offset, total, text)),
                        Err(error) => state.error = Some(error),
                    }
                }
            }
            Update::History {
                target,
                incarnation,
                revision,
                result,
            } => {
                if let Some(view) = self
                    .views
                    .get_mut(&target)
                    .filter(|v| v.process.incarnation == incarnation)
                {
                    let mut state = view.transcript.borrow_mut();
                    if state.attempted == Some(revision) {
                        state.loading = false;
                    }
                    if view.snapshot.as_ref().is_some_and(|s| {
                        // Sender navigation may have installed complete history
                        // while an older suffix request was still in flight.
                        s.revision == revision
                            && !(state.loaded_revision == Some(revision)
                                && state.messages.len() == s.total_messages
                                && state.messages.first().is_none_or(|m| m.message_index == 0))
                    }) {
                        match result {
                            Ok(messages) => {
                                state.messages = messages;
                                state.loaded_revision = Some(revision);
                                state.error = None;
                            }
                            Err(error) => state.error = Some(error),
                        }
                    }
                }
            }
            Update::Completion {
                target,
                incarnation,
                section,
                value,
            } => self.completion_update(target, incarnation, section, value),
            Update::Terminals {
                target,
                incarnation,
                result,
                observed,
            } => {
                if let Some(view) = self
                    .views
                    .get_mut(&target)
                    .filter(|v| v.process.incarnation == incarnation)
                {
                    view.terminals.update(result, observed);
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
                            if self.selected == Some(target) {
                                self.status =
                                    "Overview ready. Esc returns to your conversation.".into();
                            }
                        }
                        Err(error) => {
                            view.error = Some(safe(&error));
                            if self.selected == Some(target) {
                                self.status =
                                    "Couldn't open this overview. Please try again.".into();
                            }
                        }
                    }
                }
            }
            Update::Catalogue { route, processes } => {
                self.clients.mark_available(route);
                self.vessel_state(route, vessels::ConnectionState::Connected);
                for process in processes.into_iter().take(256) {
                    if self.new_drafts.contains_key(&process.session_id) {
                        continue;
                    }
                    let target = Target {
                        route,
                        session: process.session_id,
                    };
                    if let Some(view) = self.views.get_mut(&target) {
                        view.connection_unavailable = false;
                        if view.process.incarnation != process.incarnation {
                            view.snapshot = None;
                            view.terminals = Default::default();
                            *view.transcript.borrow_mut() = Default::default();
                            view.rendered.take();
                            view.observed = None;
                            view.error = Some(
                                "The voyage reconnected. Helm checks unconfirmed actions automatically."
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
                    if let Some(receipt) = self.views[&target]
                        .process
                        .archive
                        .as_ref()
                        .map(|a| a.receipt.clone())
                        .or_else(|| self.views[&target].process.deletion.clone())
                    {
                        let view = self.views.get_mut(&target).expect("catalogued view");
                        if view.pending.as_ref().is_some_and(|p| {
                            p.incarnation == view.process.incarnation
                                && receipt["command_id"].as_str()
                                    == Some(p.command_id.to_string().as_str())
                        }) {
                            if view
                                .pending
                                .as_ref()
                                .is_some_and(|p| !p.preserve_draft && p.draft == view.draft.text)
                            {
                                view.draft.take();
                            }
                            view.pending = None;
                            if let Err(error) = drafts::save(&self.clients[route], view) {
                                self.status = format!(
                                    "Lifecycle action confirmed; draft persistence failed: {error}"
                                );
                            }
                        }
                    }
                }
                if self.selected.is_none() && self.active_draft.is_none() {
                    let targets = self.ordered_targets();
                    self.selected = targets
                        .iter()
                        .find(|t| {
                            self.views[t].process.state
                                == voyage_protocol::process::ProcessState::Live
                        })
                        .copied()
                        .or_else(|| targets.first().copied());
                }
            }
            Update::Snapshot {
                target,
                incarnation,
                result,
            } => {
                let recovering_route = self
                    .status
                    .starts_with(&format!("{} unavailable:", self.route_label(target.route)));
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
                        if view.snapshot.as_ref() != Some(&snapshot) {
                            view.rendered.take();
                        }
                        {
                            let mut transcript = view.transcript.borrow_mut();
                            if view
                                .snapshot
                                .as_ref()
                                .is_some_and(|old| old.total_messages > snapshot.total_messages)
                            {
                                *transcript = Default::default();
                            }
                            if transcript
                                .delivery
                                .as_ref()
                                .is_some_and(|d| snapshot.messages.iter().any(|m| d.matches(m)))
                            {
                                transcript.delivery = None;
                            }
                            transcript.observe_growth(view.snapshot.as_ref().is_some_and(|old| {
                                snapshot.total_messages > old.total_messages
                                    || snapshot.run.as_ref().is_some_and(|run| {
                                        old.run.as_ref().is_some_and(|previous| {
                                            previous.run_id == run.run_id
                                                && run.partial_text_bytes
                                                    > previous.partial_text_bytes
                                        })
                                    })
                            }));
                            transcript.merge_snapshot(&snapshot);
                            transcript.dirty = true;
                        }
                        view.snapshot = Some(snapshot);
                        if view.observe_settlement(chrono::Utc::now())
                            && let Err(error) = drafts::save(&self.clients[target.route], view)
                        {
                            self.status = format!("Saving voyage status timing failed: {error}");
                        }
                        view.observed = Some(Instant::now());
                        view.connection_unavailable = !self.clients.available(target.route);
                        view.error = view.connection_unavailable.then(||
                            "Vessel unavailable · Ctrl+G to manage / retry. Runtime state is unknown.".into());
                        if recovering_route && self.selected == Some(target) {
                            self.status = "Connected. Voyage state refreshed.".into();
                        }
                    }
                    Ok(_) => view.error = Some("Snapshot identity mismatch".into()),
                    Err(error) => view.error = Some(error),
                }
                self.sync_live_inference_picker(target);
            }
            Update::RouteUnavailable { route, error } => {
                self.clients.mark_unavailable(route);
                self.vessel_state(route, vessels::classify_error(&error));
                // Availability belongs to this Vessel, not the shared footer.
                // Preserve in-flight receipts and all cached conversation state.
                for (target, view) in &mut self.views {
                    if target.route == route {
                        view.connection_unavailable = true;
                        view.error = Some("Vessel unavailable · Ctrl+G to manage / retry. Runtime state is unknown.".into());
                    }
                }
            }
            Update::RouteError { route, error } => {
                self.vessel_state(route, vessels::classify_error(&error));
                // Stream failures are provisional; the next catalogue probe
                // determines Vessel availability without noisy transport diagnostics.
                for (target, view) in &mut self.views {
                    if target.route == route {
                        view.connection_unavailable = true;
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
                        .and_modify(|view| {
                            view.process = process.clone();
                            view.snapshot = None;
                            view.rendered.take();
                        })
                        .or_insert_with(|| View::new(process));
                    self.archives = false;
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
                self.command_checks.insert(
                    (target, command_id),
                    Some(Instant::now() + Duration::from_secs(5)),
                );
                let Some(view) = self.views.get_mut(&target) else {
                    return;
                };
                let Some(pending) = view.pending.as_ref().filter(|p| p.command_id == command_id)
                else {
                    return;
                };
                let inference = pending.original.as_deref().is_some_and(|command| {
                    matches!(
                        command,
                        voyage_protocol::vessel::VoyageCommand::SetInference { .. }
                    )
                });
                match result {
                    Ok(mut value) => {
                        // Steering dispatch wraps its journal record, while receipt
                        // lookup returns that record directly. Both identify the
                        // command through request.receipt_id.
                        if value["record"]["request"]["receipt_id"].is_string() {
                            value = value["record"].clone();
                        }
                        let receipt_id = value
                            .get("command_id")
                            .or_else(|| {
                                value
                                    .get("request")
                                    .and_then(|request| request.get("receipt_id"))
                            })
                            .and_then(|id| id.as_str());
                        if value.get("status").and_then(|status| status.as_str()) == Some("unknown")
                        {
                            self.status =
                                "Not confirmed yet. Your draft is saved; Helm checks automatically."
                                    .into();
                            return;
                        }
                        if receipt_id != Some(command_id.to_string().as_str())
                            || value["status"].as_str().is_none_or(str::is_empty)
                        {
                            self.status = "The response could not be matched. Your draft is saved; Helm checks automatically.".into();
                            return;
                        }
                        let inference_status = if value["status"] == "transferred" {
                            value["original_status"].as_str()
                        } else {
                            value["status"].as_str()
                        };
                        if inference
                            && !matches!(
                                inference_status,
                                Some(
                                    "applied"
                                        | "completed"
                                        | "rejected"
                                        | "not_admitted"
                                        | "failed"
                                )
                            )
                        {
                            self.status =
                                "Inference pending, not yet applied · text preserved · Checking automatically"
                                    .into();
                            return;
                        }
                        let rejected = (inference && inference_status == Some("failed"))
                            || matches!(
                                value["status"].as_str(),
                                Some("rejected" | "not_admitted")
                            )
                            || (value["status"] == "transferred"
                                && matches!(
                                    value["original_status"].as_str(),
                                    Some("rejected" | "not_admitted")
                                ));
                        if rejected
                            && !pending.preserve_draft
                            && view.images.is_empty()
                            && (view.draft.text.is_empty() || view.draft.text.trim() == "/receipt")
                        {
                            view.draft.set_text(pending.draft.clone());
                        }
                        if !rejected && !pending.preserve_draft {
                            if let Some(voyage_protocol::vessel::VoyageCommand::SubmitContent {
                                content,
                                ..
                            }) = pending.original.as_deref()
                            {
                                view.history.record(&voyage_runtime::images::text(content));
                            } else if !pending.draft.trim_start().starts_with('/') {
                                view.history.record(&pending.draft);
                            }
                        }
                        let same_image_draft = super::attachments::pending_matches(pending, view);
                        if !rejected
                            && !pending.preserve_draft
                            && (same_image_draft
                                || (pending
                                    .original
                                    .as_deref()
                                    .is_none_or(|c| !super::attachments::is_image_submission(c))
                                    && view.images.is_empty()
                                    && view.draft.text == pending.draft))
                        {
                            view.draft.take();
                        }
                        if !rejected && !pending.preserve_draft && same_image_draft {
                            view.images.clear();
                        }
                        if !rejected && let Some(archived) = value["archived"].as_bool() {
                            if let Some(snapshot) = view.snapshot.as_mut() {
                                snapshot.lifecycle["archived"] = archived.into();
                            }
                            view.rendered.take();
                            if !archived
                                && self.active_draft.is_none()
                                && self.selected == Some(target)
                            {
                                self.archives = false;
                                self.selected = Some(target);
                            }
                        }
                        if let Some(delivery) = &mut view.transcript.borrow_mut().delivery {
                            delivery.label = if rejected {
                                "Not accepted · Draft kept"
                            } else {
                                "Received · Waiting for saved conversation"
                            }
                            .into();
                        }
                        view.pending = None;
                        self.status = if inference {
                            format!(
                                "Inference {} · {}",
                                if rejected {
                                    "rejected; text preserved"
                                } else {
                                    "applied to saved / next-turn settings; current turn unchanged"
                                },
                                super::presentation::receipt(&value)
                            )
                        } else if value["archived"] == true {
                            "Archived. Waiting for confirmed cleanup before releasing the process slot. F5 opens archives.".into()
                        } else {
                            super::presentation::receipt(&value)
                        };
                    }
                    Err(error) => {
                        if let Some(delivery) = &mut view.transcript.borrow_mut().delivery {
                            delivery.label = if refused {
                                "Not sent · Draft kept"
                            } else {
                                "Delivery unconfirmed · Checking automatically"
                            }
                            .into();
                        }
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
        if self.selected.is_some_and(|target| {
            self.views.get(&target).is_some_and(|v| {
                (v.deleted() || v.archived() != self.archives) && v.pending.is_none()
            })
        }) {
            self.selected = self.ordered_targets().first().copied();
        }
    }
}
