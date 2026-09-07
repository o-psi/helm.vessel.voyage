//! The terminal owns only selection/review; installation runs after its guard drops.
use crate::cli::{Action, Options};
use anyhow::{Result, ensure};
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    io::{self, IsTerminal},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct Screen;
impl Drop for Screen {
    fn drop(&mut self) {
        restore();
    }
}
fn restore() {
    let _ = terminal::disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}
fn safe(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_control() && c != '\n' {
                '�'
            } else {
                c
            }
        })
        .collect()
}

pub fn review(mut options: Options) -> Result<Option<(Options, Vec<String>)>> {
    ensure!(
        io::stdin().is_terminal() && io::stdout().is_terminal(),
        "noninteractive installation requires an explicit install, upgrade or rollback action; use --help"
    );
    let cancelled = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, cancelled.clone())?;
    }
    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        old_hook(info);
    }));
    let _screen = Screen;
    terminal::enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut selected = 0usize;
    let mut review: Option<Vec<String>> = None;
    let mut planning: Option<std::sync::mpsc::Receiver<Result<Vec<String>, String>>> = None;
    let mut error: Option<String> = None;
    let mut scroll = 0u16;
    loop {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if let Some(pending) = &planning {
            match pending.try_recv() {
                Ok(result) => {
                    planning = None;
                    match result {
                        Ok(lines) => {
                            review = Some(lines);
                            error = None;
                            scroll = 0;
                        }
                        Err(failure) => error = Some(failure),
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    planning = None;
                    error = Some("Release verification stopped unexpectedly.".into());
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        terminal.draw(|frame| {
            let area = frame.area();
            if area.width < 48 || area.height < 16 {
                frame.render_widget(Paragraph::new("Resize to at least 48 × 16. Ctrl+C cancels."), area);
                return;
            }
            let regions = Layout::vertical([Constraint::Length(3), Constraint::Min(1), Constraint::Length(4)]).margin(1).split(area);
            frame.render_widget(Paragraph::new("VOYAGE / INSTALLATION\nHelm interface · Vessel supervisor · Voyage runtime").style(Style::default().fg(Color::Cyan)), regions[0]);
            let content = if planning.is_some() { "Verifying release and service state…\n\nCtrl+C or Esc cancels. No installation changes are being applied.".into() } else if let Some(lines) = &review { lines.join("\n\n") } else {
                let actions = ["Install", "Upgrade", "Rollback to previous release"];
                format!("{}\n\nSource: {}\n\n[s] Service start: {}\n[r] Replace unmanaged binaries with retained backups: {}\n\nProvider credentials and configuration stay on this machine.", actions.iter().enumerate().map(|(i,a)| format!("{} {a}", if i == selected { "›" } else { " " })).collect::<Vec<_>>().join("\n"), options.bin_dir.display(), options.start, options.replace_existing)
            };
            frame.render_widget(Paragraph::new(safe(&content)).block(Block::default().borders(Borders::TOP).title(if review.is_some() { " Review actual plan " } else { " Choose action " })).wrap(Wrap { trim: false }).scroll((scroll,0)), regions[1]);
            let hint = if review.is_some() { "Enter: apply reviewed plan · Esc: change · PgUp/PgDn: scroll · Ctrl+C: cancel" } else { "↑↓: choose · Enter: review · s/r: toggle · Esc/Ctrl+C: cancel" };
            frame.render_widget(Paragraph::new(safe(&format!("{}\n{}", error.as_deref().unwrap_or(""), hint))).wrap(Wrap { trim: false }), regions[2]);
        })?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c' | 'd'))
        {
            return Ok(None);
        }
        if planning.is_some() {
            if key.code == KeyCode::Esc {
                return Ok(None);
            }
            continue;
        }
        let size = terminal.size()?;
        if size.width < 48 || size.height < 16 {
            continue;
        }
        match key.code {
            KeyCode::Esc if review.is_some() => {
                review = None;
                error = None;
                scroll = 0;
            }
            KeyCode::Esc => return Ok(None),
            KeyCode::Enter if review.is_some() => {
                return Ok(Some((options, review.take().expect("review present"))));
            }
            KeyCode::Enter => {
                options.action =
                    Some([Action::Install, Action::Upgrade, Action::Rollback][selected]);
                planning = Some(crate::planning::start(options.clone()));
                error = None;
            }
            KeyCode::Up if review.is_none() => selected = selected.saturating_sub(1),
            KeyCode::Down if review.is_none() => selected = (selected + 1).min(2),
            KeyCode::Char('s') if review.is_none() => options.start = !options.start,
            KeyCode::Char('r') if review.is_none() => {
                options.replace_existing = !options.replace_existing
            }
            KeyCode::PageUp => scroll = scroll.saturating_sub(8),
            KeyCode::PageDown => scroll = scroll.saturating_add(8),
            _ => {}
        }
    }
}
