//! Multiplexed presentation; dropping this interface only drops observations.
mod actions;
mod input;
mod updates;
use crate::composer;
mod drafts;
mod observe;
mod render;
mod state;

use crate::process_client::{safe, transport::Client};
use anyhow::{Context, Result};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute, terminal,
};
use futures_util::StreamExt;
use observe::Update;
use ratatui::{Terminal, backend::CrosstermBackend};
use state::{Target, View};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub(super) struct App {
    clients: Vec<Client>,
    views: BTreeMap<Target, View>,
    selected: Option<Target>,
    sender: mpsc::Sender<Update>,
    status: String,
    quit: bool,
}

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            crossterm::event::DisableBracketedPaste,
            terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

pub async fn run(clients: Vec<Client>) -> Result<()> {
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "connected TUI needs a terminal; use connect list/new/inspect/submit for plain operation"
    );
    terminal::enable_raw_mode()?;
    let _screen = Screen;
    execute!(
        io::stdout(),
        terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (sender, mut receiver) = mpsc::channel(64);
    let jobs = observe::spawn(&clients, sender.clone());
    let mut app = App {
        clients,
        views: BTreeMap::new(),
        selected: None,
        sender,
        status:
            "Connected views · Ctrl+N creates · Tab switches · Ctrl+C detaches without cancellation"
                .into(),
        quit: false,
    };
    let mut events = EventStream::new();
    let mut repaint = tokio::time::interval(Duration::from_millis(100));
    let result = async {
        while !app.quit {
            tokio::select! {
                _ = repaint.tick() => terminal.draw(|frame| render::draw(frame, &app)).map(|_| ())?,
                event = events.next() => match event {
                    Some(Ok(event)) => if let Err(error) = app.input(event) { app.status = safe(&error.to_string()); },
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                },
                update = receiver.recv() => if let Some(update) = update { app.update(update); },
            }
        }
        for (target, view) in &app.views { drafts::save(&app.clients[target.route], view)?; }
        Ok(())
    }.await;
    for job in jobs {
        job.abort();
    }
    result
}

impl App {
    fn route_label(&self, route: usize) -> String {
        self.clients[route]
            .ssh
            .as_deref()
            .unwrap_or("local")
            .to_owned()
    }
}
