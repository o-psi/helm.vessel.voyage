use super::*;
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEventKind};

impl App {
    pub(in crate::process_client::ui) fn sidebar_input(&mut self, event: &Event) -> Result<bool> {
        if self.help || self.explore.is_some() {
            return Ok(false);
        }
        if let Some(mut menu) = self.sidebar.menu.take() {
            let result = self.action_menu_input(&mut menu, event);
            match result {
                Ok(true) => {}
                Ok(false) => self.sidebar.menu = Some(menu),
                Err(error) => {
                    menu.error = error.to_string();
                    self.sidebar.menu = Some(menu);
                }
            }
            return Ok(true);
        }
        if let Event::Mouse(mouse) = event {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let hit = self
                    .sidebar
                    .hits
                    .borrow()
                    .iter()
                    .find(|h| h.area.contains((mouse.column, mouse.row).into()))
                    .copied();
                if let Some(hit) = hit {
                    if self.views.get(&hit.target).is_some_and(|v| {
                        v.process.incarnation == hit.incarnation && v.archived() == self.archives
                    }) {
                        self.sidebar_select(hit.target);
                        if hit.button.contains((mouse.column, mouse.row).into()) {
                            self.open_actions(hit.target);
                        }
                    }
                    return Ok(true);
                }
                self.sidebar.focus = Focus::Composer;
            }
            return Ok(false);
        }
        let Event::Key(key) = event else {
            if matches!(event, Event::Paste(_)) {
                self.sidebar_composer();
            }
            return Ok(false);
        };
        if key.code == KeyCode::F(9) {
            if let Some(target) = self.selected {
                self.open_actions(target);
            }
            return Ok(true);
        }
        if self.sidebar.focus == Focus::Composer
            && (self.interactions.borrow().focused
                || self
                    .selected
                    .and_then(|t| self.views.get(&t))
                    .is_some_and(|v| v.terminals.open || v.panel.is_some()))
        {
            return Ok(false);
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return Ok(false);
        }
        if self.sidebar.focus == Focus::Composer
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && matches!(
                key.code,
                KeyCode::Left
                    | KeyCode::Right
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
            )
        {
            return Ok(false);
        }
        let empty = self
            .selected
            .and_then(|t| self.views.get(&t))
            .is_none_or(|v| v.draft.text.is_empty());
        match key.code {
            KeyCode::Up | KeyCode::Down if self.sidebar.focus != Focus::Composer || empty => {
                let targets = self.ordered_targets();
                if !targets.is_empty() {
                    let index = self
                        .selected
                        .and_then(|t| targets.iter().position(|x| *x == t))
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::Down {
                        (index + 1) % targets.len()
                    } else {
                        (index + targets.len() - 1) % targets.len()
                    };
                    self.sidebar_select(targets[next]);
                }
                Ok(true)
            }
            KeyCode::Right if self.sidebar.focus != Focus::Composer || empty => {
                self.sidebar.focus = Focus::Button;
                Ok(true)
            }
            KeyCode::Left
                if self.sidebar.focus != Focus::Composer
                    || self
                        .selected
                        .and_then(|t| self.views.get(&t))
                        .is_some_and(|v| v.draft.cursor == 0) =>
            {
                self.sidebar.focus = Focus::Voyages;
                Ok(true)
            }
            KeyCode::Enter if self.sidebar.focus == Focus::Button => {
                if let Some(target) = self.selected {
                    self.open_actions(target);
                }
                Ok(true)
            }
            KeyCode::Enter | KeyCode::Esc if self.sidebar.focus != Focus::Composer => {
                self.sidebar_composer();
                Ok(true)
            }
            KeyCode::Char(_) | KeyCode::Backspace | KeyCode::Delete => {
                self.sidebar_composer();
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    fn action_menu_input(&mut self, menu: &mut Menu, event: &Event) -> Result<bool> {
        if matches!(
            menu.editor,
            Some(Action::Access | Action::ReadOnly | Action::Approval | Action::Unrestricted)
        ) {
            return self.access_input(menu, event);
        }
        let actions = menu.actions.clone();
        if let Event::Mouse(mouse) = event {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) && menu.editor.is_none() {
                let index = self
                    .sidebar
                    .menu_hits
                    .borrow()
                    .iter()
                    .find(|(rect, target, inc, _)| {
                        *target == menu.target
                            && *inc == menu.incarnation
                            && rect.contains((mouse.column, mouse.row).into())
                    })
                    .map(|(_, _, _, i)| *i);
                if self.sidebar.visible.get() != Some((menu.target, menu.incarnation, None)) {
                    return Ok(false);
                }
                if let Some(index) = index {
                    menu.selected = index;
                    return self.activate_action(menu, actions[index]);
                }
            }
            return Ok(false);
        }
        if let Event::Paste(text) = event {
            if matches!(
                menu.editor,
                Some(Action::Rename | Action::Branch | Action::Delete)
            ) {
                let text = super::super::safe(text).replace(['\n', '\r'], " ");
                ensure!(
                    menu.text.text.len() + text.len() <= 256,
                    "Input limit is 256 bytes"
                );
                menu.text.insert_str(&text);
            }
            return Ok(false);
        }
        let Event::Key(key) = event else {
            return Ok(false);
        };
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'q'))
        {
            self.quit = true;
            return Ok(true);
        }
        if key.code == KeyCode::Enter
            && self.sidebar.visible.get() != Some((menu.target, menu.incarnation, menu.editor))
        {
            return Ok(false);
        }
        if key.code == KeyCode::Esc {
            if menu.editor.take().is_some() {
                menu.error.clear();
                return Ok(false);
            }
            self.sidebar.focus = Focus::Button;
            return Ok(true);
        }
        if menu.editor == Some(Action::Details) {
            match key.code {
                KeyCode::Up | KeyCode::PageUp => menu.scroll = menu.scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::PageDown => menu.scroll = menu.scroll.saturating_add(1),
                _ => {}
            }
            return Ok(key.code == KeyCode::Enter);
        }
        if let Some(action) = menu.editor {
            match key.code {
                KeyCode::Enter => {
                    if let Some(reason) = self.action_reason(menu, action) {
                        anyhow::bail!(reason);
                    }
                    let text = menu.text.text.trim();
                    let command = match action {
                        Action::Rename => {
                            ensure!(!text.is_empty(), "Enter a name");
                            format!("/rename {text}")
                        }
                        Action::Branch => format!("/branch {text}"),
                        Action::Delete => {
                            ensure!(
                                text == "DELETE",
                                "Type DELETE to permanently remove this voyage's history"
                            );
                            ensure!(
                                self.views[&menu.target]
                                    .snapshot
                                    .as_ref()
                                    .map(|s| s.revision)
                                    == menu.revision,
                                "Voyage changed; reopen Delete to confirm its current history"
                            );
                            format!("/delete {}", menu.target.session)
                        }
                        _ => unreachable!(),
                    };
                    self.command_for(menu.target, command, true)?;
                    return Ok(true);
                }
                KeyCode::Char(ch)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    ensure!(
                        menu.text.text.len() + ch.len_utf8() <= 256,
                        "Input limit is 256 bytes"
                    );
                    menu.text.insert(ch);
                }
                KeyCode::Backspace => menu.text.backspace(),
                KeyCode::Delete => menu.text.delete(),
                KeyCode::Home => menu.text.line_start(),
                KeyCode::End => menu.text.line_end(),
                KeyCode::Left => {
                    menu.text.cursor = menu.text.text[..menu.text.cursor]
                        .char_indices()
                        .next_back()
                        .map_or(0, |(i, _)| i)
                }
                KeyCode::Right => {
                    if let Some(ch) = menu.text.text[menu.text.cursor..].chars().next() {
                        menu.text.cursor += ch.len_utf8();
                    }
                }
                _ => {}
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Left => return Ok(true),
            KeyCode::Up => {
                menu.selected =
                    (menu.selected.min(actions.len() - 1) + actions.len() - 1) % actions.len()
            }
            KeyCode::Down => menu.selected = (menu.selected + 1) % actions.len(),
            KeyCode::Right if actions[menu.selected.min(actions.len() - 1)] == Action::Access => {
                return self.activate_action(menu, Action::Access);
            }
            KeyCode::Enter => {
                return self.activate_action(menu, actions[menu.selected.min(actions.len() - 1)]);
            }
            _ => {}
        }
        Ok(false)
    }
    fn activate_action(&mut self, menu: &mut Menu, action: Action) -> Result<bool> {
        if let Some(reason) = self.action_reason(menu, action) {
            anyhow::bail!(reason);
        }
        match action {
            Action::Access => {
                self.begin_access(menu);
                Ok(false)
            }
            Action::ReadOnly | Action::Approval | Action::Unrestricted => unreachable!(),
            Action::Rename | Action::Branch | Action::Delete | Action::Details => {
                self.sidebar.visible.set(None);
                menu.editor = Some(action);
                menu.error.clear();
                menu.text = Composer::default();
                menu.revision = self
                    .views
                    .get(&menu.target)
                    .context("voyage unavailable")?
                    .snapshot
                    .as_ref()
                    .map(|s| s.revision);
                Ok(false)
            }
            Action::Archive | Action::Restore | Action::Cancel => {
                self.command_for(
                    menu.target,
                    match action {
                        Action::Archive => "/archive",
                        Action::Restore => "/restore",
                        _ => "/cancel",
                    }
                    .into(),
                    true,
                )?;
                Ok(true)
            }
        }
    }
}
