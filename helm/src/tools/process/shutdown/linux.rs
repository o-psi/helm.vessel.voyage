use super::TerminalShutdownFailure as Failure;
use std::{
    fs,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::Instant,
};

pub(super) struct SessionIdentity {
    pid: u32,
    start: u64,
}
struct Stat {
    pid: u32,
    session: u32,
    start: u64,
    state: char,
}
fn read_stat(pid: u32) -> io::Result<Stat> {
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{pid}/stat"))?
        .take(4097)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| io::ErrorKind::InvalidData)?;
    parse_stat(text).ok_or_else(|| io::ErrorKind::InvalidData.into())
}
fn parse_stat(text: &str) -> Option<Stat> {
    let (prefix, fields) = text.rsplit_once(") ")?;
    let pid = prefix.split_once(" (")?.0.parse().ok()?;
    let fields: Vec<_> = fields.split_whitespace().collect();
    Some(Stat {
        pid,
        state: fields.first()?.chars().next()?,
        session: fields.get(3)?.parse().ok()?,
        start: fields.get(19)?.parse().ok()?,
    })
}
impl SessionIdentity {
    pub(super) fn capture(pid: u32) -> io::Result<Self> {
        let stat = read_stat(pid)?;
        if stat.pid != pid || stat.session != pid || pid == 0 || pid > i32::MAX as u32 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        Ok(Self {
            pid,
            start: stat.start,
        })
    }
    pub(super) fn matches_leader(&self) -> bool {
        read_stat(self.pid).is_ok_and(|stat| {
            stat.pid == self.pid && stat.session == self.pid && stat.start == self.start
        })
    }
    pub(super) fn exit_status(&self) -> io::Result<Option<portable_pty::ExitStatus>> {
        use std::os::unix::process::ExitStatusExt;
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                self.pid,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        let info = unsafe { info.assume_init() };
        if unsafe { info.si_pid() } == 0 {
            return Ok(None);
        }
        let status = unsafe { info.si_status() };
        let raw = if info.si_code == libc::CLD_EXITED {
            status << 8
        } else {
            status & 0x7f
        };
        Ok(Some(std::process::ExitStatus::from_raw(raw).into()))
    }
    pub(super) fn kill_and_observe(&self, deadline: Instant) -> Result<bool, Failure> {
        if !self.matches_leader() {
            return Err(Failure::IdentityUnavailable);
        }
        let mut live = false;
        for (count, entry) in fs::read_dir("/proc")
            .map_err(|_| Failure::ObservationUnavailable)?
            .enumerate()
        {
            if count >= 65536 || Instant::now() >= deadline {
                return Err(Failure::TimedOut);
            }
            let entry = entry.map_err(|_| Failure::ObservationUnavailable)?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let stat = match read_stat(pid) {
                Ok(stat) => stat,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err(Failure::ObservationUnavailable),
            };
            if stat.session != self.pid || matches!(stat.state, 'Z' | 'X') {
                continue;
            }
            live = true;
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0_u32) };
            if fd < 0 {
                if io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                    continue;
                }
                return Err(Failure::ObservationUnavailable);
            }
            let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };
            // Recheck after opening a stable handle: a recycled number must never
            // turn a session membership scan into authority over another process.
            let current = match read_stat(pid) {
                Ok(current) => current,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(_) => return Err(Failure::ObservationUnavailable),
            };
            if current.start != stat.start || current.session != self.pid || current.pid != pid {
                continue;
            }
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0_u32,
                )
            };
            if result < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(Failure::KillFailed);
            }
        }
        Ok(!live)
    }
}
