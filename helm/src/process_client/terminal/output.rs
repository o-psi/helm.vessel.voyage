//! Bound Unix console output even when the outer PTY stops reading.
use std::io::{self, Write};
#[cfg(unix)]
use std::time::{Duration, Instant};

pub(super) struct Output {
    #[cfg(unix)]
    saved: i32,
    #[cfg(unix)]
    deadline: Instant,
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
                deadline: Instant::now() + Duration::from_millis(250),
            })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }
}
impl Output {
    pub fn begin_frame(&mut self) {
        #[cfg(unix)]
        {
            self.deadline = Instant::now() + Duration::from_millis(250);
        }
    }
}
impl Write for Output {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        #[cfg(unix)]
        {
            let end = self.deadline;
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
        Ok(())
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
