use super::*;

impl App {
    pub(super) fn input(&mut self, event: Event) -> Result<()> {
        match &event {
            Event::Mouse(mouse) => self.sidebar.pointer = Some((mouse.column, mouse.row).into()),
            Event::FocusLost => self.sidebar.pointer = None,
            _ => {}
        }
        // Motion remains passive even while a modal owns input.
        if matches!(&event, Event::Mouse(mouse) if mouse.kind == crossterm::event::MouseEventKind::Moved)
        {
            return Ok(());
        }
        if self.inference_input(&event)? {
            return Ok(());
        }
        self.sync_interactions();
        if self.new_draft_input(&event)? {
            return Ok(());
        }
        let global = matches!(&event, Event::Key(key) if key.code == KeyCode::F(4) || (key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q'))));
        if self.interactions.borrow().focused && !global {
            self.interaction_input(&event)?;
            return Ok(());
        }
        if self.transcript_input(&event) {
            return Ok(());
        }
        if matches!(event, Event::Resize(..)) {
            self.sidebar.hits.borrow_mut().clear();
            self.sidebar.visible.set(None);
        }
        if let Event::Key(key) = &event {
            if key.kind == crossterm::event::KeyEventKind::Release {
                return Ok(());
            }
            if self.sidebar.menu.is_some() && self.sidebar_input(&event)? {
                return Ok(());
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('c' | 'q') => {
                        self.quit = true;
                        return Ok(());
                    }
                    KeyCode::Char('n') => return self.create(None),
                    _ => {}
                }
            }
            if self.sidebar_input(&event)? {
                return Ok(());
            }
            if self.explore_input(key)? {
                return Ok(());
            }
            if key.code == KeyCode::F(1) {
                self.help = !self.help;
                self.help_scroll = 0;
                return Ok(());
            }
            if self.help {
                match key.code {
                    KeyCode::Esc => self.help = false,
                    KeyCode::PageUp | KeyCode::Up => {
                        self.help_scroll = self.help_scroll.saturating_sub(10)
                    }
                    KeyCode::PageDown | KeyCode::Down => {
                        self.help_scroll = self.help_scroll.saturating_add(10)
                    }
                    _ => {}
                }
                return Ok(());
            }
            if key.code == KeyCode::F(5) {
                self.show_archives(!self.archives);
                return Ok(());
            }
            if key.code == KeyCode::F(4) {
                if let Some(target) = self.selected {
                    if let Some(pending) = self.views.get(&target).and_then(|v| v.pending.clone()) {
                        self.dispatch(target, pending.command_id, pending.resolution());
                    } else {
                        self.status = "All sent messages are accounted for.".into();
                    }
                }
                return Ok(());
            }
            if self.completion_input(key)? {
                return Ok(());
            }
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                let keys = self.ordered_targets();
                if !keys.is_empty() {
                    let index = self
                        .selected
                        .and_then(|selected| keys.iter().position(|key| *key == selected))
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::BackTab {
                        (index + keys.len() - 1) % keys.len()
                    } else {
                        (index + 1) % keys.len()
                    };
                    self.selected = Some(keys[next]);
                    self.sidebar.focus = sidebar::Focus::Voyages;
                    let view = self.views.get_mut(&keys[next]).expect("known view");
                    view.unread = false;
                    view.terminals.clear_displayed();
                }
                return Ok(());
            }
            if self.terminal_input(&event)? {
                return Ok(());
            }
            if let Some(view) = self.selected.and_then(|target| self.views.get_mut(&target))
                && view.panel.is_some()
            {
                match key.code {
                    KeyCode::Esc => {
                        view.panel = None;
                        view.scroll = 0;
                    }
                    KeyCode::PageUp | KeyCode::Up => view.scroll = view.scroll.saturating_sub(10),
                    KeyCode::PageDown | KeyCode::Down => {
                        view.scroll = view.scroll.saturating_add(10)
                    }
                    _ => {}
                }
                return Ok(());
            }
            if self.interaction_input(&event)? {
                return Ok(());
            }
            if key.code == KeyCode::Enter
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT)
            {
                return self.send();
            }
        }
        if !matches!(event, Event::Key(_)) && self.sidebar_input(&event)? {
            return Ok(());
        }
        if self.help || self.explore.is_some() {
            return Ok(());
        }
        if !matches!(event, Event::Key(_))
            && (self.terminal_input(&event)? || self.interaction_input(&event)?)
        {
            return Ok(());
        }
        let Some(target) = self.selected else {
            return Ok(());
        };
        let view = self
            .views
            .get_mut(&target)
            .context("selected voyage unavailable")?;
        if view.panel.is_some() {
            return Ok(());
        }
        match event {
            Event::Paste(text) => {
                let text = safe(&text);
                anyhow::ensure!(
                    view.draft.text.len().saturating_add(text.len()) <= 64 * 1024,
                    "draft limit is 64 KiB"
                );
                view.draft.insert_str(&text);
            }
            Event::Key(key) => match key.code {
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    anyhow::ensure!(
                        view.draft.text.len() + ch.len_utf8() <= 64 * 1024,
                        "draft limit is 64 KiB"
                    );
                    view.draft.insert(ch);
                }
                KeyCode::Enter => view.draft.insert('\n'),
                KeyCode::Backspace => view.draft.backspace(),
                KeyCode::Delete => view.draft.delete(),
                KeyCode::Home => view.draft.line_start(),
                KeyCode::End => view.draft.line_end(),
                KeyCode::Left => {
                    view.draft.cursor = view.draft.text[..view.draft.cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(index, _)| index)
                }
                KeyCode::Right => {
                    if let Some(ch) = view.draft.text[view.draft.cursor..].chars().next() {
                        view.draft.cursor += ch.len_utf8();
                    }
                }
                KeyCode::Up => view.history.navigate(&mut view.draft, true),
                KeyCode::Down => view.history.navigate(&mut view.draft, false),
                KeyCode::PageUp => view.scroll = view.scroll.saturating_add(10),
                KeyCode::PageDown => view.scroll = view.scroll.saturating_sub(10),
                _ => return Ok(()),
            },
            Event::Mouse(mouse) => match mouse.kind {
                crossterm::event::MouseEventKind::ScrollUp => {
                    view.scroll = view.scroll.saturating_add(3)
                }
                crossterm::event::MouseEventKind::ScrollDown => {
                    view.scroll = view.scroll.saturating_sub(3)
                }
                _ => return Ok(()),
            },
            _ => return Ok(()),
        }
        drafts::save(&self.clients[target.route], view)
    }
}
