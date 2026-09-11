//! Direct composer paste. Native acquisition never reads the executing Vessel's clipboard.
use super::*;
use anyhow::ensure;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use voyage_protocol::vessel::VoyageCommand;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Destination {
    Live(Target),
    Draft(Uuid),
}

pub(super) struct PendingPaste {
    pub(super) destination: Destination,
    id: Uuid,
    panel: Option<(super::right_panel::Editor, String)>,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    result: tokio::sync::oneshot::Receiver<Result<Prepared, String>>,
}

pub(super) enum Prepared {
    Images(Vec<attachments::Image>),
    Text(String),
    Empty,
}

fn raster_path(path: &std::path::Path) -> bool {
    path.extension().and_then(|x| x.to_str()).is_some_and(|x| {
        matches!(
            x.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp"
        )
    })
}

fn prepare_content(
    content: crate::clipboard::Content,
    fallback: Option<String>,
) -> Result<Prepared> {
    match content {
        crate::clipboard::Content::Image { name, bytes } => {
            Ok(Prepared::Images(vec![attachments::Image::from_bytes(
                name, &bytes,
            )?]))
        }
        crate::clipboard::Content::Files(paths) => {
            ensure!(
                !paths.is_empty() && paths.len() <= attachments::MAX_IMAGES,
                "Paste at most four image files"
            );
            if let Some(text) = fallback.as_ref()
                && paths.iter().any(|p| !p.exists())
            {
                return Ok(Prepared::Text(text.clone()));
            }
            ensure!(
                paths.iter().all(|p| raster_path(p)),
                "Clipboard files must be PNG, JPEG or WebP images"
            );
            let images = paths
                .iter()
                .map(|p| attachments::Image::from_path(p))
                .collect::<Result<Vec<_>>>()?;
            attachments::validate_set(&images)?;
            Ok(Prepared::Images(images))
        }
        crate::clipboard::Content::Text(text) => {
            let candidate_paths = crate::clipboard::pasted_paths(&text);
            let text = safe(&text.replace("\r\n", "\n").replace('\r', "\n"));
            ensure!(
                text.len() <= 65536,
                "Text paste exceeds 64 KiB; draft preserved"
            );
            if let Some(paths) = candidate_paths
                && paths.iter().all(|p| raster_path(p))
            {
                return prepare_content(crate::clipboard::Content::Files(paths), Some(text));
            }
            Ok(Prepared::Text(text))
        }
        crate::clipboard::Content::Empty => Ok(Prepared::Empty),
    }
}

impl App {
    pub(super) fn cancel_paste_for_private_panel(&self) {
        if let Some(pending) = &self.clipboard_pending {
            pending.cancel.cancel();
        }
    }
    fn paste_destination(&self) -> Option<Destination> {
        if self.vessels_open()
            || self.workspace_picker.is_some()
            || self.help
            || self.explore.is_some()
            || self.workflows_open()
            || self.operator.is_some()
            || self.operator_loading.is_some()
            || self.voyage_picker.is_some()
            || self.sidebar.menu.is_some()
            || self.interactions.borrow().focused
            || self.inference_picker_open()
        {
            return None;
        }
        if let Some(id) = self.active_draft {
            return Some(Destination::Draft(id));
        }
        let target = self.selected?;
        let view = self.views.get(&target)?;
        if view.panel.is_some() || view.terminals.open {
            return None;
        }
        Some(Destination::Live(target))
    }

    pub(super) fn ensure_paste_finished(&self, destination: Destination) -> Result<()> {
        ensure!(
            self.clipboard_pending
                .as_ref()
                .is_none_or(|p| p.destination != destination),
            "Clipboard paste is still being read; wait or press Esc to cancel before sending"
        );
        Ok(())
    }

    fn ensure_paste_editable(&self, destination: Destination) -> Result<()> {
        match destination {
            Destination::Live(target) => {
                let view = self
                    .views
                    .get(&target)
                    .context("Paste destination unavailable")?;
                ensure!(
                    view.pending.is_none(),
                    "Delivery pending; text and images are frozen"
                );
                ensure!(
                    !view.deleted() && !view.archived(),
                    "This voyage is not editable"
                );
            }
            Destination::Draft(id) => self.ensure_draft_images_editable(id)?,
        }
        Ok(())
    }

    fn paste_composer_mut(&mut self, destination: Destination) -> Option<&mut composer::Composer> {
        match destination {
            Destination::Live(target) => self.views.get_mut(&target).map(|v| &mut v.draft),
            Destination::Draft(id) => self.new_draft_composer_mut(id),
        }
    }

    fn copy_paste_draft(
        &self,
        destination: Destination,
    ) -> Result<(composer::Composer, Vec<attachments::Image>)> {
        match destination {
            Destination::Live(target) => {
                let view = self
                    .views
                    .get(&target)
                    .context("Paste destination unavailable")?;
                Ok((view.draft.clone(), view.images.clone()))
            }
            Destination::Draft(id) => self.copy_new_draft_images(id),
        }
    }

    fn retain_paste_draft(
        &mut self,
        destination: Destination,
        draft: composer::Composer,
        mut images: Vec<attachments::Image>,
    ) -> Result<()> {
        self.ensure_paste_editable(destination)?;
        attachments::sync_images(&draft, &mut images)?;
        match destination {
            Destination::Live(target) => {
                let view = self
                    .views
                    .get_mut(&target)
                    .context("Paste destination unavailable")?;
                let previous = (
                    std::mem::replace(&mut view.draft, draft),
                    std::mem::replace(&mut view.images, images),
                );
                if let Err(error) = drafts::save(&self.clients[target.route], view) {
                    (view.draft, view.images) = previous;
                    return Err(error.context("Paste could not be saved; previous draft preserved"));
                }
            }
            Destination::Draft(id) => self.retain_new_draft_images(id, draft, images)?,
        }
        Ok(())
    }

    /// Runs before general composer handling, but never steals input from a
    /// terminal, permission dialog, model picker or other focused editor.
    pub(super) fn paste_input(&mut self, event: &Event) -> Result<bool> {
        let Some(destination) = self.paste_destination() else {
            return Ok(false);
        };
        if matches!(event, Event::Key(key) if key.code == KeyCode::Esc && key.kind == crossterm::event::KeyEventKind::Press)
            && let Some(pending) = &self.clipboard_pending
        {
            pending.cancel.cancel();
            self.status = "Cancelling clipboard read; draft preserved".into();
            return Ok(true);
        }
        match event {
            Event::Key(key) if key.code == KeyCode::Char('v') || key.code == KeyCode::Char('V') => {
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                {
                    return Ok(false);
                }
                if key.kind != crossterm::event::KeyEventKind::Press {
                    return Ok(true);
                }
                self.begin_paste(destination, None)?;
                Ok(true)
            }
            Event::Key(key)
                if key.code == KeyCode::Insert && key.modifiers == KeyModifiers::SHIFT =>
            {
                if key.kind == crossterm::event::KeyEventKind::Press {
                    self.begin_paste(destination, None)?;
                }
                Ok(true)
            }
            Event::Paste(text) => {
                self.ensure_paste_editable(destination)?;
                let empty_event = text.is_empty();
                let candidate_paths = crate::clipboard::pasted_paths(text);
                let text = safe(&text.replace("\r\n", "\n").replace('\r', "\n"));
                ensure!(
                    text.len() <= 65536,
                    "Text paste exceeds 64 KiB; draft preserved"
                );
                if empty_event {
                    self.begin_paste(destination, None)?;
                    return Ok(true);
                }
                if let Some(paths) = candidate_paths
                    && paths.iter().all(|p| raster_path(p))
                {
                    self.begin_paste(
                        destination,
                        Some((crate::clipboard::Content::Files(paths), Some(text))),
                    )?;
                    return Ok(true);
                }
                // Ordinary terminal text paste does not consult the OS clipboard.
                let (mut draft, images) = self.copy_paste_draft(destination)?;
                ensure!(
                    draft.insertion_len(text.len()) <= 65536,
                    "Composer limit is 64 KiB; draft preserved"
                );
                draft.insert_str(&text);
                self.retain_paste_draft(destination, draft, images)?;
                self.sidebar.focus = sidebar::Focus::Composer;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn begin_paste(
        &mut self,
        destination: Destination,
        supplied: Option<(crate::clipboard::Content, Option<String>)>,
    ) -> Result<()> {
        self.ensure_paste_editable(destination)?;
        ensure!(
            !self.clipboard_blocked,
            "Clipboard helper cleanup was not confirmed; restart Helm before another clipboard read"
        );
        ensure!(
            self.clipboard_pending.is_none(),
            "A clipboard read is already in progress; Esc cancels it"
        );
        let id = Uuid::new_v4();
        let cancel = CancellationToken::new();
        self.paste_composer_mut(destination)
            .context("Paste destination unavailable")?
            .set_paste_anchor();
        let token = cancel.clone();
        let config = self.new_chat_config.clone();
        let (sender, result) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result: Result<Prepared> = async {
                let (content, fallback) = match supplied {
                    Some(value) => value,
                    None => (crate::clipboard::read(config, token.clone()).await?, None),
                };
                ensure!(!token.is_cancelled(), "Clipboard read cancelled");
                let prepared =
                    tokio::task::spawn_blocking(move || prepare_content(content, fallback))
                        .await??;
                ensure!(!token.is_cancelled(), "Clipboard read cancelled");
                Ok(prepared)
            }
            .await;
            let _ = sender.send(result.map_err(|e| e.to_string()));
        });
        self.clipboard_pending = Some(PendingPaste {
            destination,
            id,
            panel: None,
            cancel,
            task,
            result,
        });
        self.sidebar.focus = sidebar::Focus::Composer;
        self.status = "Reading paste… Esc cancels. You can keep typing.".into();
        Ok(())
    }

    pub(super) fn cancel_panel_paste_on_input(&self, event: &Event) {
        let changes_panel = matches!(event, Event::Key(_) | Event::Paste(_) | Event::Resize(_, _))
            || matches!(event, Event::Mouse(mouse) if matches!(mouse.kind, crossterm::event::MouseEventKind::Down(_)));
        if changes_panel
            && let Some(pending) = &self.clipboard_pending
            && pending.panel.is_some()
        {
            pending.cancel.cancel();
        }
    }

    pub(super) fn begin_panel_paste(&mut self) -> Result<()> {
        let (editor, before) = self.panel_editor().context("Text field is not editable")?;
        ensure!(
            !self.clipboard_blocked,
            "Clipboard helper cleanup unconfirmed; restart Helm"
        );
        ensure!(
            self.clipboard_pending.is_none(),
            "Clipboard read already in progress"
        );
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let config = self.new_chat_config.clone();
        let (sender, result) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let result: Result<Prepared> = async {
                let content = crate::clipboard::read(config, token.clone()).await?;
                ensure!(!token.is_cancelled(), "Clipboard read cancelled");
                match content {
                    crate::clipboard::Content::Text(text) => Ok(Prepared::Text(text)),
                    _ => anyhow::bail!("This field accepts clipboard text only"),
                }
            }
            .await;
            let _ = sender.send(result.map_err(|error| error.to_string()));
        });
        self.clipboard_pending = Some(PendingPaste {
            destination: Destination::Live(editor.target()),
            id: Uuid::new_v4(),
            panel: Some((editor, before)),
            cancel,
            task,
            result,
        });
        self.status = "Reading panel text… another action cancels the paste".into();
        Ok(())
    }

    pub(super) fn poll_clipboard(&mut self) {
        let Some(pending) = self.clipboard_pending.as_mut() else {
            return;
        };
        let result = match pending.result.try_recv() {
            Ok(result) => result,
            Err(tokio::sync::oneshot::error::TryRecvError::Empty) => return,
            Err(_) => Err("Clipboard reader cleanup failed: reader ended unexpectedly".into()),
        };
        let id = pending.id;
        self.paste_result(id, result);
    }

    pub(super) fn paste_result(&mut self, id: Uuid, result: Result<Prepared, String>) {
        if self.clipboard_pending.as_ref().is_none_or(|p| p.id != id) {
            return;
        }
        let pending = self.clipboard_pending.take().expect("matching paste");
        let destination = pending.destination;
        if let Some((editor, before)) = pending.panel {
            if result
                .as_ref()
                .is_err_and(|error| error.contains("cleanup failed"))
            {
                self.clipboard_blocked = true;
            }
            let applied = if pending.cancel.is_cancelled() {
                Err(anyhow::anyhow!(
                    "Clipboard paste cancelled; field preserved"
                ))
            } else {
                match result {
                    Ok(Prepared::Text(text)) => self.apply_panel_text(editor, &before, text),
                    Ok(_) => Err(anyhow::anyhow!("This field accepts text only")),
                    Err(error) => Err(anyhow::anyhow!(error)),
                }
            };
            self.status = match applied {
                Ok(()) => "Pasted into panel; review before confirming".into(),
                Err(error) => format!("Paste failed: {}", safe(&error.to_string())),
            };
            return;
        }
        let anchor = self
            .paste_composer_mut(destination)
            .and_then(|d| d.take_paste_range());
        if result
            .as_ref()
            .is_err_and(|error| error.contains("cleanup failed"))
        {
            self.clipboard_blocked = true;
            self.status = "Clipboard helper cleanup unconfirmed; draft preserved. Restart Helm before another paste.".into();
            return;
        }
        if pending.cancel.is_cancelled() {
            self.status = "Clipboard paste cancelled; draft preserved".into();
            return;
        }
        let applied: Result<()> = (|| {
            let (at, end) = anchor.context("Draft was replaced while reading; paste again")?;
            self.ensure_paste_editable(destination)?;
            let prepared = result.map_err(anyhow::Error::msg)?;
            let (mut draft, mut images) = self.copy_paste_draft(destination)?;
            match prepared {
                Prepared::Images(added) => {
                    draft.apply_paste_range(at, end, "");
                    attachments::sync_images(&draft, &mut images)?;
                    attachments::insert_images(&mut draft, &mut images, added, at)?
                }
                Prepared::Text(text) => {
                    ensure!(
                        draft
                            .text
                            .len()
                            .saturating_sub(end - at)
                            .saturating_add(text.len())
                            <= 65536,
                        "Composer limit is 64 KiB; paste not applied"
                    );
                    draft.apply_paste_range(at, end, &text);
                }
                Prepared::Empty => {
                    anyhow::bail!("Clipboard has no image or text to paste; draft preserved")
                }
            }
            self.retain_paste_draft(destination, draft, images)
        })();
        self.status = match applied {
            Ok(()) => {
                "Pasted into composer · Backspace/Delete removes image markers · Enter sends".into()
            }
            Err(error) => format!("Paste failed: {}", safe(&error.to_string())),
        };
    }

    pub(super) async fn finish_clipboard(&mut self) -> Result<()> {
        if let Some(mut pending) = self.clipboard_pending.take() {
            pending.cancel.cancel();
            if pending.panel.is_none()
                && let Some(composer) = self.paste_composer_mut(pending.destination)
            {
                composer.clear_paste_anchor();
            }
            // The clipboard reader reaps its helpers before completing.
            pending
                .task
                .await
                .context("Clipboard reader cleanup failed")?;
            if let Ok(Err(error)) = pending.result.try_recv() {
                ensure!(
                    !error.contains("cleanup failed"),
                    "Clipboard helper cleanup unconfirmed"
                );
            }
        }
        ensure!(
            !self.clipboard_blocked,
            "Clipboard helper cleanup unconfirmed; inspect local helper processes"
        );
        Ok(())
    }

    pub(super) fn send_image_turn(&mut self, target: Target) -> Result<()> {
        ensure!(
            self.clients.available(target.route),
            "Vessel unavailable · Ctrl+G to manage / retry; draft retained"
        );
        self.ensure_paste_finished(Destination::Live(target))?;
        self.ensure_paste_editable(Destination::Live(target))?;
        let view = self.views.get_mut(&target).context("Voyage unavailable")?;
        let snapshot = view
            .snapshot
            .as_ref()
            .context("Waiting for an authenticated snapshot")?;
        ensure!(
            !snapshot.run.as_ref().is_some_and(|r| r.active()),
            "Wait for the active run to finish before sending images; draft preserved"
        );
        let command_id = Uuid::new_v4();
        let command = attachments::prepare(
            VoyageCommand::Submit {
                coordination: None,
                command_id,
                expected_revision: snapshot.revision,
                expires_at_ms: super::super::frontend::deadline()?,
                prompt: view.draft.authored_text(),
            },
            &view.draft,
            &view.images,
        )?;
        view.pending = Some(state::Pending {
            command_id,
            incarnation: view.process.incarnation,
            draft: view.draft.text.clone(),
            preserve_draft: false,
            receipt_only: false,
            original: Some(Box::new(command.clone())),
        });
        if let Err(error) = drafts::save(&self.clients[target.route], view) {
            view.pending = None;
            return Err(error.context("Cannot persist image command; nothing sent"));
        }
        let mut transcript = view.transcript.borrow_mut();
        let parts = if let VoyageCommand::SubmitContent { content, .. } = &command {
            content.clone()
        } else {
            Vec::new()
        };
        transcript.delivery = Some(super::transcript::Delivery {
            text: view.draft.authored_text(),
            parts,
            before: view.snapshot.as_ref().map_or(0, |s| s.total_messages),
            label: "Sending…".into(),
        });
        transcript.dirty = true;
        drop(transcript);
        self.dispatch(target, command_id, command);
        Ok(())
    }
}
