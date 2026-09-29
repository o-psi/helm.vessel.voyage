//! Cleanup authority comes from kernel child ownership, never a remembered PID.
use anyhow::{Result, ensure};
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    time::{Duration, Instant},
};

/// The calling process must be an independent subreaper. ECHILD establishes only
/// local descendant cleanup. Timeouts retain an unresolved cleanup obligation.
pub fn drain(timeout: Duration) -> Result<()> {
    let mut subreaper: libc::c_int = 0;
    ensure!(
        unsafe { libc::prctl(libc::PR_GET_CHILD_SUBREAPER, &mut subreaper, 0, 0, 0) } == 0
            && subreaper == 1,
        "descendant cleanup requires an independent subreaper"
    );
    let deadline = Instant::now() + timeout;
    loop {
        if reap(deadline)? {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "descendant cleanup remains unconfirmed"
        );
        stop_children(deadline)?;
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn reap(deadline: Instant) -> Result<bool> {
    loop {
        ensure!(
            Instant::now() < deadline,
            "descendant cleanup remains unconfirmed"
        );
        let result = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if result > 0 {
            continue;
        }
        if result == 0 {
            return Ok(false);
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        if error.raw_os_error() == Some(libc::ECHILD) {
            return Ok(true);
        }
        return Err(error.into());
    }
}
fn parent(pid: u32) -> io::Result<u32> {
    use io::Read;
    let mut text = String::new();
    std::fs::File::open(format!("/proc/{pid}/stat"))?
        .take(4097)
        .read_to_string(&mut text)?;
    if text.len() > 4096 {
        return Err(io::ErrorKind::InvalidData.into());
    }
    text.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().nth(1))
        .and_then(|ppid| ppid.parse().ok())
        .ok_or_else(|| io::ErrorKind::InvalidData.into())
}
fn stop_children(deadline: Instant) -> Result<()> {
    use io::Read;
    let mut children = String::new();
    let path = format!("/proc/self/task/{}/children", std::process::id());
    std::fs::File::open(path)?
        .take(1024 * 1024 + 1)
        .read_to_string(&mut children)?;
    ensure!(
        children.len() <= 1024 * 1024,
        "child observation limit exceeded"
    );
    for (count, pid) in children.split_whitespace().enumerate() {
        ensure!(
            count < 65536 && Instant::now() < deadline,
            "child observation limit exceeded"
        );
        let pid: u32 = pid.parse()?;
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0_u32) };
        if raw < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                continue;
            }
            return Err(error.into());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(raw as i32) };
        // Own unreaped children cannot be recycled; still check after pinning.
        ensure!(
            parent(pid)? == std::process::id(),
            "child ownership changed"
        );
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
            return Err(io::Error::last_os_error().into());
        }
    }
    Ok(())
}
