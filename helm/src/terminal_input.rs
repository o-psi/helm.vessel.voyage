//! Native unbuffered terminal input shared by attended CLI collectors.
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};
static OWNED: AtomicBool = AtomicBool::new(false);

struct Ownership;
impl Drop for Ownership {
    fn drop(&mut self) {
        OWNED.store(false, Ordering::Release);
    }
}
pub(crate) struct Terminal {
    inner: native::Terminal,
    _owner: Ownership,
}
impl Terminal {
    pub(crate) fn enter() -> io::Result<Self> {
        Self::enter_with_preservation(false)
    }
    pub(crate) fn enter_preserving_input() -> io::Result<Self> {
        Self::enter_with_preservation(true)
    }
    fn enter_with_preservation(preserve: bool) -> io::Result<Self> {
        OWNED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| {
                io::Error::new(io::ErrorKind::WouldBlock, "terminal input already owned")
            })?;
        let owner = Ownership;
        Ok(Self {
            inner: native::Terminal::enter(preserve)?,
            _owner: owner,
        })
    }
    // Unix emits UTF-8 bytes; Windows emits UTF-16 code units with key repeat counts.
    pub(crate) fn read(&mut self) -> io::Result<Option<(u16, u16)>> {
        self.inner.read()
    }
    pub(crate) fn discard(&mut self) -> io::Result<()> {
        self.inner.discard()
    }
    pub(crate) fn restore(&mut self) -> io::Result<()> {
        self.inner.restore()
    }
}
#[cfg(unix)]
mod native {
    use std::{
        io,
        time::{Duration, Instant},
    };
    // Only an observed quiet interval confirms that queued input was discarded.
    fn discard_poll_result(ready: i32, events: i16, expired: bool) -> io::Result<bool> {
        if ready < 0 {
            return Err(io::Error::last_os_error());
        }
        if events & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(io::Error::other(
                "terminal input unavailable during discard",
            ));
        }
        if ready == 0 {
            return Ok(true);
        }
        if expired {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "terminal input did not become quiet",
            ));
        }
        Ok(false)
    }

    pub struct Terminal {
        saved: libc::termios,
        active: bool,
        preserve: bool,
    }
    impl Terminal {
        pub fn enter(preserve: bool) -> io::Result<Self> {
            let mut saved = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(0, &mut saved) } != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut terminal = Self {
                saved,
                active: false,
                preserve,
            };
            let mut raw = terminal.saved;
            unsafe { libc::cfmakeraw(&mut raw) };
            if unsafe { libc::tcsetattr(0, libc::TCSANOW, &raw) } != 0 {
                return Err(io::Error::last_os_error());
            }
            terminal.active = true;
            if !preserve && unsafe { libc::tcflush(0, libc::TCIFLUSH) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(terminal)
        }
        pub fn read(&mut self) -> io::Result<Option<(u16, u16)>> {
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
            Ok(Some((u16::from(byte), 1)))
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
                let ready = unsafe { libc::poll(&mut descriptor, 1, 50) };
                if discard_poll_result(ready, descriptor.revents, Instant::now() >= deadline)? {
                    return Ok(());
                }
            }
        }
        pub fn restore(&mut self) -> io::Result<()> {
            if self.active {
                // Do not wait for output to drain: a stopped/full terminal must
                // not prevent cancellation from restoring the saved input mode.
                if !self.preserve {
                    let _ = unsafe { libc::tcflush(0, libc::TCIFLUSH) };
                }
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
        preserve: bool,
        queued: std::collections::VecDeque<u16>,
    }
    impl Terminal {
        fn handle(&self) -> HANDLE {
            self.handle as HANDLE
        }
        pub fn enter(preserve: bool) -> io::Result<Self> {
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
                preserve,
                queued: Default::default(),
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
            if !terminal.preserve {
                terminal.flush()?;
            }
            Ok(terminal)
        }
        fn flush(&self) -> io::Result<()> {
            if unsafe { FlushConsoleInputBuffer(self.handle()) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
        pub fn read(&mut self) -> io::Result<Option<(u16, u16)>> {
            if let Some(value) = self.queued.pop_front() {
                return Ok(Some((value, 1)));
            }
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
            if self.preserve {
                // Encode native navigation keys for the selected PTY, without
                // using a second buffered console reader.
                let sequence = match key.wVirtualKeyCode {
                    33 => Some("\x1b[5~"),
                    34 => Some("\x1b[6~"),
                    35 => Some("\x1b[F"),
                    36 => Some("\x1b[H"),
                    37 => Some("\x1b[D"),
                    38 => Some("\x1b[A"),
                    39 => Some("\x1b[C"),
                    40 => Some("\x1b[B"),
                    45 => Some("\x1b[2~"),
                    46 => Some("\x1b[3~"),
                    112 => Some("\x1bOP"),
                    113 => Some("\x1bOQ"),
                    114 => Some("\x1bOR"),
                    115 => Some("\x1bOS"),
                    116 => Some("\x1b[15~"),
                    117 => Some("\x1b[17~"),
                    118 => Some("\x1b[18~"),
                    119 => Some("\x1b[19~"),
                    120 => Some("\x1b[20~"),
                    121 => Some("\x1b[21~"),
                    122 => Some("\x1b[23~"),
                    123 => Some("\x1b[24~"),
                    _ => None,
                };
                if let Some(sequence) = sequence {
                    for _ in 0..key.wRepeatCount.clamp(1, 64) {
                        self.queued.extend(sequence.encode_utf16());
                    }
                    return Ok(self.queued.pop_front().map(|value| (value, 1)));
                }
            }
            if value == 0 {
                return Ok(None);
            }
            Ok(Some((value, key.wRepeatCount.max(1))))
        }
        pub fn discard(&mut self) -> io::Result<()> {
            let deadline = Instant::now() + Duration::from_millis(250);
            loop {
                self.flush()?;
                match unsafe { WaitForSingleObject(self.handle(), 50) } {
                    WAIT_TIMEOUT => return Ok(()),
                    WAIT_OBJECT_0 if Instant::now() >= deadline => {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "terminal input did not become quiet",
                        ));
                    }
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
        pub fn enter(_preserve: bool) -> io::Result<Self> {
            Err(io::ErrorKind::Unsupported.into())
        }
        pub fn read(&mut self) -> io::Result<Option<(u16, u16)>> {
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
