//! Explicit human-only terminal attachment. Input never enters drafts or history.
use super::{safe, transport::Client};
use crate::theme::{Role, TerminalStyles};
use anyhow::{Context, Result, ensure};
use crossterm::{
    event::{Event, EventStream},
    execute, terminal,
};
use futures_util::StreamExt;
use std::io::IsTerminal;
use uuid::Uuid;
use voyage_protocol::vessel::{ProcessInfo, TerminalAction, VesselCommand};

mod input;
mod output;
mod view;

#[derive(Debug, thiserror::Error)]
#[error("private terminal cleanup could not be confirmed; stop this Helm interface")]
pub(super) struct CleanupFailure;

struct Screen {
    finished: bool,
}
impl Screen {
    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        // No queued tail is a chat prompt, including after explicit detach.
        let discard = discard_input();
        let raw = terminal::disable_raw_mode();
        let restore = output::Output::new().and_then(|mut out| {
            execute!(
                out,
                terminal::LeaveAlternateScreen,
                crossterm::event::DisableBracketedPaste,
                crossterm::cursor::Show
            )
        });
        self.finished = true;
        if discard.is_err() || raw.is_err() || restore.is_err() {
            return Err(CleanupFailure.into());
        }
        Ok(())
    }
}
fn discard_input() -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while crossterm::event::poll(std::time::Duration::ZERO)? {
        if std::time::Instant::now() >= deadline {
            return Err(std::io::Error::other("private input disposal deadline"));
        }
        let _ = crossterm::event::read()?;
    }
    #[cfg(unix)]
    if unsafe { libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub async fn attach(
    client: &Client,
    session_id: Uuid,
    run_id: Uuid,
    terminal_id: Uuid,
) -> Result<()> {
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect { session_id })
            .await?,
    )?;
    attach_observed(client, session_id, process.incarnation, run_id, terminal_id).await
}

pub async fn attach_observed(
    client: &Client,
    session_id: Uuid,
    incarnation: Uuid,
    run_id: Uuid,
    terminal_id: Uuid,
) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "terminal attachment requires a human terminal"
    );
    let mut guard = Screen { finished: false };
    let result = attach_inner(client, session_id, incarnation, run_id, terminal_id).await;
    guard.finish()?;
    result
}

async fn attach_inner(
    client: &Client,
    session_id: Uuid,
    incarnation: Uuid,
    run_id: Uuid,
    terminal_id: Uuid,
) -> Result<()> {
    let stop = stop_signal()?;
    tokio::pin!(stop);
    let mut styles = TerminalStyles::from_env()?;
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect { session_id })
            .await?,
    )?;
    ensure!(
        process.incarnation == incarnation,
        "Voyage restarted; reopen the terminal browser before attaching"
    );
    let socket_id = client
        .connection_state()
        .borrow()
        .socket_id
        .context("private terminal socket unavailable")?;
    client
        .private_terminal(
            socket_id,
            session_id,
            incarnation,
            run_id,
            terminal_id,
            TerminalAction::Attach,
        )
        .await?;
    terminal::enable_raw_mode()?;
    execute!(
        output::Output::new()?,
        terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste
    )?;
    let (columns, rows) = terminal::size()?;
    client
        .private_terminal(
            socket_id,
            session_id,
            incarnation,
            run_id,
            terminal_id,
            TerminalAction::Resize {
                columns: columns.clamp(1, 240),
                rows: rows.saturating_sub(4).clamp(1, 120),
            },
        )
        .await?;
    let mut display = ratatui::Terminal::with_options(
        ratatui::backend::CrosstermBackend::new(output::Output::new()?),
        ratatui::TerminalOptions {
            viewport: ratatui::Viewport::Fixed(view::viewport(columns, rows)),
        },
    )?;
    let mut observation = view::Observation::new(
        run_id,
        terminal_id,
        (columns.clamp(1, 240), rows.saturating_sub(4).clamp(1, 120)),
    );
    let initial = client
        .private_terminal(
            socket_id,
            session_id,
            incarnation,
            run_id,
            terminal_id,
            TerminalAction::Snapshot,
        )
        .await?;
    ensure!(
        observation.accept(&initial)?,
        "terminal resize observation not current"
    );
    draw_screen(
        &mut display,
        &initial,
        observation.screen(),
        &client.label(),
        &mut styles,
    )?;
    ensure!(
        initial["state"] == "running",
        "terminal is no longer running"
    );
    let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
    let worker_client = client.clone();
    let worker = tokio::spawn(async move {
        let mut pending = None;
        while let Some(mut operation) = match pending.take() {
            Some(operation) => Some(operation),
            None => receiver.recv().await,
        } {
            if let TerminalAction::Write { bytes } = &mut operation {
                while let Ok(next) = receiver.try_recv() {
                    match next {
                        TerminalAction::Write { bytes: more }
                            if bytes.len() + more.len() <= 65536 =>
                        {
                            bytes.extend(more)
                        }
                        other => {
                            pending = Some(other);
                            break;
                        }
                    }
                }
            }
            // Sequential sends preserve private input ordering; never retry uncertain writes.
            worker_client
                .private_terminal(
                    socket_id,
                    session_id,
                    incarnation,
                    run_id,
                    terminal_id,
                    operation,
                )
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    });
    let mut worker = tokio_util::task::AbortOnDropHandle::new(worker);
    let mut input = EventStream::new();
    let (screens, mut snapshots) = tokio::sync::mpsc::channel(1);
    let observer_client = client.clone();
    let observer = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let result = observer_client
                .private_terminal(
                    socket_id,
                    session_id,
                    incarnation,
                    run_id,
                    terminal_id,
                    TerminalAction::Snapshot,
                )
                .await
                .map_err(|error| error.to_string());
            let failed = result.is_err();
            if screens.send(result).await.is_err() || failed {
                return;
            }
        }
    });
    let observer = tokio_util::task::AbortOnDropHandle::new(observer);
    let mut worker_finished = false;
    let result = async {
        loop {
            tokio::select! {
                result = &mut worker => { worker_finished = true; result??; break; },
                _ = &mut stop => { break; },
                event = input.next() => {
                    let Some(event) = event else {break};
                    let operation = match event? {
                        Event::Key(key) if key.kind != crossterm::event::KeyEventKind::Release => {
                            if input::detached(key.code, key.modifiers) { break; }
                            let Some(bytes) = input::keypad(key.code, key.state, key.modifiers, &observation.modes()).or_else(|| input::key_bytes(key.code, key.modifiers, &observation.modes())) else {continue};
                            TerminalAction::Write { bytes }
                        }
                        Event::Paste(text) => TerminalAction::Write { bytes: input::paste(text, &observation.modes())? },
                        Event::Resize(columns, rows) => {
                            display.backend_mut().writer_mut().begin_frame();
                            display.resize(view::viewport(columns,rows))?;
                            let (columns,rows) = (columns.clamp(1,240), rows.saturating_sub(4).clamp(1,120));
                            observation.resize((columns,rows));
                            TerminalAction::Resize {columns,rows}
                        },
                        _ => continue,
                    };
                    sender.try_send(operation).context("private terminal input queue full; attachment stopped without replay")?;
                }
                screen = snapshots.recv() => {
                    let screen=screen.context("terminal observation stopped")?.map_err(anyhow::Error::msg)?;
                    if observation.accept(&screen)? {
                        draw_screen(&mut display, &screen, observation.screen(), &client.label(), &mut styles)?;
                        if screen["state"] != "running" { break; }
                    }
                }
            }
        }
        Ok(())
    }.await;
    worker.abort();
    observer.abort();
    if !worker_finished {
        let _ = worker.await;
    }
    let _ = observer.await;
    drop(input);
    drop(display);
    result
}

/// Clip whole graphemes by terminal cells; never split CJK or combining sequences.
fn clipped(value: &str, columns: u16) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    use unicode_width::UnicodeWidthStr;
    let mut used = 0;
    safe(value)
        .graphemes(true)
        .take_while(|grapheme| {
            used += grapheme.width();
            used <= usize::from(columns)
        })
        .collect()
}

fn draw_screen(
    display: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<output::Output>>,
    screen: &serde_json::Value,
    structured: Option<&voyage_protocol::terminal::TerminalScreen>,
    host: &str,
    styles: &mut TerminalStyles,
) -> Result<()> {
    use ratatui::{layout::Rect, widgets::Paragraph};
    let host = if host == "local" {
        "This computer"
    } else {
        host
    };
    display.backend_mut().writer_mut().begin_frame();
    display.draw(|frame| {
        let area = frame.area();
        let (columns, rows) = (area.width, area.height);
        frame.render_widget(
            Paragraph::new(clipped(
                &format!(
                    "{} / PRIVATE TERMINAL / {}",
                    safe(screen["title"].as_str().unwrap_or("Program")),
                    safe(host)
                ),
                columns,
            ))
            .style(Role::PrivateTerminal.style()),
            Rect::new(0, 0, columns, rows.min(1)),
        );
        if rows > 1 {
            frame.render_widget(
                Paragraph::new(clipped(
                    "Type here to use this program. Ctrl+] returns to Helm.",
                    columns,
                )),
                Rect::new(0, 1, columns, 1),
            );
        }
        view::render(
            frame.buffer_mut(),
            Rect::new(0, 2, columns, rows.saturating_sub(4)),
            structured,
            &screen["rows"],
        );
        let state = if let Some(state) = screen["state"].as_str() {
            state.to_owned()
        } else if let Some(exit) = screen["state"].get("exited") {
            exit["code"].as_i64().map_or("stopped".into(), |code| {
                if code == 0 {
                    "finished".into()
                } else {
                    "stopped with an error".into()
                }
            })
        } else {
            "unknown".into()
        };
        if rows > 2 {
            frame.render_widget(
                Paragraph::new(clipped(
                    &format!("Program: {state} | Ctrl+] detach | Ctrl+C interrupts program"),
                    columns,
                ))
                .style(Role::AwaitingInput.style()),
                Rect::new(0, rows - 2, columns, 1),
            );
        }
        if let Some(cursor) = screen["cursor"].as_array()
            && cursor.len() == 2
        {
            let col = cursor[0].as_u64().unwrap_or(u64::MAX);
            let row = cursor[1].as_u64().unwrap_or(u64::MAX);
            if row < u64::from(rows.saturating_sub(4)) && col < u64::from(columns) {
                frame.set_cursor_position((col as u16, row as u16 + 2));
            }
        }
        styles.apply(frame.buffer_mut());
    })?;
    Ok(())
}

fn stop_signal() -> Result<std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        // Register before changing terminal modes, including during preflight I/O.
        let mut terminate = signal(SignalKind::terminate())?;
        let mut hangup = signal(SignalKind::hangup())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        Ok(Box::pin(async move {
            tokio::select! { _ = terminate.recv() => {}, _ = hangup.recv() => {}, _ = interrupt.recv() => {} }
        }))
    }
    #[cfg(not(unix))]
    {
        Ok(Box::pin(async {
            let _ = tokio::signal::ctrl_c().await;
        }))
    }
}
