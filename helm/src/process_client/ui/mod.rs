//! Multiplexed presentation; dropping this interface only drops observations.
mod actions;
mod archive;
mod attachments;
mod completion;
mod controls;
mod explore;
mod export;
mod inference;
mod input;
mod interactions;
mod lifecycle;
mod paste;
mod previews;
mod reconcile;
mod routes;
mod transcript;
mod updates;
mod vessels;
use crate::composer;
mod drafts;
mod effects;
mod new_draft;
pub(super) use new_draft::start_plain;
mod notifications;
mod observe;
mod panels;
mod presentation;
mod render;
mod right_panel;
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
use state::{Route, Target, View};
use std::{
    collections::BTreeMap,
    io::{self, IsTerminal},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

pub(super) struct App {
    coordination_request: Option<uuid::Uuid>,
    previews: previews::State,
    vessels: Option<std::cell::RefCell<vessels::Manager>>,
    vessel_button: std::cell::Cell<ratatui::layout::Rect>,
    vessel_sidebar_button: std::cell::Cell<ratatui::layout::Rect>,
    vessel_filter: Option<uuid::Uuid>,
    clients: routes::Routes,
    observers: BTreeMap<uuid::Uuid, tokio::task::JoinHandle<()>>,
    retired_observers: Vec<tokio::task::JoinHandle<()>>,
    route_tasks: BTreeMap<uuid::Uuid, Vec<tokio::task::JoinHandle<()>>>,
    pending_activations: BTreeMap<uuid::Uuid, Client>,
    pending_disconnects: std::collections::BTreeSet<uuid::Uuid>,
    clipboard_pending: Option<paste::PendingPaste>,
    clipboard_blocked: bool,
    new_chat_config: Option<crate::Config>,
    views: BTreeMap<Target, View>,
    selected: Option<Target>,
    new_drafts: BTreeMap<uuid::Uuid, new_draft::Draft>,
    active_draft: Option<uuid::Uuid>,
    workspace_picker: Option<new_draft::WorkspacePicker>,
    draft_hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, uuid::Uuid)>>,
    sender: mpsc::Sender<Update>,
    command_checks: BTreeMap<(Target, uuid::Uuid), Option<Instant>>,
    first_send_checks: BTreeMap<uuid::Uuid, Instant>,
    status: String,
    quit: bool,
    help: bool,
    archives: bool,
    help_scroll: u16,
    explore: Option<usize>,
    interactions: std::cell::RefCell<interactions::Review>,
    completion: completion::Completion,
    inference: inference::Controls,
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
            crossterm::event::DisableFocusChange,
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
    let mut styles = crate::theme::TerminalStyles::from_env()?;
    let mut effects = effects::Navigation::from_env(styles.allows_color_images())?;
    let previews =
        previews::State::new(styles.allows_color_images(), styles.allows_native_images())?;
    terminal::enable_raw_mode()?;
    let _screen = Screen;
    execute!(
        io::stdout(),
        terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste,
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableFocusChange
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let (sender, mut receiver) = mpsc::channel(64);
    // Only interactive entrypoints reach this boundary; scripted connect commands
    // remain single-target and never consult the remembered address book.
    let mut clients = clients;
    let registry_root =
        crate::process_client::cli::default_directory().with_file_name("helm-connections");
    let manager = registry_root
        .parent()
        .context("connection registry parent")
        .and_then(|parent| std::fs::create_dir_all(parent).map_err(Into::into))
        .and_then(|()| vessels::Manager::open(registry_root));
    let registry_notice = manager.as_ref().err().map(|_| "Saved Vessels unavailable: check private address-book permissions. Local work remains available.".to_owned());
    if let Ok(manager) = &manager {
        for connection in manager.autoconnect().take(16) {
            if clients.len() < 32 && !clients.iter().any(|client| client.id() == connection.id) {
                clients.push(connection.client(manager.registry()));
            }
        }
    }
    let clients = routes::Routes::new(clients);
    let selected =
        session.and_then(|session| clients.first_route().map(|route| Target { route, session }));
    let mut app = App {
        previews,
        vessels: manager.ok().map(std::cell::RefCell::new),
        vessel_button: Default::default(),
        vessel_sidebar_button: Default::default(),
        vessel_filter: None,
        clipboard_pending: None,
        clipboard_blocked: false,
        clients,
        observers: BTreeMap::new(),
        retired_observers: Vec::new(),
        coordination_request: None,
        route_tasks: BTreeMap::new(),
        pending_activations: BTreeMap::new(),
        pending_disconnects: Default::default(),
        new_chat_config,
        views: BTreeMap::new(),
        selected,
        new_drafts: BTreeMap::new(),
        active_draft: None,
        workspace_picker: None,
        draft_hits: Default::default(),
        sender,
        command_checks: BTreeMap::new(),
        first_send_checks: BTreeMap::new(),
        status: notice.or(registry_notice).unwrap_or_else(|| {
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
        inference: Default::default(),
        sidebar: Default::default(),
    };
    app.start_observers();
    app.recover_new_drafts()?;
    if session.is_none() && app.new_chat_config.is_some() {
        if let Some(id) = app.new_drafts.keys().next().copied() {
            app.select_draft(id);
            app.status =
                "Recovered your local draft. Pending first sends will recover automatically."
                    .into();
        } else {
            let workspace = app
                .new_chat_config
                .as_ref()
                .expect("chat config")
                .resolve_workspace(None)?;
            app.create(Some(workspace.to_str().context("workspace is not UTF-8")?))?;
        }
    }
    let mut events = EventStream::new();
    let mut repaint = tokio::time::interval(Duration::from_millis(100));
    let result = async {
        while !app.quit {
            app.poll_clipboard();
            app.poll_previews()?;
            app.sync_completion();
            app.refresh_transcript();
            tokio::select! {
                _ = repaint.tick() => {
                    app.reconcile_pending();
                    terminal.draw(|frame| {
                        render::draw(frame, &app);
                        effects.draw(frame, &app);
                        styles.apply(frame.buffer_mut());
                    }).map(|_| ())?;
                    repaint.reset_after(effects.repaint_after());
                },
                event = events.next() => match event {
                    Some(Ok(event)) => if let Err(error) = app.input(event) { app.status = safe(&error.to_string()); },
                    Some(Err(error)) => return Err(error.into()),
                    None => break,
                },
                update = receiver.recv() => if let Some(update) = update { app.update(update); },
            }
            app.acknowledge_departed_completions();
            if let Some((target,incarnation,run,terminal_id))=app.terminal_request.take() {
                app.previews.clear()?;
                app.sidebar.pointer = None;
                app.sidebar.resize.clear();
                execute!(io::stdout(),crossterm::event::DisableMouseCapture,crossterm::event::DisableFocusChange)?;
                drop(events);
                let result=super::terminal::attach_observed(&app.clients[target.route],target.session,incarnation,run,terminal_id).await;
                terminal::enable_raw_mode()?;
                execute!(io::stdout(),terminal::EnterAlternateScreen,crossterm::event::EnableBracketedPaste,crossterm::event::EnableMouseCapture,crossterm::event::EnableFocusChange)?;
                let (width,height) = terminal::size()?;
                terminal.resize(ratatui::layout::Rect::new(0,0,width,height))?;
                events = EventStream::new();
                app.status=match result {Ok(())=>"Back in Helm. Your program can keep running.".into(),Err(error)=>safe(&error.to_string())};
            }
        }
        for (target, view) in &app.views { drafts::save(&app.clients[target.route], view)?; }
        Ok(())
    }.await;
    let clipboard_cleanup = app.finish_clipboard().await;
    let preview_cleanup = app.previews.finish();
    for (_, job) in std::mem::take(&mut app.observers) {
        job.abort();
        app.retired_observers.push(job);
    }
    for (_, jobs) in app.route_tasks {
        for job in jobs {
            job.abort();
            app.retired_observers.push(job);
        }
    }
    if let Some(manager) = &app.vessels {
        app.retired_observers
            .extend(manager.borrow_mut().stop_tasks());
    }
    for job in app.retired_observers {
        let _ = job.await;
    }
    result.and(clipboard_cleanup).and(preview_cleanup)
}

impl App {
    fn route_label(&self, route: Route) -> String {
        if let Some(manager) = &self.vessels
            && let Some(connection) = manager.borrow().records().iter().find(|c| c.id == route.id)
        {
            return safe(if connection.alias.is_empty() {
                &connection.endpoint
            } else {
                &connection.alias
            });
        }
        let label = self.clients[route].label();
        if label == "local" {
            "This computer".into()
        } else {
            label
        }
    }
}
