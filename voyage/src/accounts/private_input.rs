//! Execution-host terminal only; never use this reader through a model-visible PTY.
use anyhow::{Result, ensure};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    terminal,
};
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};
/// Reads a bounded key without echo; Escape/Ctrl-C cancels. Caller must ensure private human attachment.
pub fn api_key() -> Result<Option<String>> {
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "API entry requires a private execution-host terminal"
    );
    eprint!("API key (hidden; Escape cancels): ");
    std::io::stderr().flush()?;
    terminal::enable_raw_mode()?;
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = terminal::disable_raw_mode();
            eprintln!();
        }
    }
    let _restore = Restore;
    let deadline = Instant::now() + Duration::from_secs(300);
    let mut key = zeroize::Zeroizing::new(String::new());
    while Instant::now() < deadline {
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        if let Event::Key(event) = event::read()? {
            if event.kind == event::KeyEventKind::Release {
                continue;
            }
            match event.code {
                KeyCode::Esc => return Ok(None),
                KeyCode::Char('c') if event.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(None);
                }
                KeyCode::Enter => {
                    ensure!(!key.is_empty(), "API key was empty");
                    return Ok(Some(key.to_string()));
                }
                KeyCode::Backspace => {
                    key.pop();
                }
                KeyCode::Char(c)
                    if !c.is_control() && !event.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    ensure!(key.len() + c.len_utf8() <= 8192, "API key exceeds bound");
                    key.push(c);
                }
                _ => {}
            }
        }
    }
    anyhow::bail!("private input timed out")
}
