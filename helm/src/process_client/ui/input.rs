use super::*;

impl App {
    pub(super) fn input(&mut self, event: Event) -> Result<()> {
        if let Event::Key(key) = &event {
            if key.kind == crossterm::event::KeyEventKind::Release {
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
            if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                let keys: Vec<_> = self.views.keys().copied().collect();
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
                    self.views.get_mut(&keys[next]).expect("known view").unread = false;
                }
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
        let Some(target) = self.selected else {
            return Ok(());
        };
        let view = self
            .views
            .get_mut(&target)
            .context("selected voyage unavailable")?;
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
            _ => return Ok(()),
        }
        drafts::save(&self.clients[target.route], view)
    }
}
