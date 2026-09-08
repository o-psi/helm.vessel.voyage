//! Discover metadata without attaching. Only explicit human action opens private capture.
use super::{App, safe, state::Target};
use anyhow::{Context, Result, ensure};
use crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Text},
    widgets::{List, ListItem, ListState, Paragraph, Wrap},
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
                Some(0) => "Finished".into(),
                Some(_) => "Stopped with an error".into(),
                None => "Stopped".into(),
            };
        }
        "Status unavailable".into()
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
    pub fn update(&mut self, result: Result<Inventory, String>, observed: Instant) {
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
                self.observed = Some(observed);
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
            return "Checking programs...".into();
        }
        let entries = self
            .inventory
            .as_ref()
            .map(|i| i.entries.as_slice())
            .unwrap_or(&[]);
        let running = entries.iter().filter(|e| e.running()).count();
        match entries.iter().find(|e| e.running()) {
            Some(entry) => format!(
                "{running} {} running · {} · F3 Console",
                if running == 1 { "program" } else { "programs" },
                safe(&entry.title)
            ),
            None => {
                if entries.is_empty() {
                    "F3 Console".into()
                } else {
                    format!(
                        "{} {} · F3 Console",
                        entries.len(),
                        if entries.len() == 1 {
                            "finished program"
                        } else {
                            "finished programs"
                        }
                    )
                }
            }
        }
    }
}

impl App {
    pub(super) fn request_terminal(&mut self, target: Target, terminal_id: Uuid) -> Result<()> {
        anyhow::ensure!(
            self.clients.current(target.route),
            "Vessel disconnected; terminal input is not buffered or replayed"
        );
        let view = &self.views[&target];
        let browser = &view.terminals;
        ensure!(
            browser.fresh(),
            "Programs are reconnecting. Please wait before opening one."
        );
        let inventory = browser
            .inventory
            .as_ref()
            .context("Finding your programs...")?;
        let entry = inventory
            .entries
            .iter()
            .find(|e| e.id == terminal_id)
            .context("This program is no longer available")?;
        ensure!(
            entry.running(),
            "Terminal is {}; select a running terminal",
            entry.state()
        );
        let run = inventory.run_id.context("This program is unavailable")?;
        self.terminal_request = Some((target, view.process.incarnation, run, terminal_id));
        Ok(())
    }
    pub(super) fn terminal_input(&mut self, event: &Event) -> Result<bool> {
        let Some(target) = self.selected else {
            if matches!(event, Event::Key(key) if key.code == KeyCode::F(3)) {
                self.status = "Start a voyage to see its programs. Ctrl+N creates one.".into();
                return Ok(true);
            }
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
                    "This program reconnected. Select it again before opening."
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
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    if inner.width < 30 || inner.height < 8 {
        frame.render_widget(
            Paragraph::new("Make this window larger to select a program. Esc returns to chat.")
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(2),
        Constraint::Length(4),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled("Console", Style::default().add_modifier(Modifier::BOLD)),
            Line::styled(
                "Choose a program to use directly.",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        areas[0],
    );
    let entries = browser
        .inventory
        .as_ref()
        .map(|i| i.entries.as_slice())
        .unwrap_or(&[]);
    if !browser.fresh() {
        frame.render_widget(Paragraph::new("Reconnecting to your programs...\nYou can return to the conversation while you wait.").wrap(Wrap { trim: false }), areas[1]);
    } else if entries.is_empty() {
        frame.render_widget(Paragraph::new("No programs to open yet.\nInteractive programs appear here when your work starts one.").wrap(Wrap { trim: false }), areas[1]);
    } else {
        let index = entries
            .iter()
            .position(|e| Some(e.id) == browser.selected)
            .unwrap_or(0);
        let panes = Layout::horizontal(if areas[1].width >= 90 {
            [Constraint::Percentage(42), Constraint::Percentage(58)]
        } else {
            [Constraint::Percentage(100), Constraint::Length(0)]
        })
        .split(areas[1]);
        let items = entries
            .iter()
            .map(|entry| {
                ListItem::new(vec![
                    Line::styled(
                        safe(&entry.title),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                    Line::styled(entry.state(), Style::default().fg(Color::DarkGray)),
                    Line::default(),
                ])
            })
            .collect::<Vec<_>>();
        frame.render_stateful_widget(
            List::new(items)
                .highlight_symbol("> ")
                .highlight_style(Style::default().fg(Color::Cyan)),
            panes[0],
            &mut ListState::default().with_selected(Some(index)),
        );
        if panes[1].width > 0 {
            let entry = &entries[index];
            let detail = Text::from(vec![
                Line::styled(
                    safe(&entry.title),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    app.route_label(target.route),
                    Style::default().fg(Color::DarkGray),
                ),
                Line::default(),
                Line::from(if entry.running() {
                    "Press Enter to open this program."
                } else {
                    "This program has ended."
                }),
                Line::default(),
                Line::from(
                    "Opening it makes its input and output private. The assistant can no longer read this terminal.",
                ),
                Line::default(),
                Line::from("Ctrl+] brings you back. Your work keeps running."),
            ]);
            let pane = panes[1].inner(ratatui::layout::Margin::new(2, 0));
            frame.render_widget(Paragraph::new(detail).wrap(Wrap { trim: false }), pane);
        }
        if let Some(run) = browser.inventory.as_ref().and_then(|i| i.run_id)
            && entries[index].running()
        {
            browser
                .displayed
                .set(Some((view.process.incarnation, run, entries[index].id)));
        }
    }
    let action = if !browser.fresh() {
        "Waiting for connection"
    } else if entries
        .iter()
        .any(|e| Some(e.id) == browser.selected && e.running())
    {
        "Up/Down Choose   Enter Open private console"
    } else {
        "Up/Down Choose   No running program selected"
    };
    frame.render_widget(
        Paragraph::new(super::presentation::wrap(
            Text::from(vec![
                Line::styled(action, Style::default().fg(Color::Cyan)),
                Line::styled(
                    "Private after opening. Passwords may be hidden.",
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            areas[2].width,
        )),
        areas[2],
    );
}
