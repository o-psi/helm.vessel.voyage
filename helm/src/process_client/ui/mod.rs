//! Multiplexed presentation; dropping this interface only drops observations.
mod actions;
mod archive;
mod completion;
mod controls;
mod explore;
mod export;
mod input;
mod interactions;
mod lifecycle;
mod transcript;
mod updates;
use crate::composer;
mod drafts;
mod observe;
mod panels;
mod presentation;
mod render;
mod sidebar;
mod state;
mod terminals;

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
    new_chat_config: Option<crate::Config>,
    views: BTreeMap<Target, View>,
    selected: Option<Target>,
    sender: mpsc::Sender<Update>,
    status: String,
    quit: bool,
    help: bool,
    archives: bool,
    help_scroll: u16,
    explore: Option<usize>,
    interactions: std::cell::RefCell<interactions::Review>,
    completion: completion::Completion,
    sidebar: sidebar::Sidebar,
    terminal_request: Option<(Target, uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
}

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            crossterm::event::DisableBracketedPaste,
            crossterm::event::DisableMouseCapture,
            terminal::LeaveAlternateScreen,
            crossterm::cursor::Show
        );
    }
}

pub async fn run(clients: Vec<Client>) -> Result<()> {
    run_selected(clients, None).await
}

pub async fn run_selected(clients: Vec<Client>, session: Option<uuid::Uuid>) -> Result<()> {
    run_with_config(clients, session, None).await
}

pub async fn run_with_config(
    clients: Vec<Client>,
    session: Option<uuid::Uuid>,
    new_chat_config: Option<crate::Config>,
) -> Result<()> {
    run_with_notice(clients, session, new_chat_config, None).await
}

pub async fn run_with_notice(
    clients: Vec<Client>,
    session: Option<uuid::Uuid>,
    new_chat_config: Option<crate::Config>,
    notice: Option<String>,
) -> Result<()> {
    anyhow::ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "connected TUI needs a terminal; use connect list/new/inspect/submit for plain operation"
    );
    terminal::enable_raw_mode()?;
    let _screen = Screen;
    execute!(
        io::stdout(),
        terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::event::EnableMouseCapture
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (sender, mut receiver) = mpsc::channel(64);
    let jobs = observe::spawn(&clients, sender.clone());
    let mut app = App {
        clients,
        new_chat_config,
        views: BTreeMap::new(),
        selected: session.map(|session| Target { route: 0, session }),
        sender,
        status: notice.unwrap_or_else(|| {
            "Your workspace is ready. Start a conversation, or press F1 for help.".into()
        }),
        quit: false,
        help: false,
        archives: false,
        help_scroll: 0,
        explore: None,
        interactions: Default::default(),
        terminal_request: None,
        completion: Default::default(),
        sidebar: Default::default(),
    };
    let mut events = EventStream::new();
    let mut repaint = tokio::time::interval(Duration::from_millis(100));
    let result = async {
        while !app.quit {
            app.sync_completion();
            app.refresh_transcript();
            tokio::select! {
                _ = repaint.tick() => terminal.draw(|frame| render::draw(frame, &app)).map(|_| ())?,
                event = events.next() => match event {
                    Some(Ok(event)) => if let Err(error) = app.input(event) { app.status = safe(&error.to_string()); },
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                },
                update = receiver.recv() => if let Some(update) = update { app.update(update); },
            }
            if let Some((target,incarnation,run,terminal_id))=app.terminal_request.take() {
                execute!(io::stdout(),crossterm::event::DisableMouseCapture)?;
                drop(events);
                let result=super::terminal::attach_observed(&app.clients[target.route],target.session,incarnation,run,terminal_id).await;
                terminal::enable_raw_mode()?;
                execute!(io::stdout(),terminal::EnterAlternateScreen,crossterm::event::EnableBracketedPaste,crossterm::event::EnableMouseCapture)?;
                let (width,height) = terminal::size()?;
                terminal.resize(ratatui::layout::Rect::new(0,0,width,height))?;
                events = EventStream::new();
                app.status=match result {Ok(())=>"Back in Helm. Your program can keep running.".into(),Err(error)=>safe(&error.to_string())};
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
        let label = self.clients[route].label();
        if label == "local" {
            "This computer".into()
        } else {
            label
        }
    }
}
