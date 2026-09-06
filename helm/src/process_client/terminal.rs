//! Explicit human-only terminal attachment. Input never enters drafts or history.
use super::{safe, transport::Client};
use anyhow::{Context, Result, ensure};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute, terminal,
};
use futures_util::StreamExt;
use std::io::{IsTerminal, Write};
use uuid::Uuid;
use voyage_protocol::process::{ProcessInfo, RuntimeCommand, TerminalOperation, VesselCommand};

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
        let _ = execute!(
            std::io::stdout(),
            terminal::LeaveAlternateScreen,
            crossterm::event::DisableBracketedPaste,
            crossterm::cursor::Show
        );
    }
}

pub async fn attach(
    client: &Client,
    session_id: Uuid,
    run_id: Uuid,
    terminal_id: Uuid,
) -> Result<()> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        "terminal attachment requires a human terminal"
    );
    let process: ProcessInfo = serde_json::from_value(
        client
            .request(VesselCommand::Inspect { session_id })
            .await?,
    )?;
    let command = |operation| RuntimeCommand::Terminal {
        run_id,
        terminal_id,
        operation,
    };
    client
        .forward(
            session_id,
            process.incarnation,
            command(TerminalOperation::Attach),
        )
        .await?;
    terminal::enable_raw_mode()?;
    let _screen = Screen;
    execute!(
        std::io::stdout(),
        terminal::EnterAlternateScreen,
        crossterm::event::EnableBracketedPaste
    )?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(64);
    let worker_client = client.clone();
    let worker = tokio::spawn(async move {
        let mut pending = None;
        while let Some(mut operation) = match pending.take() {
            Some(operation) => Some(operation),
            None => receiver.recv().await,
        } {
            if let TerminalOperation::Write { bytes } = &mut operation {
                while let Ok(next) = receiver.try_recv() {
                    match next {
                        TerminalOperation::Write { bytes: more }
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
                .forward(
                    session_id,
                    process.incarnation,
                    RuntimeCommand::Terminal {
                        run_id,
                        terminal_id,
                        operation,
                    },
                )
                .await?;
        }
        Ok::<_, anyhow::Error>(())
    });
    let mut worker = worker;
    let mut input = EventStream::new();
    let (screens, mut snapshots) = tokio::sync::mpsc::channel(1);
    let observer_client = client.clone();
    let observer = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            tick.tick().await;
            let result = observer_client
                .forward(
                    session_id,
                    process.incarnation,
                    RuntimeCommand::Terminal {
                        run_id,
                        terminal_id,
                        operation: TerminalOperation::Snapshot,
                    },
                )
                .await
                .map_err(|error| error.to_string());
            let failed = result.is_err();
            if screens.send(result).await.is_err() || failed {
                return;
            }
        }
    });
    let result = async {
        loop {
            tokio::select! {
                result = &mut worker => { result??; break; },
                event = input.next() => {
                    let Some(event) = event else {break};
                    let operation = match event? {
                        Event::Key(key) if key.kind != crossterm::event::KeyEventKind::Release => {
                            if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code,KeyCode::Char(']'|'5')) { break; }
                            let Some(bytes) = key_bytes(key.code, key.modifiers) else {continue};
                            TerminalOperation::Write { bytes }
                        }
                        Event::Paste(text) => { ensure!(text.len() <= 65536, "private paste exceeds 64 KiB"); TerminalOperation::Write {bytes:text.into_bytes()} }
                        Event::Resize(columns, rows) => TerminalOperation::Resize {columns:columns.clamp(1,500), rows:rows.saturating_sub(1).clamp(1,500)},
                        _ => continue,
                    };
                    sender.try_send(operation).context("private terminal input queue full; attachment stopped without replay")?;
                }
                screen = snapshots.recv() => {
                    let screen=screen.context("terminal observation stopped")?.map_err(anyhow::Error::msg)?;
                    ensure!(screen["terminal_id"] == serde_json::to_value(terminal_id)? && screen["run_id"] == serde_json::to_value(run_id)?, "terminal observation identity mismatch");
                    execute!(std::io::stdout(), crossterm::cursor::MoveTo(0,0), terminal::Clear(terminal::ClearType::All))?;
                    let (columns, rows) = terminal::size()?;
                    for row in screen["rows"].as_array().into_iter().flatten().take(rows.saturating_sub(1) as usize) {
                        let row = safe(row.as_str().unwrap_or_default());
                        write!(std::io::stdout(), "{}\r\n", row.chars().take(columns as usize).collect::<String>())?;
                    }
                    execute!(std::io::stdout(), crossterm::cursor::MoveTo(0,rows.saturating_sub(1)))?;
                    write!(std::io::stdout(), "Private terminal · Ctrl+] detaches")?;
                    std::io::stdout().flush()?;
                }
            }
        }
        Ok(())
    }.await;
    worker.abort();
    observer.abort();
    result
}

fn key_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    let mut bytes = match code {
        KeyCode::Char(ch @ '4'..='7') if modifiers.contains(KeyModifiers::CONTROL) => {
            vec![ch as u8 - b'4' + 0x1c]
        }
        KeyCode::Char(ch) if modifiers.contains(KeyModifiers::CONTROL) && ch.is_ascii() => {
            vec![(ch.to_ascii_uppercase() as u8) & 0x1f]
        }
        KeyCode::Char(ch) => ch.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![27],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => return None,
    };
    if modifiers.contains(KeyModifiers::ALT) {
        bytes.insert(0, 27);
    }
    Some(bytes)
}
