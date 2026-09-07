//! Discover existing read-only views without entering a command or losing a draft.
use super::App;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

const ITEMS: [(&str, &str, &str); 7] = [
    ("Available tools", "What this voyage can do", "tools"),
    (
        "Permissions",
        "How this voyage may use your computer",
        "policy",
    ),
    ("Tasks", "Progress and remaining work", "todos"),
    (
        "Delegated work",
        "Work assigned to other agents",
        "subagents",
    ),
    (
        "Saved workflows",
        "Available reusable workflows",
        "workflows",
    ),
    (
        "Models",
        "Available models and the current selection",
        "models",
    ),
    ("This machine", "Available capacity", "host_resources"),
];

impl App {
    pub(super) fn explore_input(&mut self, key: &KeyEvent) -> Result<bool> {
        if key.code == KeyCode::F(1) {
            self.explore = None;
            return Ok(false);
        }
        if key.code == KeyCode::F(8) {
            self.help = false;
            self.explore = if self.explore.is_some() {
                None
            } else {
                Some(0)
            };
            return Ok(true);
        }
        let Some(index) = self.explore else {
            return Ok(false);
        };
        match key.code {
            KeyCode::Esc => self.explore = None,
            KeyCode::Up => self.explore = Some(index.saturating_sub(1)),
            KeyCode::Down => self.explore = Some((index + 1).min(ITEMS.len() - 1)),
            KeyCode::Enter => {
                if let Some(target) = self.selected {
                    self.inspect_control(target, ITEMS[index].2)?;
                    if let Some(view) = self.views.get_mut(&target) {
                        view.terminals.open = false;
                        view.scroll = 0;
                    }
                    self.explore = None;
                } else {
                    self.status =
                        "Start a voyage with Ctrl+N to explore its tools and settings.".into();
                }
            }
            _ => {}
        }
        Ok(true)
    }
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let width = area.width.min(76);
    let area = Rect::new(
        area.x + (area.width - width) / 2,
        area.y,
        width,
        area.height,
    );
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(area);
    let title = app
        .selected
        .and_then(|t| app.views.get(&t))
        .map_or_else(|| "Ctrl+N starts a voyage to explore".into(), |v| v.title());
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                "Explore your voyage",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Line::styled(super::safe(&title), Style::default().fg(Color::DarkGray)),
        ]),
        rows[0],
    );
    let items = ITEMS
        .iter()
        .map(|(name, description, _)| {
            ListItem::new(vec![
                Line::from(*name),
                Line::styled(*description, Style::default().fg(Color::DarkGray)),
                Line::default(),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_stateful_widget(
        List::new(items).highlight_symbol("> ").highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        rows[1],
        &mut ListState::default().with_selected(app.explore),
    );
    frame.render_widget(
        Paragraph::new("Up/Down choose   Enter open   Esc back").block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::DarkGray)),
        ),
        rows[2],
    );
}
