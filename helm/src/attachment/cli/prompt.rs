//! Native secret input with exact saved-mode restoration and no parser backlog.
use super::{CliError, Secret, parse_secret};
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
#[cfg(unix)]
mod native {
    use std::{
        io,
        time::{Duration, Instant},
    };
    pub struct Terminal {
        saved: libc::termios,
        active: bool,
    }
    impl Terminal {
        pub fn enter() -> io::Result<Self> {
            let mut saved = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut terminal = Self {
                saved,
                active: false,
            };
            let mut raw = terminal.saved;
            unsafe { libc::cfmakeraw(&mut raw) };
            if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
                return Err(io::Error::last_os_error());
            }
            terminal.active = true;
            if unsafe { libc::tcflush(0, libc::TCIFLUSH) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(terminal)
        }
        pub fn read(&mut self) -> io::Result<Option<(u8, u16)>> {
            let mut descriptor = libc::pollfd {
                fd: 0,
                events: libc::POLLIN,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
            if result < 0 {
                return if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                    Ok(None)
                } else {
                    Err(io::Error::last_os_error())
                };
            }
            if result == 0 {
                return Ok(None);
            }
            let mut byte = 0u8;
            let count = unsafe { libc::read(0, (&mut byte as *mut u8).cast(), 1) };
            if count != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "terminal unavailable",
                ));
            }
            Ok(Some((byte, 1)))
        }
        pub fn discard(&mut self) -> io::Result<()> {
            // Drain already queued paste tails while echo remains disabled. A
            // short quiet interval also catches a paste split across OS reads.
            let deadline = Instant::now() + Duration::from_millis(250);
            loop {
                if unsafe { libc::tcflush(0, libc::TCIFLUSH) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut descriptor = libc::pollfd {
                    fd: 0,
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut descriptor, 1, 50) } <= 0 || Instant::now() >= deadline
                {
                    return Ok(());
                }
            }
        }
        pub fn restore(&mut self) -> io::Result<()> {
            if self.active {
                // Do not wait for output to drain: a stopped/full terminal must
                // not prevent cancellation from restoring the saved input mode.
                let _ = unsafe { libc::tcflush(0, libc::TCIFLUSH) };
                if unsafe { libc::tcsetattr(0, libc::TCSANOW, &self.saved) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                self.active = false;
            }
            Ok(())
        }
    }
    impl Drop for Terminal {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }
}
#[cfg(windows)]
mod native {
    use std::{
        io,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{Console::*, Threading::WaitForSingleObject},
    };
    pub struct Terminal {
        handle: isize,
        saved: u32,
        active: bool,
    }
    impl Terminal {
        fn handle(&self) -> HANDLE {
            self.handle as HANDLE
        }
        pub fn enter() -> io::Result<Self> {
            let handle = unsafe { GetStdHandle(STD_INPUT_HANDLE) };
            let mut saved = 0;
            if handle.is_null()
                || handle == INVALID_HANDLE_VALUE
                || unsafe { GetConsoleMode(handle, &mut saved) } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let mut terminal = Self {
                handle: handle as isize,
                saved,
                active: false,
            };
            if unsafe {
                SetConsoleMode(
                    handle,
                    saved
                        & !(ENABLE_ECHO_INPUT
                            | ENABLE_LINE_INPUT
                            | ENABLE_PROCESSED_INPUT
                            | ENABLE_VIRTUAL_TERMINAL_INPUT),
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            terminal.active = true;
            terminal.flush()?;
            Ok(terminal)
        }
        fn flush(&self) -> io::Result<()> {
            if unsafe { FlushConsoleInputBuffer(self.handle()) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        pub fn read(&mut self) -> io::Result<Option<(u8, u16)>> {
            match unsafe { WaitForSingleObject(self.handle(), 0) } {
                WAIT_TIMEOUT => return Ok(None),
                WAIT_OBJECT_0 => (),
                _ => return Err(io::Error::last_os_error()),
            }
            let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
            let mut read = 0;
            if unsafe { ReadConsoleInputW(self.handle(), &mut record, 1, &mut read) } == 0 {
                return Err(io::Error::last_os_error());
            }
            if read == 0 || u32::from(record.EventType) != KEY_EVENT {
                return Ok(None);
            }
            let key = unsafe { record.Event.KeyEvent };
            if key.bKeyDown == 0 {
                return Ok(None);
            }
            let value = unsafe { key.uChar.UnicodeChar };
            if value == 0 {
                return Ok(None);
            }
            if value > 127 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid invitation character",
                ));
            }
            Ok(Some((value as u8, key.wRepeatCount.max(1))))
        }
        pub fn discard(&mut self) -> io::Result<()> {
            let deadline = Instant::now() + Duration::from_millis(250);
            loop {
                self.flush()?;
                match unsafe { WaitForSingleObject(self.handle(), 50) } {
                    WAIT_TIMEOUT => return Ok(()),
                    WAIT_OBJECT_0 if Instant::now() >= deadline => return self.flush(),
                    WAIT_OBJECT_0 => (),
                    _ => return Err(io::Error::last_os_error()),
                }
            }
        }
        pub fn restore(&mut self) -> io::Result<()> {
            if self.active {
                if unsafe { SetConsoleMode(self.handle(), self.saved) } == 0 {
                    return Err(io::Error::last_os_error());
                }
                self.active = false;
            }
            Ok(())
        }
    }
    impl Drop for Terminal {
        fn drop(&mut self) {
            let _ = self.restore();
        }
    }
}
#[cfg(not(any(unix, windows)))]
mod native {
    use std::io;
    pub struct Terminal;
    impl Terminal {
        pub fn enter() -> io::Result<Self> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub fn read(&mut self) -> io::Result<Option<(u8, u16)>> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub fn discard(&mut self) -> io::Result<()> {
            Ok(())
        }
        pub fn restore(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
}
