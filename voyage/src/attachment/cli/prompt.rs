//! Native secret input with exact saved-mode restoration and no parser backlog.
use super::{CliError, Secret, parse_secret};
use crate::terminal_input as native;
use std::{
    io::{self, IsTerminal, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[derive(Default)]
struct State {
    terminal: Option<native::Terminal>,
    cancelled: bool,
}
#[derive(Clone, Default)]
pub struct PromptControl(Arc<Mutex<State>>);
impl PromptControl {
    /// Serialized with mode entry and reads: cancellation cannot race a late
    /// mode enable, and queued secret input is discarded before restoring echo.
    pub fn cancel_and_restore(&self) -> Result<(), CliError> {
        self.finish(true)
    }
    fn finish(&self, cancel: bool) -> Result<(), CliError> {
        let mut state = self.0.lock().map_err(|_| CliError::Input)?;
        state.cancelled |= cancel;
        if let Some(terminal) = state.terminal.as_mut() {
            let discarded = terminal.discard();
            // Even a failed drain must not bypass restoration (including the
            // cancellation path, which exits without running destructors).
            let restored = terminal.restore();
            if restored.is_ok() {
                state.terminal = None;
            }
            discarded.and(restored).map_err(|_| CliError::Input)?;
        }
        Ok(())
    }
    fn start(&self) -> Result<(), CliError> {
        if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
            return Err(CliError::Input);
        }
        let mut state = self.0.lock().map_err(|_| CliError::Input)?;
        if state.cancelled {
            return Err(CliError::Cancelled);
        }
        state.terminal = Some(native::Terminal::enter().map_err(|_| CliError::Input)?);
        // Terminal output can stall. Never hold the restoration lock while
        // writing the fixed marker: signal cancellation must still complete.
        drop(state);
        io::stderr()
            .write_all(b"Invitation key (hidden): ")
            .and_then(|_| io::stderr().flush())
            .map_err(|_| CliError::Input)
    }
}
struct Restore(PromptControl);
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = self.0.finish(false);
    }
}
pub(super) async fn read(control: PromptControl) -> Result<Secret, CliError> {
    tokio::task::spawn_blocking(move || {
        let _restore = Restore(control.clone());
        control.start()?;
        let result = (|| {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut secret = Secret(Vec::with_capacity(43));
            loop {
                if Instant::now() >= deadline {
                    return Err(CliError::Cancelled);
                }
                let key = {
                    let mut state = control.0.lock().map_err(|_| CliError::Input)?;
                    if state.cancelled {
                        return Err(CliError::Cancelled);
                    }
                    state
                        .terminal
                        .as_mut()
                        .ok_or(CliError::Cancelled)?
                        .read()
                        .map_err(|_| CliError::Input)?
                };
                let Some((byte, repeats)) = key else {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let byte = u8::try_from(byte).map_err(|_| CliError::Input)?;
                for _ in 0..repeats {
                    match byte {
                        b'\r' | b'\n' => return parse_secret(std::mem::take(&mut secret.0)),
                        3 | 4 | 27 => return Err(CliError::Cancelled),
                        8 | 127 => {
                            secret.0.pop();
                        }
                        b if b.is_ascii_alphanumeric() || b == b'_' || b == b'-' => {
                            if secret.0.len() == 43 {
                                return Err(CliError::Input);
                            }
                            secret.0.push(b);
                        }
                        _ => return Err(CliError::Input),
                    }
                }
            }
        })();
        control.finish(false)?;
        if result.is_ok() {
            let _ = io::stderr().write_all(b"\n");
        }
        result
    })
    .await
    .map_err(|_| CliError::Input)?
}
