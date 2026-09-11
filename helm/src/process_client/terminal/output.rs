//! Bound Unix console output even when the outer PTY stops reading.
use std::io::{self, Write};
#[cfg(unix)]
use std::time::{Duration, Instant};

pub(super) struct Output {
    #[cfg(unix)]
    saved: i32,
    #[cfg(unix)]
    budget: Budget,
}
impl Output {
    pub fn new() -> io::Result<Self> {
        #[cfg(unix)]
        {
            let saved = unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_GETFL) };
            if saved < 0
                || unsafe {
                    libc::fcntl(libc::STDOUT_FILENO, libc::F_SETFL, saved | libc::O_NONBLOCK)
                } < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                saved,
                budget: Budget::new(),
            })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }
}
#[derive(Clone)]
pub(super) struct Budget {
    #[cfg(unix)]
    deadline: std::sync::Arc<std::sync::Mutex<Instant>>,
}
impl Budget {
    fn new() -> Self {
        Self {
            #[cfg(unix)]
            deadline: std::sync::Arc::new(std::sync::Mutex::new(
                Instant::now() + Duration::from_millis(250),
            )),
        }
    }
    pub fn begin_frame(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            *self
                .deadline
                .lock()
                .map_err(|_| io::Error::other("terminal output budget unavailable"))? =
                Instant::now() + Duration::from_millis(250);
        }
        Ok(())
    }
}
impl Output {
    pub fn budget(&self) -> Budget {
        #[cfg(unix)]
        {
            self.budget.clone()
        }
        #[cfg(not(unix))]
        {
            Budget::new()
        }
    }
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            let end = *self
                .budget
                .deadline
                .lock()
                .map_err(|_| io::Error::other("terminal output budget unavailable"))?;
            loop {
                if Instant::now() >= end {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "outer terminal frame deadline",
                    ));
                }
                let count =
                    unsafe { libc::write(libc::STDOUT_FILENO, bytes.as_ptr().cast(), bytes.len()) };
                if count >= 0 {
                    return Ok(count as usize);
                }
                let error = io::Error::last_os_error();
                if !matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    return Err(error);
                }
                if Instant::now() >= end {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "outer terminal output stalled",
                    ));
                }
                let mut descriptor = libc::pollfd {
                    fd: libc::STDOUT_FILENO,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                unsafe {
                    libc::poll(&mut descriptor, 1, 5);
                }
            }
        }
        #[cfg(not(unix))]
        io::stdout().write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        #[cfg(unix)]
        {
            Ok(())
        }
        #[cfg(not(unix))]
        {
            io::stdout().flush()
        }
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        #[cfg(unix)]
        unsafe {
            libc::fcntl(libc::STDOUT_FILENO, libc::F_SETFL, self.saved);
        }
    }
}
