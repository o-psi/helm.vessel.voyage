//! One keyboard-searchable catalogue; activation calls handlers, never composer text.
use super::{App, discovery};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    widgets::{List, ListItem, ListState, Paragraph},
};

impl App {
    pub(super) fn explore_input(&mut self, key: &KeyEvent) -> Result<bool> {
        if key.code == KeyCode::F(1) {
            self.discovery.settings = false;
            // The global help overlay retains this catalogue and its query.
            return Ok(false);
        }
        if key.code == KeyCode::F(8) {
            if self.explore.is_some() {
                self.explore = None;
            } else {
                self.discovery_open("actions")?;
            }
            return Ok(true);
        }
        let Some(index) = self.explore else {
            return Ok(false);
        };
        let results = discovery::matches(&self.discovery.query);
        if !matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            self.discovery.detail_scroll = 0;
        }
        match key.code {
            KeyCode::Esc => self.explore = None,
            KeyCode::PageDown => {
                self.discovery.detail_scroll = self.discovery.detail_scroll.saturating_add(1)
            }
            KeyCode::PageUp => {
                self.discovery.detail_scroll = self.discovery.detail_scroll.saturating_sub(1)
            }
            KeyCode::Up => self.explore = Some(index.saturating_sub(1)),
            KeyCode::Down => self.explore = Some((index + 1).min(results.len().saturating_sub(1))),
            KeyCode::Backspace => {
                self.discovery.query.pop();
                self.explore = Some(0);
            }
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !c.is_control() =>
            {
                if self.discovery.query.len() < 256 {
                    self.discovery.query.push(c);
                }
                self.explore = Some(0);
            }
            KeyCode::Enter => {
                if let Some(item) = results.get(index) {
                    self.discovery_open(discovery::COMMANDS[*item].0)?;
                }
            }
            _ => {}
        }
        Ok(true)
    }
}
pub(super) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let results = discovery::matches(&app.discovery.query);
    let detail = results
        .get(app.explore.unwrap_or(0))
        .map(|index| {
            let (name, description, hint) = discovery::COMMANDS[*index];
            format!(
                "/{name} · {description}\n{} · {}\n{}",
                discovery::scope(name),
                discovery::shortcut(name),
                app.discovery_reason(name).unwrap_or_else(|| format!(
                    "Enter opens existing controls; executing-host checks still apply. {hint}"
                ))
            )
        })
        .unwrap_or_else(|| "No matching actions. Backspace edits the search; Esc returns.".into());
    // Reserve at least one result row at small heights. Full details also in /help.
    let detail_height = if area.height >= 12 { 6 } else { 2 };
    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(detail_height),
        Constraint::Length(1),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(format!(
            "Actions · type to search\n{}",
            super::safe(&app.discovery.query)
        )),
        rows[0],
    );
    let items = results
        .iter()
        .map(|index| {
            let (name, description, _) = discovery::COMMANDS[*index];
            ListItem::new(format!("/{name} · {description}"))
        })
        .collect::<Vec<_>>();
    frame.render_stateful_widget(
        List::new(items)
            .highlight_symbol("> ")
            .highlight_style(crate::theme::Role::Selection.style()),
        rows[1],
        &mut ListState::default().with_selected(if results.is_empty() {
            None
        } else {
            app.explore
        }),
    );
    frame.render_widget(
        Paragraph::new(super::presentation::wrap(
            ratatui::text::Text::raw(detail),
            rows[2].width,
        ))
        .scroll((app.discovery.detail_scroll, 0)),
        rows[2],
    );
    frame.render_widget(Paragraph::new("↑↓ Enter Esc · PgUp/Dn details"), rows[3]);
}
