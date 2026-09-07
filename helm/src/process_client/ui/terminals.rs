//! Discover metadata without attaching. Only explicit human action opens private capture.
use super::{App, safe, state::Target};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use serde::Deserialize;
use std::{
    cell::Cell,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Deserialize)]
pub(super) struct Entry {
    id: Uuid,
    title: String,
    state: serde_json::Value,
}
impl Entry {
    fn state(&self) -> String {
        if let Some(state) = self.state.as_str() {
            return super::presentation::label(state);
        }
        if let Some(exited) = self.state.get("exited") {
            return match exited["code"].as_i64() {
                Some(code) => format!("Exited (code {code})"),
                None => "Exited (code unavailable)".into(),
            };
        }
        "Unknown state".into()
    }
    fn running(&self) -> bool {
        self.state == "running"
    }
}
#[derive(Deserialize)]
pub(super) struct Inventory {
    run_id: Option<Uuid>,
    #[serde(rename = "value")]
    entries: Vec<Entry>,
}
#[derive(Default)]
pub(super) struct Browser {
    pub open: bool,
    inventory: Option<Inventory>,
    selected: Option<Uuid>,
    observed: Option<Instant>,
    error: Option<String>,
    // A repaint is required after navigation or changed ownership.
    displayed: Cell<Option<(Uuid, Uuid, Uuid)>>,
}
impl Browser {
    pub fn clear_displayed(&self) {
        self.displayed.set(None);
    }
    pub fn update(&mut self, result: Result<Inventory, String>) {
        match result {
            Ok(inventory) => {
                if self.inventory.as_ref().and_then(|i| i.run_id) != inventory.run_id {
                    self.displayed.set(None);
                }
                if !inventory
                    .entries
                    .iter()
                    .any(|e| Some(e.id) == self.selected)
                {
                    self.selected = inventory.entries.first().map(|e| e.id);
                    self.displayed.set(None);
                }
                self.inventory = Some(inventory);
                self.observed = Some(Instant::now());
                self.error = None;
            }
            Err(error) => {
                self.error = Some(safe(&error));
                self.displayed.set(None);
            }
        }
    }
    fn fresh(&self) -> bool {
        self.error.is_none()
            && self
                .observed
                .is_some_and(|t| t.elapsed() < Duration::from_secs(5))
    }
    pub fn summary(&self) -> String {
        if !self.fresh() {
            return "Terminals: checking / unavailable".into();
        }
        let entries = self
            .inventory
            .as_ref()
            .map(|i| i.entries.as_slice())
            .unwrap_or(&[]);
        let running = entries.iter().filter(|e| e.running()).count();
        match entries.iter().find(|e| e.running()) {
            Some(entry) => format!(
                "Terminals: {running} running | {} | F3 open",
                safe(&entry.title)
            ),
            None => format!("Terminals: {} | F3 browse", entries.len()),
        }
    }
}

impl App {
    pub(super) fn request_terminal(&mut self, target: Target, terminal_id: Uuid) -> Result<()> {
        let view = &self.views[&target];
        let browser = &view.terminals;
        ensure!(
            browser.fresh(),
            "Terminal inventory unavailable or stale; wait for reconnection"
        );
        let inventory = browser
            .inventory
            .as_ref()
            .context("Waiting for terminal inventory")?;
        let entry = inventory
            .entries
            .iter()
            .find(|e| e.id == terminal_id)
            .context("Terminal no longer available")?;
        ensure!(
            entry.running(),
            "Terminal is {}; select a running terminal",
            entry.state()
        );
        let run = inventory.run_id.context("Terminal owner unavailable")?;
        self.terminal_request = Some((target, view.process.incarnation, run, terminal_id));
        Ok(())
    }
    pub(super) fn terminal_input(&mut self, event: &Event) -> Result<bool> {
        let Some(target) = self.selected else {
            return Ok(false);
        };
        let Some(view) = self.views.get_mut(&target) else {
            return Ok(false);
        };
        let browser = &mut view.terminals;
        let Event::Key(key) = event else {
            if matches!(event, Event::Resize(..)) {
                browser.clear_displayed();
            }
            return Ok(browser.open);
        };
        if key.code == KeyCode::F(3) && key.kind == KeyEventKind::Press {
            browser.open = !browser.open;
            browser.displayed.set(None);
            return Ok(true);
        }
        if !browser.open {
            return Ok(false);
        }
        if key.kind != KeyEventKind::Press {
            return Ok(true);
        }
        match key.code {
            KeyCode::Esc => {
                browser.open = false;
                browser.displayed.set(None);
            }
            KeyCode::Up | KeyCode::Down => {
                if let Some(inventory) = &browser.inventory
                    && !inventory.entries.is_empty()
                {
                    let index = inventory
                        .entries
                        .iter()
                        .position(|e| Some(e.id) == browser.selected)
                        .unwrap_or(0);
                    let next = if key.code == KeyCode::Up {
                        index.saturating_sub(1)
                    } else {
                        (index + 1).min(inventory.entries.len() - 1)
                    };
                    browser.selected = Some(inventory.entries[next].id);
                    browser.displayed.set(None);
                }
            }
            KeyCode::Enter => {
                if let Some(entry) = browser
                    .inventory
                    .as_ref()
                    .and_then(|i| i.entries.iter().find(|e| Some(e.id) == browser.selected))
                {
                    ensure!(
                        entry.running(),
                        "Terminal is {}; select a running terminal",
                        entry.state()
                    );
                }
                let (incarnation, run, id) = browser
                    .displayed
                    .get()
                    .context("Wait for a running terminal to be displayed")?;
                ensure!(
                    incarnation == view.process.incarnation
                        && browser.inventory.as_ref().and_then(|i| i.run_id) == Some(run),
                    "Terminal ownership changed; review again"
                );
                self.request_terminal(target, id)?;
            }
            _ => {}
        }
        Ok(true)
    }
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &App, area: Rect) {
    let Some((target, view)) = app.selected.and_then(|t| app.views.get(&t).map(|v| (t, v))) else {
        return;
    };
    let browser = &view.terminals;
    browser.displayed.set(None);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(if area.width >= 64 {
            " TERMINALS / Up Down select / Enter attach / Esc back "
        } else {
            " TERMINALS / Enter open / Esc back "
        })
        .border_style(Style::default().fg(Color::Cyan));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 30 || inner.height < 10 {
        frame.render_widget(
            Paragraph::new(
                "Enlarge to at least 32 columns and 12 rows to select a terminal safely.",
            )
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let areas = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(4),
        Constraint::Length(4),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(format!(
            "Voyage: {}\nHost: {}",
            safe(&view.title()),
            app.route_label(target.route)
        ))
        .wrap(Wrap { trim: false }),
        areas[0],
    );
    let entries = browser
        .inventory
        .as_ref()
        .map(|i| i.entries.as_slice())
        .unwrap_or(&[]);
    if !browser.fresh() {
        frame.render_widget(
            Paragraph::new(format!(
                "Terminal inventory unavailable\n{}\nAttachment disabled until refreshed.",
                browser
                    .error
                    .as_deref()
                    .unwrap_or("Waiting for current owner observation...")
            ))
            .wrap(Wrap { trim: false }),
            areas[1],
        );
    } else if entries.is_empty() {
        frame.render_widget(Paragraph::new("No terminals in this voyage.\n\nInteractive programs appear here when the runtime opens a terminal. Ordinary shell commands do not create an attachable console.").wrap(Wrap { trim: false }), areas[1]);
    } else {
        let index = entries
            .iter()
            .position(|e| Some(e.id) == browser.selected)
            .unwrap_or(0);
        let items: Vec<_> = entries
            .iter()
            .map(|entry| {
                ListItem::new(vec![
                    Line::styled(
                        safe(&entry.title),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Line::styled(
                        format!("  {}", entry.state()),
                        Style::default().fg(if entry.running() {
                            Color::Green
                        } else {
                            Color::DarkGray
                        }),
                    ),
                ])
            })
            .collect();
        let panes = Layout::horizontal(if areas[1].width >= 90 {
            [Constraint::Percentage(35), Constraint::Percentage(65)]
        } else {
            [Constraint::Percentage(100), Constraint::Length(0)]
        })
        .split(areas[1]);
        frame.render_stateful_widget(
            List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" PROGRAMS "))
                .highlight_symbol("> ")
                .highlight_style(Style::default().bg(Color::DarkGray)),
            panes[0],
            &mut ListState::default().with_selected(Some(index)),
        );
        if panes[1].width > 0 {
            let entry = &entries[index];
            let detail = Text::from(vec![
                Line::styled(
                    safe(&entry.title),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::from(format!("State: {}", entry.state())),
                Line::from(format!("Host: {}", app.route_label(target.route))),
                Line::from(format!(
                    "Workspace: {}",
                    safe(&view.process.workspace.display().to_string())
                )),
                Line::default(),
                Line::styled(
                    "HOW TO USE THIS CONSOLE",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::from(if entry.running() {
                    "1. Press Enter to open the selected program."
                } else {
                    "This terminal has ended; select a running program."
                }),
                Line::from("2. Type directly into its private terminal."),
                Line::from("3. Press Ctrl+] to return to this browser."),
                Line::default(),
                Line::styled("PRIVACY", Style::default().fg(Color::Yellow)),
                Line::from("Attaching permanently stops model capture for this terminal."),
                Line::from("The assistant cannot read subsequent terminal output."),
                Line::from("Check progress here; your chat draft stays separate."),
            ]);
            frame.render_widget(
                Paragraph::new(super::presentation::wrap(
                    detail,
                    panes[1].width.saturating_sub(2),
                ))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" SELECTED PROGRAM "),
                ),
                panes[1],
            );
        }
        if let Some(run) = browser.inventory.as_ref().and_then(|i| i.run_id)
            && entries[index].running()
        {
            browser
                .displayed
                .set(Some((view.process.incarnation, run, entries[index].id)));
        }
    }
    frame.render_widget(
        Paragraph::new(Text::from(vec![
            Line::styled(
                if browser
                    .inventory
                    .as_ref()
                    .and_then(|i| i.entries.iter().find(|e| Some(e.id) == browser.selected))
                    .is_some_and(|e| e.running())
                {
                    "ENTER opens a PRIVATE terminal"
                } else {
                    "Select a running terminal"
                },
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::from("Private input goes to the program."),
            Line::from("Passwords may be invisible."),
            Line::from("Ctrl+] returns; work continues."),
        ]))
        .wrap(Wrap { trim: false }),
        areas[2],
    );
}
