use super::*;
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};

impl Manager {
    pub(super) fn input(
        &mut self,
        event: &TerminalEvent,
        sender: &mpsc::Sender<super::super::observe::Update>,
    ) -> Vec<Action> {
        if !self.panel.open {
            return Vec::new();
        }
        self.sync_focus();
        if matches!(event, TerminalEvent::Resize(..)) {
            self.panel.hits.clear();
            self.panel.reveal_selection = true;
            return Vec::new();
        }
        let button = match event {
            TerminalEvent::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => self
                    .panel
                    .hits
                    .iter()
                    .find(|(r, _)| r.contains((mouse.column, mouse.row).into()))
                    .map(|(_, b)| *b),
                MouseEventKind::ScrollDown => {
                    self.panel.scroll = self.panel.scroll.saturating_add(3);
                    None
                }
                MouseEventKind::ScrollUp => {
                    self.panel.scroll = self.panel.scroll.saturating_sub(3);
                    None
                }
                _ => None,
            },
            TerminalEvent::Paste(value) => {
                self.insert(value);
                None
            }
            TerminalEvent::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc {
                    if matches!(self.panel.page, Page::List) || self.panel.busy.is_some() {
                        self.panel.open = false;
                        self.clear_fields();
                    } else {
                        self.back();
                    }
                    return Vec::new();
                }
                match key.code {
                    KeyCode::PageDown => {
                        self.panel.scroll = self.panel.scroll.saturating_add(8);
                        None
                    }
                    KeyCode::PageUp => {
                        self.panel.scroll = self.panel.scroll.saturating_sub(8);
                        None
                    }
                    KeyCode::Tab | KeyCode::BackTab if !self.panel.focus.is_empty() => {
                        if key.code == KeyCode::BackTab {
                            self.panel.focus.prev();
                        } else {
                            self.panel.focus.next();
                        }
                        self.sync_focus();
                        self.panel.reveal_selection = true;
                        None
                    }
                    KeyCode::Backspace if self.editing() => {
                        self.panel.fields[self.panel.field].pop();
                        None
                    }
                    KeyCode::Char('u')
                        if key.modifiers.contains(KeyModifiers::CONTROL) && self.editing() =>
                    {
                        self.panel.fields[self.panel.field] = Zeroizing::new(String::new());
                        None
                    }
                    KeyCode::Char(c)
                        if self.editing()
                            && !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        self.insert(&c.to_string());
                        None
                    }
                    KeyCode::Char(' ') if !self.editing() && !self.panel.focus.is_empty() => {
                        self.panel.focus.current().copied()
                    }
                    KeyCode::Enter if !self.panel.focus.is_empty() => {
                        self.panel.focus.current().map(|button| match button {
                            Button::Field(_) => Button::Submit,
                            button => *button,
                        })
                    }
                    KeyCode::Enter => Some(match self.panel.page {
                        Page::List if self.panel.selected > self.records.len() => Button::Resume,
                        Page::List => Button::Connect,
                        Page::Form { .. } | Page::Rename(_) => Button::Submit,
                        Page::Preview { .. } | Page::Forget(_) => Button::Save,
                    }),
                    KeyCode::Up if matches!(self.panel.page, Page::List) => {
                        self.panel.selected = self.panel.selected.saturating_sub(1);
                        self.panel.reveal_selection = true;
                        None
                    }
                    KeyCode::Down if matches!(self.panel.page, Page::List) => {
                        self.panel.selected =
                            (self.panel.selected + 1).min(self.records.len() + self.pending.len());
                        self.panel.reveal_selection = true;
                        None
                    }
                    KeyCode::Char('w') => Some(Button::New),
                    KeyCode::Char('v') => Some(Button::Filter),
                    KeyCode::Char('l') => Some(Button::All),
                    KeyCode::Char('a') => Some(Button::Add),
                    KeyCode::Char('i') => Some(Button::Import),
                    KeyCode::Char('c') => Some(Button::Connect),
                    KeyCode::Char('u') => Some(Button::RetryUnavailable),
                    KeyCode::Char('d') => Some(Button::Disconnect),
                    KeyCode::Char('r') => Some(Button::Rename),
                    KeyCode::Char('n') => Some(Button::Replace),
                    KeyCode::Char('t') => Some(Button::Auto),
                    KeyCode::Char('f') => Some(Button::Forget),
                    KeyCode::Char('o') => Some(Button::Restore),
                    KeyCode::Char('p') => Some(Button::Resume),
                    KeyCode::Char('s') => Some(Button::Save),
                    _ => None,
                }
            }
            _ => None,
        };
        let actions = button.map(|b| self.press(b, sender)).unwrap_or_default();
        // Areas belong to the rendered page, not to its successor or old scroll.
        self.panel.hits.clear();
        actions
    }
    fn editing(&self) -> bool {
        self.panel.busy.is_none()
            && matches!(self.panel.page, Page::Form { .. } | Page::Rename(_))
            && matches!(self.panel.focus.current(), Some(Button::Field(_)))
    }
    fn insert(&mut self, value: &str) {
        if !self.editing() {
            return;
        }
        let field = &mut self.panel.fields[self.panel.field];
        for c in value.chars().filter(|c| !c.is_control()) {
            if field.len() + c.len_utf8() > 8192 {
                break;
            }
            field.push(c);
        }
    }
    fn press(
        &mut self,
        button: Button,
        sender: &mpsc::Sender<super::super::observe::Update>,
    ) -> Vec<Action> {
        if matches!(button, Button::Back) {
            if self.panel.busy.is_some() || matches!(self.panel.page, Page::List) {
                self.panel.open = false;
                self.clear_fields();
            } else {
                self.back();
            }
            return Vec::new();
        }
        if self.panel.busy.is_some() {
            return Vec::new();
        }
        if !matches!(self.panel.page, Page::List) && !self.panel.focus.elements().contains(&button)
        {
            return Vec::new();
        }
        self.focus_control(button);
        match button {
            Button::New if matches!(self.panel.page, Page::List) => {
                let id = self.selected().map(|c| c.id);
                self.panel.open = false;
                return vec![Action::New(id)];
            }
            Button::Filter | Button::All if matches!(self.panel.page, Page::List) => {
                let id = if matches!(button, Button::All) {
                    None
                } else {
                    self.selected().map(|c| c.id)
                };
                self.panel.open = false;
                return vec![if matches!(button, Button::Filter) && id.is_none() {
                    Action::FilterLocal
                } else {
                    Action::Filter(id)
                }];
            }
            Button::RetryUnavailable if matches!(self.panel.page, Page::List) => {
                return vec![Action::RetryUnavailable];
            }
            Button::Select(i) => self.panel.selected = i,
            Button::Field(_) => {}
            Button::Add | Button::Import | Button::Replace
                if matches!(self.panel.page, Page::List) =>
            {
                let replacement = if matches!(button, Button::Replace) {
                    self.selected().map(|c| c.id)
                } else {
                    None
                };
                if matches!(button, Button::Replace) && replacement.is_none() {
                    return Vec::new();
                }
                self.clear_fields();
                self.panel.autoconnect = true;
                self.panel.scroll = 0;
                self.panel.page = Page::Form {
                    import: matches!(button, Button::Import),
                    replacement,
                };
                self.panel.notice = "Private setup only. Obtain an invitation from the executing account owner. Import never broadens authority.".into();
            }
            Button::Import if matches!(self.panel.page, Page::Form { .. }) => {
                if let Page::Form { import, .. } = &mut self.panel.page {
                    *import = !*import;
                }
                self.clear_fields();
            }
            Button::Connect if matches!(self.panel.page, Page::List) => {
                if self.panel.selected == 0 {
                    return vec![Action::ConnectLocal];
                }
                if let Some(c) = self.selected() {
                    if c.forgotten {
                        self.panel.notice = "Restore this original connection first (o). Recovery credentials are retained; no commands will be replayed.".into();
                    } else {
                        return vec![Action::Activate(Box::new(c.clone()))];
                    }
                }
            }
            Button::Disconnect if matches!(self.panel.page, Page::List) => {
                if self.panel.selected == 0 {
                    return vec![Action::DisconnectLocal];
                }
                if let Some(c) = self.selected() {
                    return vec![Action::Disconnect(c.id)];
                }
            }
            Button::Restore if matches!(self.panel.page, Page::List) => {
                if let Some(c) = self.selected().filter(|c| c.forgotten) {
                    match self.registry.restore(c.id, c.revision) {
                        Ok(_) => { let _ = self.reload(); self.panel.notice = "Original connection restored with automatic reconnect off. Connect explicitly to observe retained receipts; nothing is replayed.".into(); }
                        Err(_) => self.panel.notice = "Restore failed. Original credential retained; check permissions or reload after a concurrent change.".into(),
                    }
                }
            }
            Button::Rename if matches!(self.panel.page, Page::List) => {
                if let Some(c) = self.selected() {
                    let (id, alias) = (c.id, c.alias.clone());
                    self.clear_fields();
                    *self.panel.fields[0] = alias;
                    self.panel.page = Page::Rename(id);
                    self.panel.scroll = 0;
                }
            }
            Button::Auto => {
                if matches!(self.panel.page, Page::List) {
                    if let Some(c) = self.selected() {
                        self.preferences(c.id, None, true);
                    }
                } else {
                    self.panel.autoconnect = !self.panel.autoconnect;
                }
            }
            Button::Forget if matches!(self.panel.page, Page::List) => {
                if let Some(c) = self.selected() {
                    self.panel.page = Page::Forget(c.id);
                    self.panel.scroll = 0;
                }
            }
            Button::Resume if matches!(self.panel.page, Page::List) => {
                if let Some(i) = self.panel.selected.checked_sub(self.records.len() + 1)
                    && let Some(id) = self.pending.get(i).copied()
                {
                    self.prepare(sender, Some(id));
                }
            }
            Button::Submit => {
                if let Page::Rename(id) = self.panel.page {
                    let alias = self.panel.fields[0].trim().to_owned();
                    self.preferences(id, Some(alias), false);
                    self.back();
                } else {
                    self.prepare(sender, None);
                }
            }
            Button::Save => {
                if let Page::Preview {
                    preview,
                    replacement,
                } = &self.panel.page
                {
                    // A replacement must get a new immutable registry identity. Never repin old routes.
                    if replacement.is_some_and(|id| id == preview.connection.id) {
                        self.panel.notice =
                            "Replacement refused: new access must have a new connection identity."
                                .into();
                        return Vec::new();
                    }
                    let result = self.registry.save_preview(
                        preview,
                        self.panel.fields[2].trim().to_owned(),
                        self.panel.autoconnect,
                    );
                    match result {
                        Ok(c) => { self.back(); let _ = self.reload(); self.panel.notice = "Saved. Old access, drafts and pending deliveries remain separate; forget old access explicitly if desired.".into(); return vec![Action::Activate(Box::new(c))]; }
                        Err(_) => self.panel.notice = "Not saved. Check private storage, duplicate access or a concurrent registry change; review again.".into(),
                    }
                } else if let Page::Forget(id) = self.panel.page
                    && let Some(c) = self.records.iter().find(|c| c.id == id)
                {
                    match self.registry.forget(id, c.revision) {
                        Ok(_) => {
                            self.back();
                            let _ = self.reload();
                            self.panel.notice = "Forgotten locally. Remote access is not revoked; drafts and pending deliveries are retained.".into();
                            return vec![Action::Forget(id)];
                        }
                        Err(_) => {
                            self.panel.notice = "Not forgotten: reload after a concurrent change or storage failure.".into();
                            let _ = self.reload();
                        }
                    }
                }
            }
            _ => (),
        }
        Vec::new()
    }
}
