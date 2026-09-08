use super::*;

impl App {
    pub(in crate::process_client::ui) fn new_draft_input(&mut self, event: &Event) -> Result<bool> {
        if let Event::Mouse(mouse) = event
            && mouse.kind
                == crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left)
        {
            let id = self
                .draft_hits
                .borrow()
                .iter()
                .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
                .map(|(_, id)| *id);
            if let Some(id) = id {
                self.select_draft(id);
                return Ok(true);
            }
            if self.active_draft.is_some() {
                let _ = self.sidebar_input(event)?;
                if self.selected.is_some() {
                    self.active_draft = None;
                    return Ok(true);
                }
            }
        }
        let Some(id) = self.active_draft else {
            return Ok(false);
        };
        let key = match event {
            Event::Key(key) => Some(key),
            _ => None,
        };
        if let Some(key) = key {
            if key.kind == crossterm::event::KeyEventKind::Release {
                return Ok(true);
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c' | 'q') => self.quit = true,
                    KeyCode::Char('n') => {
                        self.create(None)?;
                    }
                    _ => {}
                }
                return Ok(true);
            }
            if key.code == KeyCode::F(1) {
                self.status = "Draft commands: /model [NAME], /thinking [VALUE], /service [VALUE], /access MODE, /workspace PATH, /new [PATH], /discard. Enter sends; Tab changes view. First sends recover automatically.".into();
                return Ok(true);
            }
            if key.code == KeyCode::F(5) {
                self.active_draft = None;
                self.show_archives(!self.archives);
                return Ok(true);
            }
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc) {
                let ids: Vec<_> = self.new_drafts.keys().copied().collect();
                let index = ids.iter().position(|i| *i == id).unwrap_or(0);
                let next = if key.code == KeyCode::BackTab {
                    index.checked_sub(1)
                } else {
                    Some(index + 1).filter(|i| *i < ids.len())
                };
                if key.code != KeyCode::Esc
                    && let Some(next) = next
                {
                    self.select_draft(ids[next]);
                } else if let Some(target) = self.ordered_targets().first().copied() {
                    self.active_draft = None;
                    self.selected = Some(target);
                } else {
                    self.select_draft(ids[0]);
                }
                return Ok(true);
            }
            if key.code == KeyCode::Enter
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            {
                let text = self.new_drafts[&id].composer.text.clone();
                if text.trim_start().starts_with('/') {
                    self.draft_command(id, text.trim())?;
                } else {
                    self.send_new_draft()?;
                }
                return Ok(true);
            }
        }
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        if draft.saved.start.is_some() || draft.busy {
            self.status = "First send pending. Text and settings are frozen; Helm continues setup and checks delivery automatically.".into();
            return Ok(true);
        }
        match event {
            Event::Paste(text) => {
                let text = safe(text);
                anyhow::ensure!(
                    draft.composer.text.len().saturating_add(text.len()) <= 65536,
                    "draft limit is 64 KiB"
                );
                draft.composer.insert_str(&text);
            }
            Event::Key(key) => match key.code {
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL) =>
                {
                    anyhow::ensure!(
                        draft.composer.text.len() + ch.len_utf8() <= 65536,
                        "draft limit is 64 KiB"
                    );
                    draft.composer.insert(ch);
                }
                KeyCode::Enter => {
                    anyhow::ensure!(draft.composer.text.len() < 65536, "draft limit is 64 KiB");
                    draft.composer.insert('\n');
                }
                KeyCode::Backspace => draft.composer.backspace(),
                KeyCode::Delete => draft.composer.delete(),
                KeyCode::Home => draft.composer.line_start(),
                KeyCode::End => draft.composer.line_end(),
                KeyCode::Left => {
                    draft.composer.cursor = draft.composer.text[..draft.composer.cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i)
                }
                KeyCode::Right => {
                    if let Some(ch) = draft.composer.text[draft.composer.cursor..].chars().next() {
                        draft.composer.cursor += ch.len_utf8();
                    }
                }
                _ => return Ok(true),
            },
            _ => return Ok(true),
        }
        draft.saved.text = draft.composer.text.clone();
        storage::save(&draft.saved)?;
        Ok(true)
    }

    pub(in crate::process_client::ui) fn draft_command(
        &mut self,
        id: Uuid,
        text: &str,
    ) -> Result<()> {
        if super::super::inference::parse(text).is_some() {
            return self.inference_command(
                super::super::inference::Destination::Draft(id),
                text,
                false,
            );
        }
        if text == "/quit" {
            self.quit = true;
            return Ok(());
        }
        if text == "/help" {
            self.status = "Draft commands: /model [NAME], /thinking [VALUE], /service [VALUE], /access read-only|approval|unrestricted, /workspace PATH, /new [PATH], /discard. Enter sends; Tab changes view. First sends recover automatically.".into();
        } else {
            let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
            anyhow::ensure!(
                draft.saved.start.is_none() && !draft.busy,
                "Waiting for first-send confirmation before changing this draft"
            );
            if text == "/discard" {
                draft.saved.finished = true;
                draft.saved.text.clear();
                draft.saved.images.clear();
                storage::save(&draft.saved)?;
                self.new_drafts.remove(&id);
                self.active_draft = None;
                self.status = "Local draft discarded.".into();
                return Ok(());
            }
            if let Some(access) = text.strip_prefix("/access ") {
                let config = draft
                    .saved
                    .config
                    .as_mut()
                    .context("Remote drafts use executing-host policy")?;
                config.access = Some(match access.trim() {
                    "read-only" => crate::config::AccessMode::ReadOnly,
                    "approval" => crate::config::AccessMode::Approval,
                    "unrestricted" => crate::config::AccessMode::Unrestricted,
                    _ => anyhow::bail!("use /access read-only, approval or unrestricted"),
                });
                draft.saved.explicit.access = config.access;
            } else if let Some(workspace) = text.strip_prefix("/workspace ") {
                let path = PathBuf::from(workspace);
                anyhow::ensure!(
                    path.is_absolute(),
                    "workspace must be absolute on the executing host"
                );
                draft.saved.workspace = path;
            } else if text != "/help" && text != "/new" && !text.starts_with("/new ") {
                anyhow::bail!(
                    "This is a local draft. /help lists commands available before first send"
                );
            }
            self.status = "Draft settings saved. Send a message to start the voyage.".into();
        }
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        draft.composer.take();
        draft.saved.text.clear();
        storage::save(&draft.saved)?;
        if text == "/new" || text.starts_with("/new ") {
            self.create(text.strip_prefix("/new "))?;
        }
        Ok(())
    }
}

impl App {
    pub(in crate::process_client::ui) fn draft_access(&self, id: Uuid) -> Result<String> {
        let draft = self.new_drafts.get(&id).context("draft unavailable")?;
        let config = draft
            .saved
            .config
            .as_ref()
            .context("Remote drafts use executing-host policy")?;
        Ok(config.access_mode().to_string())
    }

    pub(in crate::process_client::ui) fn set_draft_access(
        &mut self,
        id: Uuid,
        mode: crate::config::AccessMode,
    ) -> Result<()> {
        let draft = self.new_drafts.get_mut(&id).context("draft unavailable")?;
        anyhow::ensure!(
            draft.saved.start.is_none() && !draft.busy,
            "Waiting for first-send confirmation before changing this draft"
        );
        let mut saved = draft.saved.clone();
        let config = saved
            .config
            .as_mut()
            .context("Remote drafts use executing-host policy")?;
        config.access = Some(mode);
        saved.explicit.access = Some(mode);
        storage::save(&saved)?;
        draft.saved = saved;
        self.status = "Draft access saved · message preserved".into();
        Ok(())
    }
}
