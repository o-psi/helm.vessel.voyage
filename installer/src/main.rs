mod flow;
mod service;

use anyhow::{Result, bail};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use flow::{Step, Wizard};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct TerminalGuard;
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}
fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(
        io::stdout(),
        event::DisableBracketedPaste,
        LeaveAlternateScreen,
        Show
    );
}

fn draw(frame: &mut Frame, w: &Wizard) {
    let area = frame.area();
    if area.width < 48 || area.height < 20 {
        frame.render_widget(
            Paragraph::new("Voyage setup preview\nResize to at least 48 x 20.\nCtrl+C cancels.")
                .wrap(Wrap { trim: false }),
            area,
        );
        return;
    }
    let regions = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .margin(1)
    .split(area);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("VOYAGE", Style::default().fg(Color::Cyan).bold()),
                Span::raw("  /  SETUP PREVIEW"),
            ]),
            Line::from("Mock actions only. Use example values; never enter secrets."),
        ]),
        regions[0],
    );
    let s = w.step();
    let route = w.route();
    let progress = route
        .iter()
        .position(|v| *v == s)
        .map(|i| format!("{} / {}", i + 1, route.len()))
        .unwrap_or_else(|| "Preview complete".into());
    let panel = Block::default()
        .borders(Borders::TOP)
        .title(format!(" {}  ·  {} ", progress, s.title()))
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = panel.inner(regions[1]);
    frame.render_widget(panel, regions[1]);
    let parts = Layout::vertical([
        Constraint::Length(4),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(s.hint())
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: false }),
        parts[0],
    );
    if matches!(s, Step::Review | Step::Done) {
        let content = if s == Step::Review {
            w.summary()
        } else {
            w.actions()
        };
        let chunks = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(if s == Step::Review { 3 } else { 1 }),
        ])
        .split(parts[1]);
        frame.render_widget(
            Paragraph::new(content.join("\n\n"))
                .wrap(Wrap { trim: false })
                .scroll((w.scroll, 0)),
            chunks[0],
        );
        if s == Step::Review {
            render_choices(frame, w, chunks[1]);
        } else {
            frame.render_widget(
                Paragraph::new("Enter: finish   Esc: review again")
                    .style(Style::default().fg(Color::Cyan)),
                chunks[1],
            );
        }
    } else if !s.options().is_empty() {
        render_choices(frame, w, parts[1]);
    } else {
        // Keep the editing tail visible; all input is bounded and terminal controls rejected.
        let count = usize::from(parts[1].width.saturating_sub(4));
        let tail: String = w
            .input
            .chars()
            .rev()
            .take(count)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        frame.render_widget(
            Paragraph::new(format!("> {tail}▏"))
                .style(Style::default().fg(Color::Cyan))
                .wrap(Wrap { trim: false }),
            parts[1],
        );
    }
    if let Some(error) = &w.error {
        frame.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Yellow))
                .wrap(Wrap { trim: false }),
            parts[2],
        );
    }
    frame.render_widget(Paragraph::new(if s == Step::Roles { "↑↓ move · Space toggle · Enter next · Esc back\nCtrl+C cancel" } else { "↑↓ choose · Enter next · Esc back · Ctrl+C cancel\nPgUp/PgDn scroll review · Ctrl+U clear text" }).style(Style::default().fg(Color::DarkGray)), regions[2]);
}

fn render_choices(frame: &mut Frame, w: &Wizard, area: Rect) {
    let options: Vec<_> = w
        .step()
        .options()
        .iter()
        .enumerate()
        .map(|(i, text)| {
            let text = if w.step() == Step::Roles {
                format!("[{}] {text}", if w.roles[i] { 'x' } else { ' ' })
            } else {
                (*text).into()
            };
            let mut lines = Vec::new();
            let mut line = String::new();
            for word in text.split_whitespace() {
                let candidate = if line.is_empty() {
                    word.into()
                } else {
                    format!("{line} {word}")
                };
                if Line::from(candidate.clone()).width() > usize::from(area.width.saturating_sub(3))
                    && !line.is_empty()
                {
                    lines.push(Line::from(std::mem::take(&mut line)));
                    line = word.into();
                } else {
                    line = candidate;
                }
            }
            lines.push(Line::from(line));
            ListItem::new(lines)
        })
        .collect();
    frame.render_stateful_widget(
        List::new(options)
            .highlight_symbol("› ")
            .highlight_style(Style::default().fg(Color::Cyan).bold()),
        area,
        &mut ListState::default().with_selected(Some(w.cursor)),
    );
}

enum Action {
    Continue,
    Finish,
    Cancel,
}
fn handle_key(w: &mut Wizard, key: KeyEvent) -> Action {
    if key.kind == KeyEventKind::Release {
        return Action::Continue;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'd'))
    {
        return Action::Cancel;
    }
    let options = w.step().options().len();
    match key.code {
        KeyCode::Esc => w.back(),
        KeyCode::Enter => {
            if w.step() == Step::Review && w.cursor == 2 {
                return Action::Cancel;
            }
            if w.next() {
                return Action::Finish;
            }
        }
        KeyCode::Up if options > 0 => w.cursor = w.cursor.saturating_sub(1),
        KeyCode::Down if options > 0 => w.cursor = (w.cursor + 1).min(options - 1),
        KeyCode::Char(' ') if w.step() == Step::Roles => w.roles[w.cursor] = !w.roles[w.cursor],
        KeyCode::PageDown if matches!(w.step(), Step::Review | Step::Done) => {
            w.scroll = w.scroll.saturating_add(5).min(200)
        }
        KeyCode::PageUp => w.scroll = w.scroll.saturating_sub(5),
        KeyCode::Char('u') if options == 0 && key.modifiers.contains(KeyModifiers::CONTROL) => {
            w.input.clear()
        }
        KeyCode::Backspace if options == 0 => {
            w.input.pop();
        }
        KeyCode::Char(c)
            if options == 0
                && !matches!(w.step(), Step::Done)
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && !c.is_control()
                && w.input.len() + c.len_utf8() <= 512 =>
        {
            w.input.push(c);
            w.error = None;
        }
        _ => {}
    }
    Action::Continue
}

fn paste(w: &mut Wizard, text: &str) {
    if !w.step().options().is_empty() || w.step() == Step::Done {
        return;
    }
    if text.chars().any(char::is_control) || w.input.len().saturating_add(text.len()) > 512 {
        w.error = Some("Paste rejected: use one line, no controls, at most 512 bytes.".into());
    } else {
        w.input.push_str(text);
        w.error = None;
    }
}

fn run() -> Result<bool> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if let Some(command @ ("service-status" | "service-stop" | "service-uninstall")) =
        args.first().map(String::as_str)
    {
        service::manage(command, &args[1..])?;
        return Ok(false);
    }
    if args
        .first()
        .is_some_and(|arg| arg == "install-user-service")
    {
        service::install(&args[1..])?;
        return Ok(false);
    }
    if args == ["--help"] || args == ["-h"] {
        println!(
            "voyage-installer — setup and service installation\n\nWith no arguments: interactive setup preview (all wizard actions mocked).\n\nLinux systemd user service (real installation):\n  voyage-installer install-user-service --bin-dir /absolute/release/bin [--start] [--dry-run]\n\nInstalls private versioned binaries and a Vessel user unit. Existing units are never replaced.\n--start enables and starts the service; otherwise activation is manual.\n--dry-run checks inputs and prints the unit without writing or running systemctl.\nProvider credentials remain in the executing user's existing configuration.\nNo elevated privileges or automatic lingering configuration.\n\nService operations: service-status, service-stop, service-uninstall\nStopping/uninstalling preserves voyage processes and data; drain voyages separately.\n\nOptions: --help, --version"
        );
        return Ok(false);
    }
    if args == ["--version"] {
        println!("voyage-installer {}", env!("CARGO_PKG_VERSION"));
        return Ok(false);
    }
    if !args.is_empty() {
        bail!("Unknown arguments. Run voyage-installer --help.");
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("Setup preview needs an interactive terminal. Use install.sh for piped launches.");
    }
    let cancelled = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, cancelled.clone())?;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        previous(info);
    }));
    let _guard = TerminalGuard;
    terminal::enable_raw_mode()?;
    execute!(
        io::stdout(),
        EnterAlternateScreen,
        event::EnableBracketedPaste
    )?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut w = Wizard::new();
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(true);
        }
        terminal.draw(|frame| draw(frame, &w))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let input = event::read()?;
        let size = terminal.size()?;
        if (size.width < 48 || size.height < 20)
            && !matches!(input, Event::Key(key) if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'd')))
        {
            continue;
        }
        match input {
            Event::Key(key) => match handle_key(&mut w, key) {
                Action::Continue => {}
                Action::Finish => return Ok(false),
                Action::Cancel => return Ok(true),
            },
            Event::Paste(text) => paste(&mut w, &text),
            _ => {}
        }
    }
}

fn main() {
    match run() {
        Ok(cancelled) => {
            // Help/version output needs no completion notice.
            if std::env::args().len() == 1 {
                println!(
                    "{} Nothing was installed or configured.",
                    if cancelled {
                        "Preview cancelled."
                    } else {
                        "Preview finished."
                    }
                );
            }
            if cancelled {
                std::process::exit(130);
            }
        }
        Err(error) => {
            eprintln!("Installer: {error:#}");
            std::process::exit(1);
        }
    }
}
